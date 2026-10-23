//! Decision translation: the OpenAI Decisions wire format and TypeSafe's System One.
//!
//! The pipeline only calls this module. The request is decoded once into [`ir::Request`]; every
//! candidate is dry-run through [`feasible`] and every attempt encodes through [`encode`]. The
//! answer comes back through [`response`], which restores the client's names and typed values.

mod ir;
mod openai;
mod systemone;

pub use ir::{
    Choice, ChoiceValue, DecideMap, DecisionTarget, Decoded, Input, Instructions, Level, Question,
    QuestionKind, Request, Usage,
};

use serde_json::Value;

use crate::xlate::warn::{Refusal, RefusalKind};

/// A deployment that cannot take a request.
#[must_use]
pub fn refusal(param: &str, message: impl Into<String>) -> Refusal {
    Refusal {
        kind: RefusalKind::Unsupported,
        param: param.to_owned(),
        message: message.into(),
    }
}

/// Decodes an OpenAI-shaped decision request. The body has already passed validation; the checks
/// left here are the ones translation needs (it never rejects a request a native deployment can
/// serve).
pub fn decode(body: &Value) -> Result<Decoded, Refusal> {
    openai::decode(body)
}

/// Whether `target` can take the decoded request.
pub fn feasible(target: DecisionTarget, decoded: &Decoded) -> Result<(), Refusal> {
    match target {
        DecisionTarget::SystemOne => systemone::feasible(&decoded.request),
    }
}

/// The upstream body and the mapping its answer needs.
pub fn encode(
    target: DecisionTarget,
    decoded: &Decoded,
    upstream_model: &str,
) -> Result<(Value, DecideMap), Refusal> {
    match target {
        DecisionTarget::SystemOne => systemone::encode(&decoded.request, upstream_model),
    }
}

/// The client-shaped answer of a translated decision.
pub fn response(upstream: &Value, map: &DecideMap, shown: &str) -> Value {
    openai::encode_response(upstream, map, shown)
}

#[cfg(test)]
mod tests;
