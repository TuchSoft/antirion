//! Calls the gateway makes to its own Models on a request's behalf: embeddings for embedding
//! classifiers and the semantic cache, and short chat questions for LLM classifiers and judges. A
//! call goes to the first usable deployment of the Model that speaks the needed API (OpenAI-family,
//! or Anthropic Messages for chat), then the next on failure; it is never refused by the caller's
//! limits, and its usage is settled as an auxiliary line of the request: `router` (tagged
//! `antirion.routing`), `cache_embedding` or `mcp_tool_index`.

use std::sync::Arc;
use std::time::Duration;

use antirion_limits::cost::CostStatus;
use antirion_limits::ledger::Purpose;
use antirion_limits::money::Nusd;
use antirion_providers::{DeploymentRt, Native, UpstreamCall, UpstreamOp};
use antirion_resilience::{KeyMode, KeyPick};
use antirion_runtime::{Runtime, Snapshot};
use bytes::Bytes;
use http::HeaderValue;
use serde_json::{Value, json};

use crate::account::{ShadowCall, Slot, auxiliary_line, charge_line};
use crate::ctx::Ctx;
use crate::outcome::collect;
use crate::reqlog::Draft;

/// The largest answer read.
const MAX_ANSWER: u64 = 4 << 20;
/// Ledger line numbers of MCP tool-index embedding calls start here, beyond attempts.
const FIRST_TOOL_INDEX_LINE: u32 = 600;
/// Ledger line numbers of routing calls start here, beyond attempts and before mirrors.
const FIRST_LINE: u32 = 500;
/// Ledger line numbers of semantic-cache embedding calls start here, beyond attempts.
const FIRST_CACHE_LINE: u32 = 400;

/// What is asked.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Question<'a> {
    /// One vector per text, in order.
    Embed(&'a [String]),
    Chat {
        system: &'a str,
        user: &'a str,
        max_tokens: u32,
    },
}

/// What came back.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Answer {
    Vectors(Vec<Vec<f32>>),
    Text(String),
}

fn fits(d: &DeploymentRt, q: Question<'_>) -> Option<bool> {
    if !d.usable() {
        return None;
    }
    match q {
        Question::Embed(_) => d.speaks(Native::OpenAi).then_some(false),
        Question::Chat { .. } if d.speaks(Native::OpenAi) => Some(false),
        Question::Chat { .. } => d.speaks(Native::AnthropicMessages).then_some(true),
    }
}

fn body(d: &DeploymentRt, q: Question<'_>, anthropic: bool) -> Value {
    match q {
        Question::Embed(texts) => json!({"model": d.model, "input": texts}),
        Question::Chat {
            system,
            user,
            max_tokens,
        } if anthropic => json!({
            "model": d.model, "system": system, "max_tokens": max_tokens, "temperature": 0,
            "messages": [{"role": "user", "content": user}],
        }),
        Question::Chat {
            system,
            user,
            max_tokens,
        } => json!({
            "model": d.model, "max_tokens": max_tokens, "temperature": 0,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        }),
    }
}

fn read(v: &Value, q: Question<'_>, anthropic: bool) -> Option<Answer> {
    match q {
        Question::Embed(texts) => {
            let mut rows: Vec<(u64, Vec<f32>)> = v
                .get("data")?
                .as_array()?
                .iter()
                .map(|e| {
                    let index = e.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let vector = e
                        .get("embedding")?
                        .as_array()?
                        .iter()
                        .map(|x| x.as_f64().map(|x| x as f32))
                        .collect::<Option<Vec<f32>>>()?;
                    Some((index, vector))
                })
                .collect::<Option<Vec<_>>>()?;
            rows.sort_by_key(|(i, _)| *i);
            (rows.len() == texts.len()).then(|| Answer::Vectors(rows.into_iter().map(|(_, v)| v).collect()))
        }
        Question::Chat { .. } if anthropic => {
            let text: String = v
                .get("content")?
                .as_array()?
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            Some(Answer::Text(text))
        }
        Question::Chat { .. } => v
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|s| Answer::Text(s.to_owned())),
    }
}

/// Who asks, which decides the ledger line's purpose and number.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Asker {
    pub(crate) purpose: Purpose,
    /// The call's number among the request's calls of this purpose.
    pub(crate) line: u32,
}

impl Asker {
    pub(crate) const fn router(line: u32) -> Asker {
        Asker {
            purpose: Purpose::Router,
            line,
        }
    }

    pub(crate) const fn tool_index(line: u32) -> Asker {
        Asker {
            purpose: Purpose::McpToolIndex,
            line,
        }
    }

    fn line(self) -> u32 {
        let base = match self.purpose {
            Purpose::Router => FIRST_LINE,
            Purpose::McpToolIndex => FIRST_TOOL_INDEX_LINE,
            _ => FIRST_CACHE_LINE,
        };
        base.saturating_add(self.line)
    }
}

/// The request an internal call is made on behalf of: what settlement needs, without the rest of
/// its `Ctx`.
#[derive(Clone, Copy)]
struct OnBehalf<'a> {
    rt: &'a Arc<Runtime>,
    snap: &'a Arc<Snapshot>,
    request_id: &'a antirion_core::RequestId,
    log: &'a Draft,
    account: &'a Slot,
}

/// Asks Model `model`; each call is bounded by `timeout`.
pub(crate) async fn ask(
    cx: &Ctx,
    model: &str,
    q: Question<'_>,
    timeout: Duration,
    asker: Asker,
) -> Result<Answer, String> {
    ask_settled(
        &cx.rt,
        &cx.snap,
        &cx.request_id,
        &cx.log,
        &cx.account,
        model,
        q,
        timeout,
        asker,
    )
    .await
}

/// Asks Model `model` on a request's behalf outside its `Ctx`, settling the call like `ask`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn ask_settled(
    rt: &Arc<Runtime>,
    snap: &Arc<Snapshot>,
    request_id: &antirion_core::RequestId,
    log: &Draft,
    account: &Slot,
    model: &str,
    q: Question<'_>,
    timeout: Duration,
    asker: Asker,
) -> Result<Answer, String> {
    let place = OnBehalf {
        rt,
        snap,
        request_id,
        log,
        account,
    };
    let resolved = place
        .snap
        .routing
        .resolve(model)
        .or_else(|| {
            snap.routing.models.get(model).map(|m| antirion_router::Resolved {
                model: std::sync::Arc::clone(m),
                alias_params: None,
                captures: None,
                via: "exact",
            })
        })
        .ok_or_else(|| format!("model {model} does not exist"))?;
    let candidates =
        crate::authorize::candidates(&place.snap.providers, &place.snap.routing, &resolved.model);
    let mut last = format!("model {model} has no deployment for this call");
    for d in candidates {
        let Some(anthropic) = fits(d, q) else { continue };
        match one(place, d, q, anthropic, timeout, asker).await {
            Ok(a) => return Ok(a),
            Err(e) => last = e,
        }
    }
    Err(last)
}

async fn one(
    place: OnBehalf<'_>,
    d: &DeploymentRt,
    q: Question<'_>,
    anthropic: bool,
    timeout: Duration,
    asker: Asker,
) -> Result<Answer, String> {
    let op = match q {
        Question::Embed(_) => UpstreamOp::OpenAi {
            class: "embed",
            path: "/embeddings",
        },
        Question::Chat { .. } if anthropic => UpstreamOp::Messages,
        Question::Chat { .. } => UpstreamOp::OpenAi {
            class: "generate",
            path: "/chat/completions",
        },
    };
    let key = match crate::route::pick_key(place.rt, place.snap, d, &[], None, KeyMode::Untried) {
        KeyPick::Key(k) => Some(k),
        KeyPick::Keyless => None,
        KeyPick::None => return Err("no usable key".to_owned()),
    };
    let payload = serde_json::to_vec(&body(d, q, anthropic)).unwrap_or_default();
    let prepared = d
        .build_request(UpstreamCall {
            op,
            stream: false,
            body: Bytes::from(payload).into(),
            content_type: Some(HeaderValue::from_static("application/json")),
            request_id: place.request_id,
            response_timeout: timeout,
            connect_timeout: Some(timeout),
            anthropic: None,
            key: key.as_deref(),
            byok: None,
        })
        .map_err(|_| "the request could not be built".to_owned())?;
    let mut request = prepared.request;
    request.resend_when_closed = false;
    let tls = place
        .snap
        .security
        .egress_tls
        .clone()
        .unwrap_or_else(|| d.provider.tls.clone());
    d.authorize(&mut request, &place.rt.creds, &tls)
        .await
        .map_err(|_| "no credential".to_owned())?;
    let resp = place.rt.egress.send(request).await.map_err(|e| e.to_string())?;
    let status = resp.status.as_u16();
    let raw = collect(resp.body, MAX_ANSWER)
        .await
        .map_err(|_| "the answer could not be read".to_owned())?;
    let v: Option<Value> = serde_json::from_slice(&raw).ok();
    settle(
        place.rt,
        place.snap,
        place.log,
        place.account,
        d,
        status,
        v.as_ref(),
        asker,
    );
    if !(200..300).contains(&status) {
        return Err(format!("status {status}"));
    }
    v.as_ref()
        .and_then(|v| read(v, q, anthropic))
        .ok_or_else(|| "the answer is malformed".to_owned())
}

/// Asks Model `model` on behalf of a guardrail: the first usable deployment that fits, then the
/// next on failure, each bounded by `timeout`. Its usage is not settled on the request.
pub(crate) async fn ask_for_guardrail(
    rt: &antirion_runtime::Runtime,
    snap: &antirion_runtime::Snapshot,
    request_id: &antirion_core::RequestId,
    model: &str,
    q: Question<'_>,
    timeout: Duration,
) -> Result<Answer, String> {
    let resolved = snap
        .routing
        .resolve(model)
        .ok_or_else(|| format!("model {model} does not exist"))?;
    let candidates = crate::authorize::candidates(&snap.providers, &snap.routing, &resolved.model);
    let mut last = format!("model {model} has no deployment for this call");
    for d in candidates {
        let Some(anthropic) = fits(d, q) else { continue };
        let op = match q {
            Question::Embed(_) => UpstreamOp::OpenAi {
                class: "embed",
                path: "/embeddings",
            },
            Question::Chat { .. } if anthropic => UpstreamOp::Messages,
            Question::Chat { .. } => UpstreamOp::OpenAi {
                class: "generate",
                path: "/chat/completions",
            },
        };
        let payload = serde_json::to_vec(&body(d, q, anthropic)).unwrap_or_default();
        let Ok(prepared) = d.build_request(UpstreamCall {
            op,
            stream: false,
            body: Bytes::from(payload).into(),
            content_type: Some(HeaderValue::from_static("application/json")),
            request_id,
            response_timeout: timeout,
            connect_timeout: Some(timeout),
            anthropic: None,
            key: None,
            byok: None,
        }) else {
            last = "the request could not be built".to_owned();
            continue;
        };
        let mut request = prepared.request;
        request.resend_when_closed = false;
        let tls = snap
            .security
            .egress_tls
            .clone()
            .unwrap_or_else(|| d.provider.tls.clone());
        if d.authorize(&mut request, &rt.creds, &tls).await.is_err() {
            last = "no credential".to_owned();
            continue;
        }
        let resp = match rt.egress.send(request).await {
            Ok(r) => r,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        let status = resp.status.as_u16();
        let raw = collect(resp.body, MAX_ANSWER)
            .await
            .map_err(|_| "the answer could not be read".to_owned())?;
        if !(200..300).contains(&status) {
            return Err(format!("the model answered with status {status}"));
        }
        let v: Value = serde_json::from_slice(&raw).map_err(|_| "the answer is not JSON".to_owned())?;
        return read(&v, q, anthropic).ok_or_else(|| "the answer is malformed".to_owned());
    }
    Err(last)
}

/// Settles the call's usage as an auxiliary line of the request: priced at once, its cost added to
/// the request's record, then charged to the budgets off the request's path.
#[allow(clippy::too_many_arguments)]
fn settle(
    rt: &Arc<Runtime>,
    snap: &Arc<Snapshot>,
    log: &Draft,
    account: &Slot,
    d: &DeploymentRt,
    status: u16,
    v: Option<&Value>,
    asker: Asker,
) {
    let Some(facts) = account.shadow_facts() else {
        return;
    };
    let usage = v.map(crate::apis::usage_of).filter(|u| !u.is_empty());
    let raw = v.and_then(|v| v.get("usage")).cloned();
    let Some(d) = snap.providers.deployments.get(&d.name) else {
        return;
    };
    let call = ShadowCall {
        deployment: d,
        line: asker.line(),
        answered: (200..300).contains(&status),
        usage,
        raw,
        e_in: 0,
        purpose: asker.purpose,
    };
    let line = auxiliary_line(snap, &facts, &call, rt.now());
    let (cost, cost_status) = (line.billed.or(line.cost).unwrap_or(Nusd::ZERO), line.cost_status);
    log.update(|r| {
        let (sum, worst) = r.auxiliary.unwrap_or((Nusd::ZERO, CostStatus::Exact));
        r.auxiliary = Some((
            Nusd(sum.0.saturating_add(cost.0)),
            less_certain(worst, cost_status),
        ));
    });
    let (rt, snap) = (Arc::clone(rt), Arc::clone(snap));
    let shutdown = rt.shutdown.clone();
    let tracked = Arc::clone(&rt);
    tracked.tasks.spawn(async move {
        tokio::select! {
            () = shutdown.cancelled() => {}
            _ = charge_line(&rt, &snap, &facts, asker.purpose, line) => {}
        }
    });
}

/// The less certain of two cost statuses.
pub(crate) fn less_certain(a: CostStatus, b: CostStatus) -> CostStatus {
    let rank = |s: CostStatus| match s {
        CostStatus::Exact => 0,
        CostStatus::Approximate => 1,
        CostStatus::Estimated => 2,
        CostStatus::Unpriced => 3,
    };
    if rank(b) > rank(a) { b } else { a }
}
