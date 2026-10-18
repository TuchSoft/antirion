//! Provider checks. `POST /providers/{name}:test` proves a saved provider's credential and
//! `POST /providers:test` one of a provider not saved yet: by listing the provider's models where
//! its type has a listing call, else with the cheapest call that proves the credential, so neither
//! needs a deployment (one named in the body is tested as before, with its own model).
//! `GET /providers/{name}/models` lists the provider's models with the catalog's metadata and
//! prices, or the catalog's models of the type when the provider cannot list them.
//!
//! Every call goes through the provider's egress policy, TLS profile and credential machinery. A
//! saved provider's test is recorded per key for `GET /runtime/providers`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use antirion_catalog::modelsdev::ModelInfo;
use antirion_config::DetailCode;
use antirion_core::audit::Outcome;
use antirion_core::{RequestId, UtcMicros};
use antirion_egress::{OutboundRequest, Payload};
use antirion_identity::rbac::Scope;
use antirion_limits::cost::CostStatus;
use antirion_limits::estimate::OutSource;
use antirion_limits::ledger::{CallTarget, LedgerLine, LineStatus, Purpose, Reserved, UsageSource};
use antirion_limits::money::{Dec, Nusd};
use antirion_limits::usage::{Units, Usage};
use antirion_providers::listing::{Fetched, Listed, Listing, TestMethod, TestRecord};
use antirion_providers::{
    DeploymentRt, ProviderRt, ProviderType, Publisher, UpstreamAuth, UpstreamCall, UpstreamOp,
};
use axum::response::Response;
use http::{HeaderValue, Method, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Map, Value, json};

use crate::call::Call;
use crate::error::AdminError;

/// The longest a test or one page of a list waits for its answer.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The bytes of a call's answer read.
const CALL_BODY: usize = 64 << 10;
/// The bytes of one page of a model list read.
const LIST_BODY: usize = 8 << 20;
/// The most pages of a model list read.
const MAX_PAGES: usize = 20;
/// How long a node keeps a provider's model list.
const LIST_MAX_AGE: Duration = Duration::from_secs(300);

/// The model a minimal call uses on a Vertex provider without a deployment.
const VERTEX_PROBE_MODEL: &str = "gemini-2.5-flash";
/// The model a minimal call uses on a Bedrock provider without a deployment.
const BEDROCK_PROBE_MODEL: &str = "amazon.nova-micro-v1:0";

fn invalid(field: &str, code: DetailCode, message: &str) -> AdminError {
    AdminError::validation(vec![antirion_config::ConfigError::new(
        &antirion_config::DocPath::root().key(field),
        code,
        message,
    )])
}

fn members(call: &Call, allowed: &[&str]) -> Result<Map<String, Value>, AdminError> {
    if call.body.is_empty() {
        return Ok(Map::new());
    }
    let body = call.json_body()?;
    if let Some(k) = body.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid(k, DetailCode::UnknownField, "unknown member"));
    }
    Ok(body)
}

/// An upstream answer.
struct Answer {
    status: u16,
    upstream_id: Option<String>,
    body: Vec<u8>,
}

/// Why a call did not prove the credential: the upstream's status when it answered, and a
/// message scrubbed of the provider's secrets.
#[derive(Clone, Debug)]
struct Failure {
    status: Option<u16>,
    message: String,
}

impl Failure {
    fn local(message: &str) -> Failure {
        Failure {
            status: None,
            message: message.to_owned(),
        }
    }
}

/// An upstream error's message, with every secret of the provider taken out.
fn scrubbed(body: &[u8], secrets: &[String]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let m = v
        .pointer("/error/message")
        .or_else(|| v.get("message"))
        .or_else(|| v.pointer("/error"))
        .and_then(Value::as_str)?;
    let mut out: String = m.chars().take(500).collect();
    for s in secrets.iter().filter(|s| s.len() >= 8) {
        out = out.replace(s.as_str(), "[REDACTED]");
    }
    Some(out)
}

fn rejected(answer: &Answer, p: &ProviderRt) -> Failure {
    let status = answer.status;
    Failure {
        status: Some(status),
        message: scrubbed(&answer.body, &p.secrets)
            .unwrap_or_else(|| format!("The provider answered {status}.")),
    }
}

/// Authorizes `request` with the provider's credential, sends it through egress and reads up to
/// `limit` bytes of the answer.
async fn exchange(
    call: &Call,
    d: &DeploymentRt,
    mut request: OutboundRequest,
    limit: usize,
) -> Result<Answer, Failure> {
    let tls = call
        .snap
        .security
        .egress_tls
        .clone()
        .unwrap_or_else(|| d.provider.tls.clone());
    if d.authorize(&mut request, &call.rt.creds, &tls).await.is_err() {
        return Err(Failure::local(
            "The gateway could not obtain the provider's credential.",
        ));
    }
    let resp = match tokio::time::timeout(TIMEOUT, call.rt.egress.send(request)).await {
        Err(_) => return Err(Failure::local("The provider did not answer in time.")),
        Ok(Err(e)) => return Err(Failure::local(&e.to_string())),
        Ok(Ok(resp)) => resp,
    };
    let status = resp.status.as_u16();
    let upstream_id = ["x-request-id", "request-id", "x-amzn-requestid"]
        .iter()
        .find_map(|h| resp.headers.get(*h).and_then(|v| v.to_str().ok()))
        .map(str::to_owned);
    let mut read = Vec::new();
    let mut body = resp.body;
    let reading = async {
        while let Some(Ok(frame)) = body.frame().await {
            if let Ok(data) = frame.into_data() {
                read.extend_from_slice(&data);
                if read.len() >= limit {
                    break;
                }
            }
        }
    };
    if tokio::time::timeout(TIMEOUT, reading).await.is_err() {
        return Err(Failure::local("The provider's answer did not arrive in time."));
    }
    Ok(Answer {
        status,
        upstream_id,
        body: read,
    })
}

/// A provider's model list, every page of it, read with `key`.
struct ListOutcome {
    result: Result<Vec<Listed>, Failure>,
    status: Option<u16>,
    upstream_id: Option<String>,
}

async fn list(call: &Call, p: &Arc<ProviderRt>, listing: Listing, key: Option<&str>) -> ListOutcome {
    let d = DeploymentRt::direct(p, "", "", None);
    let mut models: Vec<Listed> = Vec::new();
    let mut page: Option<String> = None;
    let mut out = ListOutcome {
        result: Ok(Vec::new()),
        status: None,
        upstream_id: None,
    };
    for _ in 0..MAX_PAGES {
        let request_id = RequestId::generate();
        let Ok(request) = d.listing_request(listing, key, &request_id, page.as_deref(), TIMEOUT) else {
            out.result = Err(Failure::local("The provider's request could not be built."));
            return out;
        };
        let answer = match Box::pin(exchange(call, &d, request, LIST_BODY)).await {
            Ok(a) => a,
            Err(f) => {
                out.result = Err(f);
                return out;
            }
        };
        out.status = Some(answer.status);
        if out.upstream_id.is_none() {
            out.upstream_id.clone_from(&answer.upstream_id);
        }
        if !(200..300).contains(&answer.status) {
            out.result = Err(rejected(&answer, p));
            return out;
        }
        match antirion_providers::listing::parse(listing, &answer.body) {
            Err(e) => {
                out.result = Err(Failure {
                    status: Some(answer.status),
                    message: format!("The provider answered, but {e}."),
                });
                return out;
            }
            Ok(parsed) => {
                for m in parsed.models {
                    if !models.iter().any(|x| x.id == m.id) {
                        models.push(m);
                    }
                }
                match parsed.next {
                    Some(next) if Some(&next) != page.as_ref() => page = Some(next),
                    _ => break,
                }
            }
        }
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    out.result = Ok(models);
    out
}

/// What a minimal call sends: a models list where the type has one, a token count where it is free,
/// else a one-token completion; and whether it is a completion (recorded in the ledger).
fn probe(d: &DeploymentRt) -> (UpstreamOp<'static>, Option<Value>, bool) {
    let ping = "ping";
    let one_token =
        json!({"model": d.model, "max_tokens": 1, "messages": [{"role": "user", "content": ping}]});
    match d.provider.kind {
        k if k.openai_family() && k != ProviderType::AzureOpenAi => (
            UpstreamOp::OpenAi {
                class: "models",
                path: "/models",
            },
            None,
            false,
        ),
        k if k.anthropic_api() => (UpstreamOp::Messages, Some(one_token), true),
        ProviderType::Vertex if d.publisher == Publisher::Google => (
            UpstreamOp::GeminiCountTokens,
            Some(json!({"contents": [{"role": "user", "parts": [{"text": ping}]}]})),
            false,
        ),
        ProviderType::Vertex => (
            UpstreamOp::Messages,
            Some(json!({"max_tokens": 1, "messages": [{"role": "user", "content": ping}]})),
            true,
        ),
        ProviderType::Gemini => (
            UpstreamOp::GeminiGenerate,
            Some(json!({"contents": [{"role": "user", "parts": [{"text": ping}]}],
                        "generationConfig": {"maxOutputTokens": 1}})),
            true,
        ),
        ProviderType::Bedrock => (
            UpstreamOp::Converse,
            Some(
                json!({"messages": [{"role": "user", "content": [{"text": ping}]}],
                        "inferenceConfig": {"maxTokens": 1}}),
            ),
            true,
        ),
        ProviderType::Cohere => (UpstreamOp::CohereChat, Some(one_token), true),
        _ => (
            UpstreamOp::OpenAi {
                class: "chat",
                path: "/chat/completions",
            },
            Some(one_token),
            true,
        ),
    }
}

/// The tokens a one-token completion reports, in any provider's shape.
fn usage_of(body: &[u8]) -> Usage {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return Usage::default();
    };
    let n = |paths: &[&str]| {
        paths
            .iter()
            .find_map(|p| v.pointer(p).and_then(Value::as_u64))
            .unwrap_or(0)
    };
    Usage::estimated(
        n(&[
            "/usage/prompt_tokens",
            "/usage/input_tokens",
            "/usage/inputTokens",
            "/usage/meta/billed_units/input_tokens",
            "/usageMetadata/promptTokenCount",
        ]),
        n(&[
            "/usage/completion_tokens",
            "/usage/output_tokens",
            "/usage/outputTokens",
            "/usage/meta/billed_units/output_tokens",
            "/usageMetadata/candidatesTokenCount",
        ]),
    )
}

/// Records a completion test as a `test` ledger line that no budget is charged with.
fn record_spend(call: &Call, d: &DeploymentRt, usage: Usage, ok: bool) {
    let now = call.rt.now();
    let price = call
        .snap
        .pricing
        .price(d.provider.kind.as_str(), &d.base_model, &d.name);
    let units = Units {
        requests: u64::from(ok),
        ..Units::default()
    };
    let (cost, billed, cost_status, version) = match price {
        Some(p) => match antirion_limits::cost::cost(&p, &usage, &units, None, Dec::ONE) {
            Ok(c) => (Some(c.total), Some(c.billed), c.status, Some(p.version())),
            Err(_) => (None, None, CostStatus::Estimated, Some(p.version())),
        },
        None => (Some(Nusd::ZERO), Some(Nusd::ZERO), CostStatus::Unpriced, None),
    };
    call.rt.record_spend(vec![LedgerLine {
        request_id: call.request_id.as_str().to_owned(),
        line: 0,
        purpose: Purpose::Test,
        shadow: false,
        status: if ok {
            LineStatus::Success
        } else {
            LineStatus::Failed
        },
        admitted_at: now,
        settled_at: now,
        organization: String::new(),
        team: String::new(),
        project: String::new(),
        key: String::new(),
        user: Some(call.principal.id.clone()),
        end_user: None,
        tags: Vec::new(),
        model: None,
        route: None,
        provider: Some(d.provider.name.clone()),
        deployment: (!d.name.is_empty()).then(|| d.name.clone()),
        upstream_model: Some(d.model.clone()),
        call: CallTarget::default(),
        endpoint: "provider_test".to_owned(),
        stream: false,
        usage,
        usage_raw: None,
        usage_source: UsageSource::Upstream,
        cost,
        billed,
        cost_status,
        price_version: version,
        service_tier: None,
        avoided: None,
        reserved: Reserved {
            input: 0,
            output: 0,
            usd: None,
            out_source: OutSource::Request,
        },
        budgets: Vec::new(),
    }]);
}

/// A test's outcome.
struct Tested {
    ok: bool,
    method: TestMethod,
    status: Option<u16>,
    latency_ms: f64,
    error: Option<String>,
    models: Option<Vec<Listed>>,
    upstream_id: Option<String>,
    at: UtcMicros,
}

/// A minimal call on `d`; a models list there is read as one.
async fn minimal_call(
    call: &Call,
    d: &DeploymentRt,
    key: Option<&str>,
) -> (Result<Answer, Failure>, bool, bool) {
    let (op, body, completion) = probe(d);
    let listing = body.is_none();
    let request_id = RequestId::generate();
    let prepared = d.build_request(UpstreamCall {
        op,
        stream: false,
        body: Payload::from(bytes::Bytes::from(
            body.as_ref().map(Value::to_string).unwrap_or_default(),
        )),
        content_type: Some(HeaderValue::from_static("application/json")),
        request_id: &request_id,
        response_timeout: TIMEOUT,
        connect_timeout: None,
        anthropic: None,
        key,
        byok: None,
    });
    let Ok(prepared) = prepared else {
        return (
            Err(Failure::local("The provider's request could not be built.")),
            listing,
            completion,
        );
    };
    let mut request = prepared.request;
    request.resend_when_closed = false;
    if listing {
        request.method = Method::GET;
        request.body = Payload::from(bytes::Bytes::new());
        request.headers.remove(http::header::CONTENT_TYPE);
    }
    let answer = Box::pin(exchange(call, d, request, CALL_BODY)).await;
    (answer, listing, completion)
}

/// The deployment a minimal call without one uses: the provider's first deployment, else one made
/// for a model every account of the type can call.
fn probe_deployment(call: &Call, p: &Arc<ProviderRt>, saved: bool) -> DeploymentRt {
    if saved {
        let mut own: Vec<&Arc<DeploymentRt>> = call
            .snap
            .providers
            .deployments
            .values()
            .filter(|d| d.provider.name == p.name)
            .collect();
        own.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(d) = own.first() {
            let mut d = d.as_ref().clone();
            d.provider = Arc::clone(p);
            return d;
        }
    }
    let model = match p.kind {
        ProviderType::Bedrock => BEDROCK_PROBE_MODEL,
        _ => VERTEX_PROBE_MODEL,
    };
    DeploymentRt::direct(p, "", model, None)
}

/// Tests a provider's credential: with `deployment`'s own call when one is named, else by listing
/// the models, else with a minimal call.
async fn run(
    call: &Call,
    p: &Arc<ProviderRt>,
    deployment: Option<&DeploymentRt>,
    key: Option<&str>,
    saved: bool,
) -> Tested {
    let started = Instant::now();
    let mut t = Tested {
        ok: false,
        method: TestMethod::MinimalCall,
        status: None,
        latency_ms: 0.0,
        error: None,
        models: None,
        upstream_id: None,
        at: call.rt.now(),
    };
    let listing = p.listing();
    match (deployment, listing) {
        (None, Some(listing)) => {
            t.method = TestMethod::ListModels;
            let out = Box::pin(list(call, p, listing, key)).await;
            t.status = out.status;
            t.upstream_id = out.upstream_id;
            match out.result {
                Ok(models) => {
                    t.ok = true;
                    t.models = Some(models);
                }
                Err(f) => {
                    t.ok = f.status.is_some_and(|s| (200..300).contains(&s));
                    t.error = (!t.ok).then_some(f.message);
                }
            }
        }
        (d, _) => {
            let owned;
            let d = match d {
                Some(d) => d,
                None => {
                    owned = probe_deployment(call, p, saved);
                    &owned
                }
            };
            let (answer, listed, completion) = Box::pin(minimal_call(call, d, key)).await;
            if listed {
                t.method = TestMethod::ListModels;
            }
            match answer {
                Err(f) => t.error = Some(f.message),
                Ok(a) => {
                    t.status = Some(a.status);
                    t.upstream_id.clone_from(&a.upstream_id);
                    t.ok = (200..300).contains(&a.status);
                    if !t.ok {
                        t.error = Some(rejected(&a, p).message);
                    }
                    if listed
                        && t.ok
                        && let Some(listing) = listing
                    {
                        t.models = antirion_providers::listing::parse(listing, &a.body)
                            .ok()
                            .map(|page| page.models);
                    }
                    if completion {
                        record_spend(call, d, usage_of(&a.body), t.ok);
                    }
                }
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    {
        t.latency_ms = (started.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0;
    }
    t
}

/// A model as the Admin API shows it: what the provider listed, filled in with what the catalog
/// knows (name, limits, modalities, price).
fn model_json(p: &ProviderRt, id: &str, listed: Option<&Listed>) -> Value {
    let kind = p.kind;
    let known = p.catalog.model_info(kind.as_str(), id, kind.generic());
    model_value(id, listed, known)
}

fn model_value(id: &str, listed: Option<&Listed>, known: Option<ModelInfo>) -> Value {
    let mut info = known.unwrap_or_default();
    info.id = id.to_owned();
    if let Some(l) = listed {
        info.name = l.name.clone().or(info.name);
        info.context_window = l.context_window.or(info.context_window);
        info.max_output_tokens = l.max_output_tokens.or(info.max_output_tokens);
        if let Some((input, output)) = &l.modalities {
            info.modalities = Some(antirion_catalog::modelsdev::Modalities {
                input: input.clone(),
                output: output.clone(),
            });
        }
    }
    let mut v = info.to_value();
    if let Some(o) = v.as_object_mut() {
        o.insert("listed_by_provider".into(), Value::Bool(listed.is_some()));
    }
    v
}

fn result_json(p: &ProviderRt, t: &Tested) -> Value {
    let models = t.models.as_ref().map(|ms| {
        ms.iter()
            .map(|m| model_json(p, &m.id, Some(m)))
            .collect::<Vec<_>>()
    });
    json!({
        "ok": t.ok,
        "method": t.method.as_str(),
        "status_code": t.status,
        "latency_ms": t.latency_ms,
        "error": t.error,
        "models": models,
        "tested_at": t.at.to_rfc3339_seconds(),
        "upstream_request_id": t.upstream_id,
    })
}

/// A fingerprint of a saved provider's configuration, which a kept model list must match.
fn fingerprint(call: &Call, name: &str) -> u64 {
    let mut h = DefaultHasher::new();
    call.snap
        .config
        .doc
        .pointer(&format!(
            "/providers/{}",
            name.replace('~', "~0").replace('/', "~1")
        ))
        .map(Value::to_string)
        .hash(&mut h);
    h.finish()
}

/// The name of the key a test used: the named one, else the one the provider sends first.
fn key_name(p: &ProviderRt, key: Option<&str>) -> Option<String> {
    if !matches!(p.auth, UpstreamAuth::Header { .. }) {
        return None;
    }
    key.map(str::to_owned).or_else(|| {
        p.keys
            .iter()
            .find(|k| k.weight > 0)
            .or_else(|| p.keys.first())
            .map(|k| k.name.clone())
    })
}

/// `POST /providers/{name}:test`.
pub(crate) async fn test(call: &Call, name: &str) -> Result<Response, AdminError> {
    if call.method != Method::POST {
        return Err(AdminError::method_not_allowed("POST"));
    }
    call.allow_params(&[])?;
    call.require("providers:write", &[Scope::Gateway])?;
    let p = call
        .snap
        .providers
        .providers
        .get(name)
        .ok_or_else(AdminError::not_found)?;
    let body = members(call, &["deployment", "credential"])?;
    let deployment = match body.get("deployment").and_then(Value::as_str) {
        None => None,
        Some(n) => Some(
            call.snap
                .providers
                .deployments
                .get(n)
                .filter(|d| d.provider.name == p.name)
                .ok_or_else(|| {
                    invalid(
                        "deployment",
                        DetailCode::UnresolvedReference,
                        "the deployment is not one of the provider's",
                    )
                })?,
        ),
    };
    let key = body.get("credential").and_then(Value::as_str);
    if let Some(k) = key
        && !p.keys.iter().any(|x| x.name == k)
    {
        return Err(invalid(
            "credential",
            DetailCode::UnresolvedReference,
            "the provider has no credential of that name",
        ));
    }
    let t = Box::pin(run(call, p, deployment.map(AsRef::as_ref), key, true)).await;
    call.rt.state.provider_checks.record_test(
        name,
        TestRecord {
            key: key_name(p, key),
            ok: t.ok,
            method: t.method,
            at: t.at,
            error: t.error.clone(),
        },
    );
    if deployment.is_none()
        && key.is_none()
        && let Some(models) = &t.models
    {
        call.rt.state.provider_checks.store_list(
            name,
            Fetched {
                fingerprint: fingerprint(call, name),
                at: t.at,
                models: Arc::new(models.clone()),
            },
        );
    }
    call.audit_with(
        "POST providers:test",
        &format!("providers/{name}"),
        if t.ok { Outcome::Success } else { Outcome::Failed },
        StatusCode::OK,
        json!({"deployment": deployment.map(|d| d.name.clone()), "credential": key, "ok": t.ok,
               "method": t.method.as_str()}),
    )
    .await;
    Ok(call.json(StatusCode::OK, &result_json(p, &t)))
}

/// Whether a spec has a secret reference, and whether it has a `[REDACTED]` placeholder.
pub(crate) fn secret_forms(v: &Value) -> (bool, bool) {
    match v {
        Value::String(s) => (
            antirion_config::secrets::has_reference(s),
            s == antirion_config::secrets::REDACTED,
        ),
        Value::Array(items) => items
            .iter()
            .map(secret_forms)
            .fold((false, false), |a, b| (a.0 || b.0, a.1 || b.1)),
        Value::Object(m) => m
            .values()
            .map(secret_forms)
            .fold((false, false), |a, b| (a.0 || b.0, a.1 || b.1)),
        _ => (false, false),
    }
}

/// What an unsaved test may resolve: under change control, literal secrets only, since nothing
/// is approved; a `[REDACTED]` placeholder stands for the saved provider's secret, which is sent
/// only where the saved provider sends it (the same type and base URL).
fn check_secrets(call: &Call, name: &str, spec: &Value, candidate: &Value) -> Result<(), AdminError> {
    let (references, redacted) = secret_forms(spec);
    if (references || redacted) && crate::change_control::governed(&call.snap.config.doc, candidate) {
        return Err(invalid(
            "spec",
            DetailCode::Conflict,
            "providers are under change control: test a provider before saving it with literal secrets only",
        ));
    }
    if redacted {
        let saved = call.snap.config.doc.get("providers").and_then(|p| p.get(name));
        let same = |field: &str| saved.and_then(|s| s.get(field)) == spec.get(field);
        if saved.is_none() || !same("type") || !same("base_url") {
            return Err(invalid(
                "spec",
                DetailCode::Conflict,
                "a [REDACTED] value is the saved provider's secret, sent only to its saved type and base URL",
            ));
        }
    }
    Ok(())
}

/// `POST /providers:test`: tests a provider that is not saved yet; it stores nothing.
pub(crate) async fn test_unsaved(call: &Call) -> Result<Response, AdminError> {
    if call.method != Method::POST {
        return Err(AdminError::method_not_allowed("POST"));
    }
    call.allow_params(&[])?;
    call.require("providers:write", &[Scope::Gateway])?;
    let body = members(call, &["name", "spec", "metadata"])?;
    let name = match body.get("name").and_then(Value::as_str) {
        Some(n) if !n.is_empty() => n.to_owned(),
        _ => return Err(invalid("name", DetailCode::Required, "name is required")),
    };
    let spec = match body.get("spec") {
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
    let current = Arc::clone(&call.snap.config);
    let mut candidate = current.doc.clone();
    let Some(root) = candidate.as_object_mut() else {
        return Err(AdminError::internal());
    };
    let providers = root
        .entry("providers")
        .or_insert_with(|| Value::Object(Map::new()));
    if !providers.is_object() {
        *providers = Value::Object(Map::new());
    }
    if let Some(map) = providers.as_object_mut() {
        map.insert(name.clone(), spec.clone());
    }
    check_secrets(call, &name, &spec, &candidate)?;
    let p = Box::pin(call.rt.compile_provider(candidate, &current, &name))
        .await
        .map_err(AdminError::validation)?;
    let t = Box::pin(run(call, &p, None, None, false)).await;
    call.audit_with(
        "POST providers:test",
        &format!("providers/{name}"),
        if t.ok { Outcome::Success } else { Outcome::Failed },
        StatusCode::OK,
        json!({"saved": false, "type": p.kind.as_str(), "ok": t.ok, "method": t.method.as_str()}),
    )
    .await;
    Ok(call.json(StatusCode::OK, &result_json(&p, &t)))
}

/// `GET /providers/{name}/models`.
pub(crate) async fn models(call: &Call, name: &str) -> Result<Response, AdminError> {
    if call.method != Method::GET {
        return Err(AdminError::method_not_allowed("GET"));
    }
    call.allow_params(&["refresh", "q", "id"])?;
    call.require("providers:read", &[Scope::Gateway])?;
    let p = call
        .snap
        .providers
        .providers
        .get(name)
        .ok_or_else(AdminError::not_found)?;
    let refresh = call.bool_param("refresh", false)?;
    let now = call.rt.now();
    let checks = &call.rt.state.provider_checks;
    let mut error = None;
    let mut listed: Option<(Arc<Vec<Listed>>, UtcMicros)> = None;
    if let Some(listing) = p.listing() {
        let print = fingerprint(call, name);
        let kept = (!refresh)
            .then(|| checks.list(name, print, now, LIST_MAX_AGE))
            .flatten();
        listed = match kept {
            Some(f) => Some((f.models, f.at)),
            None => match Box::pin(list(call, p, listing, None)).await.result {
                Ok(models) => {
                    let models = Arc::new(models);
                    checks.store_list(
                        name,
                        Fetched {
                            fingerprint: print,
                            at: now,
                            models: Arc::clone(&models),
                        },
                    );
                    Some((models, now))
                }
                Err(f) => {
                    error = Some(f.message);
                    None
                }
            },
        };
    }
    let kind = p.kind;
    let (mut items, source, fetched_at): (Vec<Value>, &str, Option<String>) = match &listed {
        Some((models, at)) => (
            models.iter().map(|m| model_json(p, &m.id, Some(m))).collect(),
            "provider",
            Some(at.to_rfc3339_seconds()),
        ),
        None => (
            p.catalog
                .models_of_type(kind.as_str())
                .into_iter()
                .map(|info| model_value(&info.id.clone(), None, Some(info)))
                .collect(),
            "catalog",
            None,
        ),
    };
    if let Some(id) = call.param("id") {
        items.retain(|m| m.get("id").and_then(Value::as_str) == Some(id));
        if items.is_empty()
            && let Some(info) = p.catalog.model_info(kind.as_str(), id, kind.generic())
        {
            items.push(model_value(id, None, Some(info)));
        }
    }
    if let Some(q) = call.param("q").map(str::to_lowercase).filter(|q| !q.is_empty()) {
        items.retain(|m| {
            ["id", "name"].iter().any(|k| {
                m.get(*k)
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.to_lowercase().contains(&q))
            })
        });
    }
    Ok(call.json(
        StatusCode::OK,
        &json!({"items": items, "source": source, "fetched_at": fetched_at, "error": error}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_forms_are_found_anywhere_in_a_spec() {
        assert_eq!(
            secret_forms(&json!({"auth": {"api_key": "${env:KEY}"}})),
            (true, false)
        );
        assert_eq!(
            secret_forms(&json!({"auth": {"api_keys": [{"name": "a", "key": "[REDACTED]"}]}})),
            (false, true)
        );
        assert_eq!(
            secret_forms(&json!({"auth": {"api_key": "sk-literal"}})),
            (false, false)
        );
    }

    #[test]
    fn a_listed_model_is_filled_in_from_the_catalog() {
        let listed = Listed {
            id: "m".into(),
            name: None,
            context_window: Some(10),
            max_output_tokens: None,
            modalities: None,
        };
        let known = ModelInfo {
            id: "m".into(),
            name: Some("Model".into()),
            context_window: Some(99),
            max_output_tokens: Some(5),
            modalities: None,
            price: Some(antirion_catalog::modelsdev::ModelPrice {
                input: "1".into(),
                output: "2".into(),
                cache_read: None,
                cache_write: None,
            }),
            release_date: None,
        };
        let v = model_value("m", Some(&listed), Some(known));
        assert_eq!(v["name"], json!("Model"));
        assert_eq!(v["context_window"], json!(10), "the provider's own value first");
        assert_eq!(v["max_output_tokens"], json!(5));
        assert_eq!(v["price"]["input"], json!("1"));
        assert_eq!(v["price_source"], json!("catalog"));
        assert_eq!(v["listed_by_provider"], json!(true));
        let bare = model_value("x", None, None);
        assert_eq!(bare["price"], Value::Null);
        assert_eq!(bare["price_source"], Value::Null);
        assert_eq!(bare["listed_by_provider"], json!(false));
    }
}
