//! Endpoints answered without an upstream: the Models list, the model info metadata and stored
//! Responses.

use std::collections::HashMap;
use std::sync::Arc;

use antirion_core::Stage;
use antirion_dialects::Endpoint;
use antirion_error::{Code, GatewayError, Message};
use antirion_providers::{Capabilities, DeploymentRt, Source};
use antirion_router::ModelRt;
use antirion_runtime::identity::KeyRt;
use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{HeaderValue, Response};
use serde_json::{Value, json};

use crate::authorize::{
    candidates, deployment_allowed, model_allowed, model_not_allowed, reachable_deployments,
};
use crate::ctx::{Body, Ctx, full};

/// `GET /v1/models` and `GET /v1/models/{id}`: exactly the Models the key can use, those whose names
/// pass its model lists and that have a deployment it may use.
/// The list shape a models request asks for.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Shape {
    OpenAi,
    Anthropic,
    Gemini,
}

fn shape(parts: &http::request::Parts) -> Shape {
    let path = parts.uri.path();
    if path.starts_with("/gemini/") || path.starts_with("/v1beta/") {
        Shape::Gemini
    } else if path.starts_with("/anthropic/") || parts.headers.contains_key("anthropic-version") {
        Shape::Anthropic
    } else {
        Shape::OpenAi
    }
}

fn query(parts: &http::request::Parts, name: &str) -> Option<String> {
    parts.uri.query()?.split('&').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k == name).then(|| v.to_owned())
    })
}

fn bad_param(param: &str, message: &'static str) -> GatewayError {
    GatewayError::new(Code::InvalidRequest, Stage::Parse, Message::Fixed(message)).param(param.to_owned())
}

/// The Anthropic- or Gemini-shaped page of model names.
fn paged(
    cx: &Ctx,
    parts: &http::request::Parts,
    shape: Shape,
    names: &[&str],
) -> Result<Value, GatewayError> {
    let created_at = antirion_core::UtcMicros(cx.rt.now().0).to_rfc3339_seconds();
    if shape == Shape::Anthropic {
        let limit = match query(parts, "limit") {
            None => 20,
            Some(l) => l
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=1000).contains(n))
                .ok_or_else(|| bad_param("limit", "limit must be between 1 and 1000."))?,
        };
        let (start, end) = match (query(parts, "after_id"), query(parts, "before_id")) {
            (Some(a), _) => {
                let s = names.iter().position(|n| *n == a).map_or(names.len(), |i| i + 1);
                (s, (s + limit).min(names.len()))
            }
            (None, Some(b)) => {
                let e = names.iter().position(|n| *n == b).unwrap_or(0);
                (e.saturating_sub(limit), e)
            }
            (None, None) => (0, limit.min(names.len())),
        };
        let page = names.get(start..end).unwrap_or_default();
        let data: Vec<Value> = page
            .iter()
            .map(|n| json!({"type": "model", "id": n, "display_name": n, "created_at": created_at}))
            .collect();
        return Ok(json!({
            "data": data,
            "has_more": end < names.len(),
            "first_id": page.first(),
            "last_id": page.last(),
        }));
    }
    let size = match query(parts, "pageSize") {
        None => 50,
        Some(l) => l
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=1000).contains(n))
            .ok_or_else(|| bad_param("pageSize", "pageSize must be between 1 and 1000."))?,
    };
    let start = query(parts, "pageToken")
        .and_then(|t| names.iter().position(|n| *n == t))
        .map_or(0, |i| i + 1);
    let end = (start + size).min(names.len());
    let page = names.get(start..end).unwrap_or_default();
    let models: Vec<Value> = page
        .iter()
        .map(|n| {
            json!({"name": format!("models/{n}"), "displayName": n,
                   "supportedGenerationMethods": ["generateContent", "streamGenerateContent", "countTokens"]})
        })
        .collect();
    let mut body = json!({"models": models});
    if end < names.len()
        && let (Some(last), Some(o)) = (page.last(), body.as_object_mut())
    {
        o.insert("nextPageToken".into(), Value::from(*last));
    }
    Ok(body)
}

pub(crate) fn models(
    cx: &mut Ctx,
    parts: &http::request::Parts,
    endpoint: &Endpoint,
    key: &KeyRt,
) -> Response<Body> {
    let shape = shape(parts);
    if shape == Shape::Anthropic {
        cx.envelope = antirion_core::Envelope::Anthropic;
    }
    let cx = &*cx;
    let levels = cx.snap.identity.access_levels(key);
    if let Some(id) = &endpoint.path_param
        && !model_allowed(&levels, id)
    {
        return cx.error(&model_not_allowed(id));
    }
    let now = cx.rt.now();
    let created = now.0 / 1_000_000;
    let object = |name: &str, owned_by: &str| json!({"id": name, "object": "model", "created": created, "owned_by": owned_by});
    let visible = listed(cx, key);
    let body = match &endpoint.path_param {
        None if shape != Shape::OpenAi => {
            let names: Vec<&str> = visible.map(|(name, _)| name).collect();
            match paged(cx, parts, shape, &names) {
                Ok(b) => b,
                Err(e) => return cx.error(&e),
            }
        }
        None => {
            let data: Vec<Value> = visible.map(|(name, m)| object(name, &m.owned_by)).collect();
            json!({"object": "list", "data": data})
        }
        Some(id) => match visible.into_iter().find(|(name, _)| name == id) {
            Some((name, m)) => object(name, &m.owned_by),
            None => {
                let e = GatewayError::new(
                    Code::ModelNotFound,
                    Stage::Resolve,
                    Message::built(format!(
                        "The model `{}` does not exist or you do not have access to it.",
                        id.chars().take(256).collect::<String>()
                    )),
                )
                .param("model");
                return cx.error(&e);
            }
        },
    };
    json_response(cx, &body)
}

/// The names [`models`] and [`model_info`] answer for: every public, enabled, non-wildcard Model
/// and its aliases that passes the key's model lists, is not retired, admits the caller and has no
/// known deployment or one the key may use.
fn listed<'a>(cx: &'a Ctx, key: &'a KeyRt) -> impl Iterator<Item = (&'a str, &'a Arc<ModelRt>)> + 'a {
    let levels = cx.snap.identity.access_levels(key);
    let now = cx.rt.now();
    let caller = crate::authorize::caller(key);
    cx.snap.routing.listed().into_iter().filter(move |(name, m)| {
        model_allowed(&levels, name) && !m.lifecycle.retired(now) && m.admits(caller) && {
            let known = candidates(&cx.snap.providers, &cx.snap.routing, m);
            known.is_empty() || known.iter().any(|d| deployment_allowed(&levels, d))
        }
    })
}

/// `GET /v1/model/info`: one entry per name [`models`] lists that has an enabled deployment the key
/// may use, with the capability metadata clients reading LiteLLM's endpoint expect. It is not an
/// OpenAI standard endpoint; the standard Models API is `/v1/models`.
pub(crate) fn model_info(cx: &Ctx, key: &KeyRt) -> Response<Body> {
    let mut by_model: HashMap<&str, Option<Facts>> = HashMap::new();
    let mut entries: Vec<(String, Value)> = Vec::new();
    for (name, model) in listed(cx, key) {
        let computed = by_model.entry(model.name.as_str()).or_insert_with(|| {
            let deployments = eligible(cx, key, model);
            (!deployments.is_empty()).then(|| facts(&deployments))
        });
        if let Some(facts) = computed {
            entries.push((name.to_owned(), facts.to_value()));
        }
    }
    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let data: Vec<Value> = entries
        .into_iter()
        .map(|(name, info)| json!({"model_name": name, "model_info": info}))
        .collect();
    json_response(cx, &json!({"data": data}))
}

/// The deployments of a Model's route that can serve and that the key may use, `provider/model`
/// leaves included.
fn eligible(cx: &Ctx, key: &KeyRt, model: &ModelRt) -> Vec<Arc<DeploymentRt>> {
    let levels = cx.snap.identity.access_levels(key);
    reachable_deployments(&cx.snap, model)
        .into_iter()
        .filter(|d| d.usable() && deployment_allowed(&levels, d))
        .collect()
}

/// The operation classes a Model's `mode` is read from, by priority, with LiteLLM's names.
const MODES: [(&str, &str); 7] = [
    ("generate", "chat"),
    ("embed", "embedding"),
    ("image_generate", "image_generation"),
    ("transcribe", "audio_transcription"),
    ("speech", "audio_speech"),
    ("moderate", "moderation"),
    ("rerank", "rerank"),
];

/// One Model name's capabilities, aggregated over the deployments that can serve it.
struct Facts {
    /// The smallest input bound, when every deployment states one.
    max_input: Option<u64>,
    /// Whether every deployment states an input bound.
    input_complete: bool,
    /// The smallest output bound, when every deployment states one.
    max_output: Option<u64>,
    /// Whether every deployment states an output bound.
    output_complete: bool,
    /// Whether any deployment takes images.
    vision: bool,
    /// Whether any deployment takes tools.
    tools: bool,
    /// Whether any deployment reasons.
    reasoning: bool,
    /// The operations any deployment serves, in [`MODES`] order.
    operations: [bool; MODES.len()],
}

impl Default for Facts {
    fn default() -> Facts {
        Facts {
            max_input: None,
            input_complete: true,
            max_output: None,
            output_complete: true,
            vision: false,
            tools: false,
            reasoning: false,
            operations: [false; MODES.len()],
        }
    }
}

impl Facts {
    /// The name's primary mode: the first operation in [`MODES`] order, `chat` when none.
    fn mode(&self) -> &'static str {
        MODES
            .iter()
            .zip(self.operations.iter())
            .find(|(_, on)| **on)
            .map_or("chat", |((_, mode), _)| *mode)
    }

    /// The entry's `model_info` object: booleans and the mode always, a limit only when every
    /// deployment states it.
    fn to_value(&self) -> Value {
        let mut info = serde_json::Map::new();
        if self.input_complete
            && let Some(n) = self.max_input
        {
            info.insert("max_input_tokens".into(), Value::from(n));
        }
        if self.output_complete
            && let Some(n) = self.max_output
        {
            info.insert("max_output_tokens".into(), Value::from(n));
        }
        info.insert("supports_vision".into(), Value::from(self.vision));
        info.insert("supports_function_calling".into(), Value::from(self.tools));
        info.insert("supports_reasoning".into(), Value::from(self.reasoning));
        info.insert("mode".into(), Value::from(self.mode()));
        Value::Object(info)
    }
}

/// Aggregates the deployments of one name: any for capabilities and the mode, the smallest bound
/// when every deployment states one.
fn facts(deployments: &[Arc<DeploymentRt>]) -> Facts {
    let mut out = Facts::default();
    for d in deployments {
        let known =
            d.provider
                .catalog
                .model_info(d.provider.kind.as_str(), &d.base_model, d.provider.kind.generic());
        let input = number(&d.capabilities, "max_input_tokens")
            .or_else(|| number(&d.capabilities, "context_window"))
            .or_else(|| known.as_ref().and_then(|k| k.context_window));
        out.input_complete &= input.is_some();
        out.max_input = smallest(out.max_input, input);
        let output = number(&d.capabilities, "max_output_tokens")
            .or_else(|| known.as_ref().and_then(|k| k.max_output_tokens));
        out.output_complete &= output.is_some();
        out.max_output = smallest(out.max_output, output);
        let images = known
            .as_ref()
            .and_then(|k| k.modalities.as_ref())
            .map(|m| m.input.iter().any(|s| s == "image"));
        out.vision |= configured(&d.capabilities, "vision").or(images).unwrap_or(false);
        out.tools |= configured(&d.capabilities, "tools").unwrap_or(false);
        out.reasoning |= configured(&d.capabilities, "reasoning").unwrap_or(false);
        for (i, (class, _)) in MODES.iter().enumerate() {
            if let Some(on) = out.operations.get_mut(i) {
                *on |= d.capabilities.operation(class);
            }
        }
    }
    out
}

fn number(caps: &Capabilities, name: &str) -> Option<u64> {
    caps.get(name).0.as_u64()
}

fn smallest(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// A flag the deployment, its provider or the catalog sets. A provider type's optimistic routing
/// default is not a capability claim, so it is not reported.
fn configured(caps: &Capabilities, name: &str) -> Option<bool> {
    let (value, source) = caps.get(name);
    (source != Source::TypeDefault).then(|| value.as_bool().unwrap_or(false))
}

/// `count_tokens` for a model with no native counting API: a local estimate of the input, about four
/// characters per token, never sent upstream.
pub(crate) fn count_estimate(cx: &Ctx, raw: &[u8], responses: bool) -> Response<Body> {
    let body: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let mut chars = 0usize;
    let mut walk = vec![&body];
    while let Some(v) = walk.pop() {
        match v {
            Value::String(s) => chars += s.chars().count(),
            Value::Array(a) => walk.extend(a.iter()),
            Value::Object(o) => walk.extend(o.iter().filter(|(k, _)| k.as_str() != "model").map(|(_, v)| v)),
            _ => {}
        }
    }
    let tokens = chars.div_ceil(4).max(1);
    let body = if responses {
        crate::apis::responses_count(tokens as u64)
    } else {
        json!({"input_tokens": tokens})
    };
    let mut resp = json_response(cx, &body);
    resp.headers_mut().insert(
        http::HeaderName::from_static(antirion_core::headers::TOKEN_COUNT_SOURCE),
        HeaderValue::from_static("estimate"),
    );
    resp
}

pub(crate) fn json_response(cx: &Ctx, body: &Value) -> Response<Body> {
    let mut resp = Response::new(full(Bytes::from(serde_json::to_vec(body).unwrap_or_default())));
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    cx.finish(&mut resp, false);
    resp
}

#[cfg(test)]
mod tests {
    use antirion_providers::ProviderType;

    use super::*;

    fn class(name: &str) -> usize {
        MODES.iter().position(|(c, _)| *c == name).unwrap()
    }

    #[test]
    fn the_mode_is_the_first_operation_in_priority_order() {
        let mut facts = Facts::default();
        assert_eq!(facts.mode(), "chat", "a name with no operation is a chat Model");
        if let Some(on) = facts.operations.get_mut(class("speech")) {
            *on = true;
        }
        assert_eq!(facts.mode(), "audio_speech");
        if let Some(on) = facts.operations.get_mut(class("embed")) {
            *on = true;
        }
        assert_eq!(facts.mode(), "embedding", "an earlier class wins");
    }

    #[test]
    fn limits_keep_the_smallest_known_value() {
        assert_eq!(smallest(None, None), None);
        assert_eq!(smallest(Some(8), None), Some(8));
        assert_eq!(smallest(None, Some(8)), Some(8));
        assert_eq!(smallest(Some(8), Some(4)), Some(4));
        assert_eq!(smallest(Some(4), Some(8)), Some(4));
    }

    #[test]
    fn a_type_default_is_not_a_capability_claim() {
        let declared = Capabilities::resolve(
            ProviderType::OpenAiCompatible,
            None,
            Some(&json!({"reasoning": false, "vision": true})),
        );
        assert_eq!(configured(&declared, "reasoning"), Some(false));
        assert_eq!(configured(&declared, "vision"), Some(true));
        let default = Capabilities::resolve(ProviderType::OpenAiCompatible, None, None);
        assert_eq!(configured(&default, "reasoning"), None);
        assert_eq!(configured(&default, "tools"), None);
    }
}
