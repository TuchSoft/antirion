//! Decision translation orchestration: decode the OpenAI-shaped request once, decide every
//! deployment's eligibility, build each attempt's System One body and map its answer back.

use std::collections::HashMap;
use std::sync::Arc;

use antirion_core::Stage;
use antirion_dialects::decide::{self, DecideMap, DecisionTarget, Decoded};
use antirion_dialects::xlate::warn::Refusal;
use antirion_error::{GatewayError, Message};
use antirion_providers::DeploymentRt;
use serde_json::Value;

use crate::cell::{Cell, cell};
use crate::parse::Parsed;
use crate::reasons;

/// The decoded request of a translated decision, shared by every attempt.
#[derive(Debug)]
pub(crate) struct DecisionXlate {
    pub(crate) decoded: Arc<Decoded>,
}

/// What one translated attempt sets aside to map its answer back.
#[derive(Debug)]
pub(crate) struct AttemptDecide {
    pub(crate) map: DecideMap,
}

/// What `resolve` learned: the decoded request (when some candidate translates) and each
/// translated candidate's verdict (`None` when feasible).
pub(crate) struct Resolution {
    pub(crate) xlate: Option<Arc<DecisionXlate>>,
    pub(crate) verdicts: HashMap<String, Option<Refusal>>,
}

/// Decodes the request when a candidate is a decision translation, then checks every translated
/// candidate. Identity candidates are left to the chat translation verdicts.
pub(crate) fn resolve(
    endpoint: &antirion_dialects::Endpoint,
    parsed: &Parsed,
    servable: &[&Arc<DeploymentRt>],
) -> Result<Resolution, GatewayError> {
    let translated = servable
        .iter()
        .any(|d| matches!(cell(endpoint, parsed, d), Some(Cell::Decide { .. })));
    if !translated {
        return Ok(Resolution {
            xlate: None,
            verdicts: HashMap::new(),
        });
    }
    let Parsed::Json { doc: Some(doc), .. } = parsed else {
        return Err(untranslatable());
    };
    let decoded = decide::decode(doc);
    let xlate = match &decoded {
        Ok(d) => Some(Arc::new(DecisionXlate {
            decoded: Arc::new(d.clone()),
        })),
        Err(_) => None,
    };
    let mut verdicts = HashMap::new();
    for d in servable {
        let Some(Cell::Decide { to }) = cell(endpoint, parsed, d) else {
            continue;
        };
        let verdict = match (&decoded, &xlate) {
            (Ok(_), Some(x)) => decide::feasible(to, &x.decoded).err(),
            (Err(r), _) => Some(r.clone()),
            _ => None,
        };
        verdicts.insert(d.name.clone(), verdict);
    }
    let all_translated = servable
        .iter()
        .all(|d| matches!(cell(endpoint, parsed, d), Some(Cell::Decide { .. })));
    if all_translated {
        let refusal = match &decoded {
            Err(r) => Some(r.clone()),
            Ok(_) => verdicts.values().filter_map(|v| v.as_ref()).next().cloned(),
        };
        if let Some(r) = refusal {
            return Err(crate::xlate::refusal_error(&r));
        }
    }
    Ok(Resolution { xlate, verdicts })
}

/// The upstream body of one translated decision attempt and the map its answer needs.
pub(crate) fn body(
    x: &DecisionXlate,
    target: DecisionTarget,
    d: &DeploymentRt,
) -> Result<(Value, DecideMap), GatewayError> {
    decide::encode(target, &x.decoded, &d.model).map_err(|r| crate::xlate::refusal_error(&r))
}

/// The client-shaped answer of a translated decision.
pub(crate) fn response(upstream: &Value, map: &DecideMap, shown: &str) -> Value {
    decide::response(upstream, map, shown)
}

fn untranslatable() -> GatewayError {
    GatewayError::with_reason(
        reasons::UNTRANSLATABLE_REQUEST,
        Stage::Resolve,
        Message::Fixed("The decision request could not be translated."),
    )
}
