//! The spend ledger and its reports: `GET /reports/spend/requests/{request_id}`,
//! `GET /reports/spend/ledger` and `GET /reports/spend/summary`, and the effective price of a
//! deployment, `GET /pricing/deployments/{name}`. Lines are visible to principals with
//! `analytics:read` in their tenancy scope.

use std::sync::Arc;

use antirion_core::UtcMicros;
use antirion_identity::rbac::Scope;
use antirion_limits::book::{GROUP_ATTRS, LedgerFilter, Summary};
use antirion_limits::ledger::LedgerLine;
use axum::response::Response;
use http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::call::Call;
use crate::error::AdminError;
use crate::pagination;

const FILTERS: &[&str] = &[
    "organization",
    "team",
    "project",
    "key",
    "user",
    "end_user",
    "model",
    "deployment",
    "tag",
    "status",
];

fn chain(l: &LedgerLine) -> Vec<Scope> {
    let mut c = vec![Scope::Gateway];
    for (s, v) in [
        (Scope::Organization as fn(String) -> Scope, &l.organization),
        (Scope::Team, &l.team),
        (Scope::Project, &l.project),
    ] {
        if !v.is_empty() {
            c.push(s(v.clone()));
        }
    }
    c
}

fn readable(call: &Call, l: &LedgerLine) -> bool {
    let c = chain(l);
    call.principal.sees(&c) && call.require("analytics:read", &c).is_ok()
}

fn require_any(call: &Call) -> Result<(), AdminError> {
    if call
        .principal
        .bindings
        .iter()
        .any(|b| b.role.allows("analytics:read"))
    {
        return Ok(());
    }
    call.require("analytics:read", &[Scope::Gateway])
}

fn time_param(call: &Call, name: &str) -> Result<Option<UtcMicros>, AdminError> {
    call.param(name)
        .map(|v| {
            UtcMicros::parse_rfc3339(v)
                .map_err(|_| AdminError::invalid_parameter(format!("{name} must be an RFC 3339 time")))
        })
        .transpose()
}

/// One page of lines, by position in the filtered list.
fn page(call: &Call, scope: &str, lines: &[Arc<LedgerLine>]) -> Result<Response, AdminError> {
    let (limit, after) = pagination::page_params(call, scope)?;
    let start = after.and_then(|a| a.parse::<usize>().ok()).unwrap_or(0);
    let items: Vec<Value> = lines
        .iter()
        .skip(start)
        .take(limit)
        .map(|l| l.to_json())
        .collect();
    let next = (start + items.len() < lines.len())
        .then(|| pagination::encode(&call.rt.cursor_key, scope, &(start + items.len()).to_string()));
    Ok(call.json(StatusCode::OK, &json!({"items": items, "next_cursor": next})))
}

/// One page of the ledger as a feed: the newest request first, a request's lines in their order.
/// The ledger is walked newest line first; the lines of one request admitted together arrive last
/// line first, so they are held until the request is complete and then counted in their order.
struct Feed {
    start: usize,
    limit: usize,
    /// Readable lines counted so far.
    seen: usize,
    items: Vec<Value>,
    more: bool,
    request: Vec<Arc<LedgerLine>>,
}

impl Feed {
    fn new(start: usize, limit: usize) -> Feed {
        Feed {
            start,
            limit,
            seen: 0,
            items: Vec::new(),
            more: false,
            request: Vec::new(),
        }
    }

    /// Takes the next line of the walk; false once the page and the knowledge of a next page
    /// are complete.
    fn push(&mut self, l: &Arc<LedgerLine>) -> bool {
        let same = self
            .request
            .last()
            .is_some_and(|r| r.admitted_at == l.admitted_at && r.request_id == l.request_id);
        if !same && !self.flush() {
            return false;
        }
        self.request.push(Arc::clone(l));
        true
    }

    /// Counts the held request's lines in their order.
    fn flush(&mut self) -> bool {
        for l in std::mem::take(&mut self.request).into_iter().rev() {
            if self.items.len() == self.limit {
                self.more = true;
                return false;
            }
            if self.seen >= self.start {
                self.items.push(l.to_json());
            }
            self.seen += 1;
        }
        true
    }
}

/// `GET /reports/spend/requests/{request_id}`.
pub(crate) async fn request(call: &Call, id: &str) -> Result<Response, AdminError> {
    if call.method != Method::GET {
        return Err(AdminError::method_not_allowed("GET"));
    }
    call.allow_params(&["limit", "cursor"])?;
    require_any(call)?;
    let lines: Vec<Arc<LedgerLine>> = call
        .rt
        .spend_of_request(id)
        .await
        .into_iter()
        .filter(|l| readable(call, l))
        .collect();
    if lines.is_empty() {
        return Err(AdminError::not_found());
    }
    page(call, "spend_request", &lines)
}

fn filter(call: &Call) -> Result<LedgerFilter, AdminError> {
    let p = |k: &str| call.param(k).map(str::to_owned);
    Ok(LedgerFilter {
        from: time_param(call, "from")?,
        to: time_param(call, "to")?,
        organization: p("organization"),
        team: p("team"),
        project: p("project"),
        key: p("key"),
        user: p("user"),
        end_user: p("end_user"),
        model: p("model"),
        deployment: p("deployment"),
        tag: p("tag"),
        status: p("status"),
    })
}

/// `GET /reports/spend/ledger`.
pub(crate) async fn ledger(call: &Call) -> Result<Response, AdminError> {
    if call.method != Method::GET {
        return Err(AdminError::method_not_allowed("GET"));
    }
    let mut allowed = vec!["limit", "cursor", "from", "to"];
    allowed.extend(FILTERS);
    call.allow_params(&allowed)?;
    require_any(call)?;
    let f = filter(call)?;
    let (limit, after) = pagination::page_params(call, "spend_ledger")?;
    let start = after.and_then(|a| a.parse::<usize>().ok()).unwrap_or(0);
    let mut feed = Feed::new(start, limit);
    let mut open = true;
    call.rt
        .for_each_spend_line(&f, true, |l| {
            open = !readable(call, l) || feed.push(l);
            open
        })
        .await;
    if open {
        feed.flush();
    }
    let next = feed.more.then(|| {
        pagination::encode(
            &call.rt.cursor_key,
            "spend_ledger",
            &(start + feed.items.len()).to_string(),
        )
    });
    Ok(call.json(StatusCode::OK, &json!({"items": feed.items, "next_cursor": next})))
}

/// `GET /reports/spend/summary?group_by=<attrs>`, over the lines the ledger's filters select.
pub(crate) async fn summary(call: &Call) -> Result<Response, AdminError> {
    if call.method != Method::GET {
        return Err(AdminError::method_not_allowed("GET"));
    }
    let mut allowed = vec!["limit", "cursor", "from", "to", "group_by"];
    allowed.extend(FILTERS);
    call.allow_params(&allowed)?;
    require_any(call)?;
    let group_by: Vec<String> = call
        .param("group_by")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_owned)
        .collect();
    if let Some(bad) = group_by.iter().find(|a| !GROUP_ATTRS.contains(&a.as_str())) {
        return Err(AdminError::invalid_parameter(format!(
            "group_by: unknown attribute {bad:?}"
        )));
    }
    let f = filter(call)?;
    let mut summary = Summary::new(&group_by);
    call.rt
        .for_each_spend_line(&f, false, |l| {
            if readable(call, l) {
                summary.add(l);
            }
            true
        })
        .await;
    let rows = summary.finish();
    let (limit, after) = pagination::page_params(call, "spend_summary")?;
    let start = after.and_then(|a| a.parse::<usize>().ok()).unwrap_or(0);
    let items: Vec<Value> = rows.iter().skip(start).take(limit).cloned().collect();
    let next = (start + items.len() < rows.len()).then(|| {
        pagination::encode(
            &call.rt.cursor_key,
            "spend_summary",
            &(start + items.len()).to_string(),
        )
    });
    Ok(call.json(StatusCode::OK, &json!({"items": items, "next_cursor": next})))
}

/// `GET /pricing/deployments/{name}`: the merged price and its version.
pub(crate) fn price(call: &Call, name: &str) -> Result<Response, AdminError> {
    if call.method != Method::GET {
        return Err(AdminError::method_not_allowed("GET"));
    }
    call.allow_params(&[])?;
    call.require("pricing:read", &[Scope::Gateway])?;
    let d = call
        .snap
        .providers
        .deployments
        .get(name)
        .ok_or_else(AdminError::not_found)?;
    Ok(call.json(StatusCode::OK, &crate::pricing::effective(call, d)))
}

#[cfg(test)]
mod tests {
    use antirion_limits::cost::CostStatus;
    use antirion_limits::estimate::OutSource;
    use antirion_limits::ledger::{LineStatus, Purpose, Reserved, UsageSource};
    use antirion_limits::usage::Usage;

    use super::*;

    fn line(request: &str, line: u32, at: i64) -> Arc<LedgerLine> {
        Arc::new(LedgerLine {
            request_id: request.into(),
            line,
            purpose: Purpose::Primary,
            shadow: false,
            status: LineStatus::Success,
            admitted_at: UtcMicros(at),
            settled_at: UtcMicros(at),
            organization: String::new(),
            team: String::new(),
            project: String::new(),
            key: String::new(),
            user: None,
            end_user: None,
            tags: Vec::new(),
            model: None,
            route: None,
            provider: None,
            deployment: None,
            upstream_model: None,
            call: antirion_limits::ledger::CallTarget::default(),
            endpoint: String::new(),
            stream: false,
            usage: Usage::default(),
            usage_raw: None,
            usage_source: UsageSource::Upstream,
            cost: None,
            billed: None,
            cost_status: CostStatus::Exact,
            price_version: None,
            service_tier: None,
            avoided: None,
            reserved: Reserved {
                input: 0,
                output: 0,
                usd: None,
                out_source: OutSource::Request,
            },
            budgets: Vec::new(),
        })
    }

    /// The feed's pages from a walk newest line first equal pages of the whole ledger sorted as a
    /// feed: newest request first, a request's lines in their order.
    #[test]
    fn feed_pages_match_the_sorted_ledger() {
        let mut all = Vec::new();
        for (i, at) in [5, 9, 9, 2, 9, 7].into_iter().enumerate() {
            for n in 0..u32::try_from(1 + i % 3).unwrap() {
                all.push(line(&format!("r{i}"), n, at));
            }
        }
        let key = |l: &Arc<LedgerLine>| (l.request_id.clone(), l.line);
        let mut feed_order = all.clone();
        feed_order.sort_by(|a, b| {
            b.admitted_at
                .cmp(&a.admitted_at)
                .then_with(|| b.request_id.cmp(&a.request_id))
                .then_with(|| a.line.cmp(&b.line))
        });
        let mut walk = all.clone();
        walk.sort_by(|a, b| {
            (b.admitted_at, &b.request_id, b.line).cmp(&(a.admitted_at, &a.request_id, a.line))
        });
        for limit in [1, 2, 4, 50] {
            for start in [0, 1, 3, 11, 12, 20] {
                let mut feed = Feed::new(start, limit);
                let mut open = true;
                for l in &walk {
                    open = feed.push(l);
                    if !open {
                        break;
                    }
                }
                if open {
                    feed.flush();
                }
                let want: Vec<_> = feed_order.iter().skip(start).take(limit).map(key).collect();
                let got: Vec<_> = feed
                    .items
                    .iter()
                    .map(|v| {
                        (
                            v["request_id"].as_str().unwrap().to_owned(),
                            u32::try_from(v["line"].as_u64().unwrap()).unwrap(),
                        )
                    })
                    .collect();
                assert_eq!(got, want, "start {start}, limit {limit}");
                assert_eq!(
                    feed.more,
                    start + want.len() < all.len(),
                    "start {start}, limit {limit}"
                );
            }
        }
    }
}
