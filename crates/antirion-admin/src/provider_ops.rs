//! Provider actions: `:rotate_credential` puts a new credential in place, keeping the old one as a
//! standby until its overlap ends. Credential tests are `provider_check`.

use std::time::Duration;

use antirion_core::UtcMicros;
use antirion_identity::rbac::Scope;
use antirion_providers::ProviderRt;
use antirion_runtime::changes::Target;
use axum::response::Response;
use http::{Method, StatusCode};
use serde_json::{Map, Value, json};

use crate::call::Call;
use crate::commit::{Proposal, api_write};
use crate::error::AdminError;

/// The longest overlap a rotation keeps the old credential.
const MAX_OVERLAP: Duration = Duration::from_secs(7 * 86_400);

fn invalid(field: &str, code: antirion_config::DetailCode, message: &str) -> AdminError {
    AdminError::validation(vec![antirion_config::ConfigError::new(
        &antirion_config::DocPath::root().key(field),
        code,
        message,
    )])
}

fn members(call: &Call, allowed: &[&str]) -> Result<Map<String, Value>, AdminError> {
    let body = call.json_body()?;
    if let Some(k) = body.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid(
            k,
            antirion_config::DetailCode::UnknownField,
            "unknown member",
        ));
    }
    Ok(body)
}

fn provider<'a>(call: &'a Call, name: &str) -> Result<&'a std::sync::Arc<ProviderRt>, AdminError> {
    call.snap
        .providers
        .providers
        .get(name)
        .ok_or_else(AdminError::not_found)
}

/// `POST /providers/{name}:rotate_credential`.
pub(crate) async fn rotate(call: &Call, name: &str) -> Result<Response, AdminError> {
    if call.method != Method::POST {
        return Err(AdminError::method_not_allowed("POST"));
    }
    call.allow_params(&[])?;
    call.require("providers:write", &[Scope::Gateway])?;
    let body = members(call, &["credential", "retire_after", "key"])?;
    provider(call, name)?;
    let credential = body
        .get("credential")
        .filter(|c| c.as_str().is_some_and(|s| !s.is_empty()) || c.is_object())
        .cloned()
        .ok_or_else(|| {
            invalid(
                "credential",
                antirion_config::DetailCode::Required,
                "credential is required",
            )
        })?;
    let overlap = body
        .get("retire_after")
        .and_then(Value::as_str)
        .and_then(antirion_config::scalar::parse_duration)
        .ok_or_else(|| {
            invalid(
                "retire_after",
                antirion_config::DetailCode::InvalidFormat,
                "retire_after is a duration",
            )
        })?;
    if overlap > MAX_OVERLAP {
        return Err(invalid(
            "retire_after",
            antirion_config::DetailCode::OutOfRange,
            "retire_after is at most 7d",
        ));
    }
    let key = body.get("key").and_then(Value::as_str).map(str::to_owned);
    let now = call.rt.now();
    let standby = format!(
        "{}-retiring-{}",
        key.as_deref().unwrap_or(antirion_providers::SINGLE_KEY),
        now.0.div_euclid(1_000_000)
    );
    let resource = format!("providers/{name}");
    let (provider_name, standby_name) = (name.to_owned(), standby.clone());
    let committed = api_write(
        call,
        "POST providers:rotate_credential",
        &resource,
        StatusCode::OK,
        |state| {
            let mut candidate = state.doc.clone();
            let auth = candidate
                .pointer_mut(&format!("/providers/{provider_name}/auth"))
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    invalid(
                        "credential",
                        antirion_config::DetailCode::Conflict,
                        "the provider has no API-key credential to rotate",
                    )
                })?;
            let old = match &key {
                None => {
                    let old = auth.remove("api_key").ok_or_else(|| {
                        invalid(
                            "credential",
                            antirion_config::DetailCode::Conflict,
                            "the provider has no auth.api_key; name the key to rotate",
                        )
                    })?;
                    auth.insert(
                        "api_keys".into(),
                        json!([{"name": antirion_providers::SINGLE_KEY, "key": credential.clone()}]),
                    );
                    old
                }
                Some(k) => {
                    let entry = auth
                        .get_mut("api_keys")
                        .and_then(Value::as_array_mut)
                        .and_then(|keys| {
                            keys.iter_mut()
                                .find(|e| e.get("name").and_then(Value::as_str) == Some(k.as_str()))
                        })
                        .and_then(Value::as_object_mut)
                        .ok_or_else(|| {
                            invalid(
                                "key",
                                antirion_config::DetailCode::UnresolvedReference,
                                "the provider has no key of that name",
                            )
                        })?;
                    entry
                        .insert("key".into(), credential.clone())
                        .unwrap_or(Value::Null)
                }
            };
            let keys = auth
                .entry("api_keys")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .ok_or_else(AdminError::internal)?;
            keys.push(json!({"name": standby_name, "key": old, "weight": 0}));
            Ok(Proposal {
                candidate,
                written: vec![Target::Resource("providers", provider_name.clone())],
                credentials: Vec::new(),
                meta_edits: Vec::new(),
            })
        },
    )
    .await?;
    let retire_at = now.saturating_add(overlap);
    if call
        .rt
        .schedule_retirement(name, &standby, retire_at)
        .await
        .is_err()
    {
        return Err(AdminError::store_unavailable());
    }
    let snap = call.rt.gens.load();
    let shown = antirion_runtime::snapshot::redacted(&snap.config.doc, &snap.secret_paths);
    let spec = shown
        .pointer(&format!("/providers/{name}"))
        .cloned()
        .unwrap_or(Value::Null);
    let meta = snap
        .config
        .meta
        .resources
        .get("providers")
        .and_then(|m| m.get(name));
    let mut item = crate::resources::representation(name, &spec, meta, snap.generation);
    crate::hints::decorate(&snap, name, &mut item);
    decorate(&snap, name, &mut item, call.rt.now());
    if let Some(m) = item.get_mut("metadata").and_then(Value::as_object_mut) {
        m.insert(
            "retires_at".into(),
            json!({standby: retire_at.to_rfc3339_seconds()}),
        );
    }
    let resp = call.json(StatusCode::OK, &item);
    Ok(call.mutated(resp, committed.generation).await)
}

/// Adds `metadata.credential_rotated_at` and `metadata.rotation_due` to a provider's
/// representation.
pub(crate) fn decorate(snap: &antirion_runtime::Snapshot, name: &str, item: &mut Value, now: UtcMicros) {
    let Some(meta) = snap
        .config
        .meta
        .resources
        .get("providers")
        .and_then(|m| m.get(name))
    else {
        return;
    };
    let max_age = snap
        .config
        .doc
        .pointer(&format!("/providers/{name}/credential_rotation_max_age"))
        .and_then(Value::as_str)
        .and_then(antirion_config::scalar::parse_duration);
    let due = max_age.is_some_and(|max| {
        meta.credentials_rotated_at
            .values()
            .any(|at| UtcMicros::parse_rfc3339(at).is_ok_and(|t| now.since(t) > max))
    });
    if let Some(m) = item.get_mut("metadata").and_then(Value::as_object_mut) {
        m.insert("credential_rotated_at".into(), json!(meta.credentials_rotated_at));
        m.insert("rotation_due".into(), json!(due));
    }
}
