//! `GET /reports/guardrails/summary`: event counts over the events list's selection, per group of
//! up to two dimensions and per time bucket, with the matches (entity counts) of each group and how
//! many of its events share their fingerprint with another event of the selection.

use std::collections::{BTreeMap, HashMap};

use antirion_store::{SafetyEventRow, SafetyFilter};
use axum::response::Response;
use http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::call::Call;
use crate::error::AdminError;
use crate::safety::{SELECTION, selection};
use crate::{analytics, logquery, pagination};

const SCOPE: &str = "reports/guardrails/summary";
const DIMENSIONS: [&str; 6] = [
    "category",
    "severity",
    "action",
    "guardrail",
    "entity",
    "review_state",
];
/// The most events one summary reads.
const MAX_EVENTS: usize = 200_000;
const BATCH: usize = 5_000;

/// The value of a dimension other than `entity`.
fn value_of(e: &SafetyEventRow, dim: &str) -> Option<String> {
    Some(match dim {
        "category" => e.category.clone(),
        "severity" => e.severity.clone(),
        "action" => e.action.clone(),
        "guardrail" => e.guardrail.clone(),
        "review_state" => e.review_state.clone(),
        _ => return None,
    })
}

/// The entity types of an event with their match counts.
fn entities_of(e: &SafetyEventRow) -> Vec<(String, u64)> {
    serde_json::from_str::<Value>(&e.entities)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|x| {
            Some((
                x.get("type")?.as_str()?.to_owned(),
                x.get("count").and_then(Value::as_u64).unwrap_or(0),
            ))
        })
        .collect()
}

fn group_by(call: &Call) -> Result<Vec<&'static str>, AdminError> {
    let mut out: Vec<&'static str> = Vec::new();
    for (_, v) in call.query.iter().filter(|(k, _)| k == "group_by") {
        for name in v.split(',').filter(|s| !s.is_empty()) {
            let d = DIMENSIONS
                .iter()
                .find(|d| **d == name)
                .ok_or_else(|| AdminError::invalid_parameter(format!("group_by does not accept {name:?}")))?;
            if !out.contains(d) {
                out.push(d);
            }
        }
    }
    if out.len() > 2 {
        return Err(AdminError::invalid_parameter(
            "group_by accepts at most 2 dimensions",
        ));
    }
    Ok(out)
}

/// Calls `visit` with each event of the selection the caller may read, a page at a time; refuses
/// a selection of more than `MAX_EVENTS` events.
async fn each_event(
    call: &Call,
    f: &SafetyFilter,
    mut visit: impl FnMut(&SafetyEventRow),
) -> Result<(), AdminError> {
    call.rt.read_safety().await;
    let mut read = 0usize;
    let mut after = None;
    loop {
        let page = call
            .rt
            .store
            .list_safety_events(f, after, BATCH)
            .await
            .map_err(|_| AdminError::store_unavailable())?;
        let full = page.len() == BATCH;
        after = page.last().map(|e| (e.time_us, e.id.clone()));
        for e in &page {
            let c = crate::safety::chain(e);
            if call.principal.sees(&c) && call.require("logs:read", &c).is_ok() {
                visit(e);
                read += 1;
            }
        }
        if read > MAX_EVENTS {
            return Err(AdminError::invalid_parameter(
                "the selection holds more than 200000 events; narrow it",
            ));
        }
        if !full {
            return Ok(());
        }
    }
}

#[derive(Default)]
struct Cell {
    events: u64,
    matches: u64,
    /// The fingerprint of each event counted, by its number in [`Fingerprints`]; whether an
    /// event is related is known only once every event has been read.
    fingerprints: Vec<u32>,
}

/// The selection's fingerprints, numbered, with how many events have each.
#[derive(Default)]
struct Fingerprints {
    ids: HashMap<String, u32>,
    counts: Vec<u64>,
}

impl Fingerprints {
    fn add(&mut self, fingerprint: &str) -> u32 {
        let id = match self.ids.get(fingerprint) {
            Some(id) => *id,
            None => {
                let id = u32::try_from(self.counts.len()).unwrap_or(u32::MAX);
                self.ids.insert(fingerprint.to_owned(), id);
                self.counts.push(0);
                id
            }
        };
        if let Some(n) = self.counts.get_mut(id as usize) {
            *n += 1;
        }
        id
    }

    /// Whether another event of the selection shares the fingerprint.
    fn shared(&self, id: u32) -> bool {
        self.counts.get(id as usize).is_some_and(|n| *n > 1)
    }
}

pub(crate) async fn summary(call: &Call) -> Result<Response, AdminError> {
    if call.method != Method::GET {
        return Err(AdminError::method_not_allowed("GET"));
    }
    logquery::allow(call, &SELECTION, &["group_by", "interval", "limit", "cursor"])?;
    crate::logs::require_some(call, "logs:read")?;
    let (limit, after) = pagination::page_params(call, SCOPE)?;
    let offset = after
        .map(|a| {
            a.parse::<usize>()
                .map_err(|_| AdminError::invalid_parameter("the cursor is not valid"))
        })
        .transpose()?
        .unwrap_or(0);
    let dims = group_by(call)?;
    let interval = call
        .param("interval")
        .map(|v| {
            analytics::interval_us(v).ok_or_else(|| {
                AdminError::invalid_parameter("interval must be one of 1m, 5m, 15m, 1h, 6h, 1d, hour, day")
            })
        })
        .transpose()?;
    let mut f = selection(call)?;
    let starts = match interval {
        Some(step) => {
            let (from, to) = analytics::range(call)?;
            f.from_us = Some(from);
            f.to_us = Some(to);
            Some((analytics::starts(from, to, Some(step), None)?, step))
        }
        None => None,
    };
    let mut fingerprints = Fingerprints::default();
    let mut cells: BTreeMap<(Option<i64>, Vec<Option<String>>), Cell> = BTreeMap::new();
    each_event(call, &f, |e| {
        let fingerprint = fingerprints.add(&e.fingerprint);
        let start = match &starts {
            Some((list, step)) => {
                let Some(first) = list.first() else { return };
                let i = (e.time_us - first).div_euclid(*step);
                let Some(s) = usize::try_from(i).ok().and_then(|i| list.get(i)) else {
                    return;
                };
                Some(*s)
            }
            None => None,
        };
        let ents = entities_of(e);
        let mut keys: Vec<(Vec<Option<String>>, u64)> = vec![(Vec::new(), ents.iter().map(|(_, c)| c).sum())];
        for d in &dims {
            let mut next = Vec::new();
            for (k, m) in keys {
                if *d == "entity" {
                    if ents.is_empty() {
                        let mut k2 = k.clone();
                        k2.push(None);
                        next.push((k2, 0));
                    }
                    for (t, c) in &ents {
                        let mut k2 = k.clone();
                        k2.push(Some(t.clone()));
                        next.push((k2, *c));
                    }
                } else {
                    let mut k2 = k;
                    k2.push(value_of(e, d));
                    next.push((k2, m));
                }
            }
            keys = next;
        }
        for (k, m) in keys {
            let c = cells.entry((start, k)).or_default();
            c.events += 1;
            c.matches += m;
            c.fingerprints.push(fingerprint);
        }
    })
    .await?;
    let rows: Vec<Value> = cells
        .into_iter()
        .skip(offset)
        .take(limit + 1)
        .map(|((start, values), c)| {
            let group: serde_json::Map<String, Value> = dims
                .iter()
                .zip(values)
                .map(|(d, v)| ((*d).to_owned(), json!(v)))
                .collect();
            json!({
                "group": group,
                "start": start.map(|t| antirion_core::UtcMicros(t).to_rfc3339()),
                "events": c.events,
                "related": c.fingerprints.iter().filter(|id| fingerprints.shared(**id)).count(),
                "matches": c.matches,
            })
        })
        .collect();
    let more = rows.len() > limit;
    let items: Vec<Value> = rows.into_iter().take(limit).collect();
    let next_cursor =
        more.then(|| pagination::encode(&call.rt.cursor_key, SCOPE, &(offset + limit).to_string()));
    Ok(call.json(
        StatusCode::OK,
        &json!({"items": items, "next_cursor": next_cursor}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_are_read_with_their_counts() {
        let e = SafetyEventRow {
            entities: r#"[{"type":"email","count":2},{"type":"phone","count":1}]"#.into(),
            ..SafetyEventRow::default()
        };
        assert_eq!(
            entities_of(&e),
            [("email".to_owned(), 2), ("phone".to_owned(), 1)]
        );
        assert_eq!(value_of(&e, "entity"), None);
    }

    #[test]
    fn fingerprints_are_shared_once_two_events_have_them() {
        let mut f = Fingerprints::default();
        let a = f.add("a");
        let b = f.add("b");
        assert!(!f.shared(a));
        assert_eq!(f.add("a"), a);
        assert!(f.shared(a) && !f.shared(b));
    }
}
