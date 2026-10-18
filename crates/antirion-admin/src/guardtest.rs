//! `POST /guardrails/{name}:test`: one guardrail run on a sample text or body. `POST
//! /guardrails:test`: the same for a guardrail that is not saved (a form being filled in), compiled
//! with the validation of a save and stored nowhere.

use std::sync::Arc;

use antirion_config::{ConfigError, DetailCode, DocPath};
use antirion_core::audit::Outcome;
use antirion_guardrails::Def;
use antirion_identity::rbac::Scope;
use antirion_runtime::guardtest::{Sample, TestError};
use axum::response::Response;
use http::{Method, StatusCode};
use serde_json::{Map, Value, json};

use crate::call::Call;
use crate::error::AdminError;

fn invalid(field: &str, code: DetailCode, message: &str) -> AdminError {
    AdminError::validation(vec![ConfigError::new(
        &DocPath::parse_dotted(field),
        code,
        message,
    )])
}

/// The stage and the sample of a test: `stage` (required unless `default_input`), and `text` or
/// `body`, members of `obj`; `at` prefixes the reported field paths.
fn sample_of(obj: &Map<String, Value>, at: &str, default_input: bool) -> Result<(bool, Sample), AdminError> {
    let field = |k: &str| {
        if at.is_empty() {
            k.to_owned()
        } else {
            format!("{at}.{k}")
        }
    };
    if let Some(extra) = obj
        .keys()
        .find(|k| !matches!(k.as_str(), "stage" | "text" | "body"))
    {
        return Err(invalid(
            &field(extra),
            DetailCode::UnknownField,
            "not a field of a guardrail test",
        ));
    }
    let output = match obj.get("stage") {
        Some(Value::String(s)) if s == "input" => false,
        Some(Value::String(s)) if s == "output" => true,
        Some(_) => {
            return Err(invalid(
                &field("stage"),
                DetailCode::InvalidEnum,
                "must be input or output",
            ));
        }
        None if default_input => false,
        None => {
            return Err(invalid(
                &field("stage"),
                DetailCode::Required,
                "the stage is required",
            ));
        }
    };
    let sample = match (obj.get("text"), obj.get("body")) {
        (Some(Value::String(t)), None) => Sample::Text(t.clone()),
        (None, Some(b @ Value::Object(_))) => Sample::Body(b.clone()),
        (Some(_), Some(_)) => {
            return Err(invalid(
                &field("body"),
                DetailCode::Conflict,
                "give either text or body, not both",
            ));
        }
        (Some(_), None) => {
            return Err(invalid(
                &field("text"),
                DetailCode::TypeMismatch,
                "expected a string",
            ));
        }
        (None, Some(_)) => {
            return Err(invalid(
                &field("body"),
                DetailCode::TypeMismatch,
                "expected an object",
            ));
        }
        (None, None) => {
            return Err(invalid(
                &field("text"),
                DetailCode::Required,
                "a text or a body is required",
            ));
        }
    };
    Ok((output, sample))
}

/// Runs `def` on the sample through the data plane's tester.
async fn run_def(
    call: &Call,
    def: Arc<Def>,
    output: bool,
    sample: Sample,
    stage_field: &str,
) -> Result<Value, AdminError> {
    let tester = call
        .rt
        .state
        .guard_tests
        .tester()
        .ok_or_else(AdminError::internal)?;
    match tester.test(Arc::clone(&call.snap), def, output, sample).await {
        Ok(result) => Ok(result),
        Err(TestError::Unknown) => Err(AdminError::not_found()),
        Err(TestError::Invalid(m)) => Err(invalid(stage_field, DetailCode::Conflict, &m)),
        Err(TestError::Unavailable) => Err(AdminError::internal()),
    }
}

/// Runs guardrail `name` on the request's sample.
pub(crate) async fn run(call: &Call, name: &str) -> Result<Response, AdminError> {
    if call.method != Method::POST {
        return Err(AdminError::method_not_allowed("POST"));
    }
    call.allow_params(&[])?;
    call.require("policies:read", &[Scope::Gateway])?;
    let def = Arc::clone(call.snap.guardrails.get(name).ok_or_else(AdminError::not_found)?);
    let (output, sample) = sample_of(&call.json_body()?, "", false)?;
    let result = run_def(call, def, output, sample, "stage").await?;
    Ok(call.json(StatusCode::OK, &result))
}

/// The keys a guardrail spec sends its calls to.
const DESTINATIONS: &[&str] = &["url", "base_url", "endpoint", "region", "provider"];

/// Every destination of a spec, as (key, value) pairs in document order.
fn destinations(v: &Value, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                if DESTINATIONS.contains(&k.as_str()) {
                    out.push((k.clone(), x.clone()));
                } else {
                    destinations(x, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|x| destinations(x, out)),
        _ => {}
    }
}

/// What an unsaved test may resolve: under change control, literal secrets only, since nothing is
/// approved; a `[REDACTED]` placeholder stands for the saved guardrail's secret, which is sent
/// only where the saved guardrail sends it (the same type and destinations).
fn check_secrets(call: &Call, name: &str, spec: &Value, candidate: &Value) -> Result<(), AdminError> {
    let (references, redacted) = crate::provider_check::secret_forms(spec);
    if (references || redacted) && crate::change_control::governed(&call.snap.config.doc, candidate) {
        return Err(invalid(
            "spec",
            DetailCode::Conflict,
            "guardrails are under change control: test a guardrail before saving it with literal secrets only",
        ));
    }
    if redacted {
        let saved = call
            .snap
            .config
            .doc
            .pointer("/guardrails/definitions")
            .and_then(|d| d.get(name));
        let routed = |v: &Value| {
            let mut out = Vec::new();
            destinations(v, &mut out);
            out
        };
        let same = saved.is_some_and(|s| s.get("type") == spec.get("type") && routed(s) == routed(spec));
        if !same {
            return Err(invalid(
                "spec",
                DetailCode::Conflict,
                "a [REDACTED] value is the saved guardrail's secret, sent only to its saved type and destinations",
            ));
        }
    }
    Ok(())
}

/// A name for an unsaved guardrail that no saved one has.
fn free_name(call: &Call) -> String {
    let defs = call.snap.config.doc.pointer("/guardrails/definitions");
    let taken = |n: &str| defs.and_then(|d| d.get(n)).is_some();
    (0..)
        .map(|i| {
            if i == 0 {
                "unsaved".to_owned()
            } else {
                format!("unsaved_{i}")
            }
        })
        .find(|n| !taken(n))
        .unwrap_or_default()
}

/// `POST /guardrails:test`: runs an unsaved guardrail spec on a sample text or body. `name` is the
/// name it would be saved under (an existing guardrail's when editing one); the spec is compiled as
/// a save of it would be, with the same validation errors, and nothing is stored.
pub(crate) async fn run_unsaved(call: &Call) -> Result<Response, AdminError> {
    if call.method != Method::POST {
        return Err(AdminError::method_not_allowed("POST"));
    }
    call.allow_params(&[])?;
    call.require("policies:write", &[Scope::Gateway])?;
    let body = call.json_body()?;
    if let Some(extra) = body
        .keys()
        .find(|k| !matches!(k.as_str(), "name" | "spec" | "input"))
    {
        return Err(invalid(
            extra,
            DetailCode::UnknownField,
            "not a field of a guardrail test",
        ));
    }
    let name = match body.get("name") {
        None => free_name(call),
        Some(Value::String(n)) if !n.is_empty() => n.clone(),
        Some(_) => {
            return Err(invalid(
                "name",
                DetailCode::TypeMismatch,
                "name must be a non-empty string",
            ));
        }
    };
    let mut spec = match body.get("spec") {
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => {
            return Err(invalid(
                "spec",
                DetailCode::TypeMismatch,
                "spec must be an object",
            ));
        }
        None => return Err(invalid("spec", DetailCode::Required, "spec is required")),
    };
    let (output, sample) = match body.get("input") {
        Some(Value::Object(input)) => sample_of(input, "input", true)?,
        Some(_) => {
            return Err(invalid(
                "input",
                DetailCode::TypeMismatch,
                "input must be an object",
            ));
        }
        None => return Err(invalid("input", DetailCode::Required, "input is required")),
    };
    // A rule turned off still compiles for its test.
    if let Some(o) = spec.as_object_mut() {
        o.remove("enabled");
    }
    let current = Arc::clone(&call.snap.config);
    let mut candidate = current.doc.clone();
    let Some(root) = candidate.as_object_mut() else {
        return Err(AdminError::internal());
    };
    let guardrails = root
        .entry("guardrails")
        .or_insert_with(|| Value::Object(Map::new()));
    if !guardrails.is_object() {
        *guardrails = Value::Object(Map::new());
    }
    if let Some(g) = guardrails.as_object_mut() {
        let defs = g
            .entry("definitions")
            .or_insert_with(|| Value::Object(Map::new()));
        if !defs.is_object() {
            *defs = Value::Object(Map::new());
        }
        if let Some(map) = defs.as_object_mut() {
            map.insert(name.clone(), spec.clone());
        }
    }
    check_secrets(call, &name, &spec, &candidate)?;
    let def = Box::pin(call.rt.compile_guardrail(candidate, &current, &name))
        .await
        .map_err(AdminError::validation)?;
    let kind = def.type_name();
    let result = run_def(call, def, output, sample, "input.stage").await?;
    call.audit_with(
        "POST guardrails:test",
        &format!("guardrails/{name}"),
        Outcome::Success,
        StatusCode::OK,
        json!({"saved": false, "type": kind, "verdict": result.get("verdict")}),
    )
    .await;
    Ok(call.json(StatusCode::OK, &result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_are_found_anywhere_in_a_spec() {
        let mut out = Vec::new();
        destinations(
            &json!({"type": "webhook", "webhook": {"url": "https://a", "headers": {"x": "[REDACTED]"}},
                    "lakera": {"base_url": "https://b"}}),
            &mut out,
        );
        assert_eq!(
            out,
            [
                ("url".to_owned(), json!("https://a")),
                ("base_url".to_owned(), json!("https://b"))
            ]
        );
    }

    #[test]
    fn an_unsaved_test_takes_its_stage_from_the_input_or_defaults_to_input() {
        let obj = |v: Value| v.as_object().cloned().unwrap();
        let (output, _) = sample_of(&obj(json!({"text": "x"})), "input", true).unwrap();
        assert!(!output);
        let (output, _) = sample_of(&obj(json!({"text": "x", "stage": "output"})), "input", true).unwrap();
        assert!(output);
        assert!(sample_of(&obj(json!({"text": "x"})), "", false).is_err());
        assert!(sample_of(&obj(json!({"text": "x", "stage": "both"})), "input", true).is_err());
        assert!(sample_of(&obj(json!({"text": "x", "stage": 1})), "input", true).is_err());
    }
}
