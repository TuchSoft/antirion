//! What the request log, the ledger and the budget events report for a request match what the
//! request did: the usage an embeddings answer reported, the calls the gateway made on the
//! request's behalf, a budget refusal that is not an exhaustion, and a refusal at authentication.

use std::net::SocketAddr;
use std::time::Duration;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "request-accounting-bootstrap-token-0123456789abcdef";

/// One HTTP/1.1 exchange on a fresh connection: (status, lowercase headers, body).
async fn exchange(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, Vec<(String, String)>, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\ncontent-type: application/json\r\n\
         content-length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));
    let body = if chunked { dechunk(rest) } else { rest.to_owned() };
    (status, headers, body)
}

fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let n = usize::from_str_radix(size.split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&rest[..n]);
        s = &rest[n + 2..];
    }
    out
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// A USD amount of a record, a ledger line or a report, in nano-dollars.
fn nusd(v: &Value) -> i64 {
    let s = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    let (whole, frac) = s.split_once('.').unwrap_or((&s, ""));
    let frac = format!("{frac:0<9}");
    whole.parse::<i64>().unwrap() * 1_000_000_000 + frac[..9].parse::<i64>().unwrap()
}

/// The upstream's answer to `path`: three-dimension vectors for embeddings (two tokens per input),
/// else a short chat completion (three tokens in, one out).
fn answer(path: &str, body: &Value) -> String {
    if path.ends_with("/embeddings") {
        let inputs = match &body["input"] {
            Value::Array(items) => items.len(),
            _ => 1,
        };
        let data: Vec<Value> = (0..inputs)
            .map(|i| json!({"object": "embedding", "index": i, "embedding": [0.25, -0.5, 0.75]}))
            .collect();
        let tokens = 2 * inputs;
        return json!({"object": "list", "data": data, "model": "up-embed",
            "usage": {"prompt_tokens": tokens, "total_tokens": tokens}})
        .to_string();
    }
    json!({"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "up-gpt",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}})
    .to_string()
}

/// An upstream answering embeddings and chat completions.
async fn upstream(listener: TcpListener) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        connections.spawn(async move {
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            let (head, body) = loop {
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= len {
                        break (head.to_owned(), body.to_owned());
                    }
                }
            };
            let path = head.split(' ').nth(1).unwrap_or_default();
            let text = answer(path, &serde_json::from_str(&body).unwrap_or(Value::Null));
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

struct Gateway {
    data: SocketAddr,
    admin: SocketAddr,
    shutdown: Box<dyn FnOnce() + Send>,
    tasks: JoinSet<()>,
    mock: JoinSet<()>,
    _dir: ScratchDir,
}

impl Gateway {
    /// A gateway whose providers answer from a mock upstream, with `extra` merged into the base
    /// document: deployments `chat` and `embed` priced at one dollar per million input tokens and
    /// two per million output tokens, Models `gpt` and `emb`, and content logging on.
    async fn start(name: &str, extra: Value) -> Gateway {
        let dir = ScratchDir::new(name);
        let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mock_addr = mock.local_addr().unwrap();
        let mut mock_task = JoinSet::new();
        mock_task.spawn(upstream(mock));
        let mut config = json!({
            "version": 1,
            "server": {"listen": "127.0.0.1:0"},
            "admin": {"listen": "127.0.0.1:0"},
            "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
            "auth": {"admin": {"bootstrap_token": TOKEN}},
            "providers": {
                "oa": {"type": "openai", "base_url": format!("http://{mock_addr}/v1"), "auth": {"api_key": "sk-up"}}
            },
            "deployments": {
                "chat": {"provider": "oa", "model": "up-gpt", "price": {"input": "1", "output": "2"}},
                "embed": {"provider": "oa", "model": "up-embed", "price": {"input": "1", "output": "2"}}
            },
            "models": {
                "gpt": {"target": {"deployment": "chat"}},
                "emb": {"target": {"deployment": "embed"}}
            },
            "policies": {"observability": {"content": {"enabled": true}}}
        });
        merge(&mut config, extra);
        let file = dir.path().join("gw.json");
        std::fs::write(&file, config.to_string()).unwrap();
        let node = start(ServeOptions {
            config: Some(file),
            test_mode: true,
            no_config: None,
        })
        .await
        .unwrap();
        let (data, admin) = (node.data_addr().unwrap(), node.admin_addr().unwrap());
        let token = node.rt.shutdown.clone();
        let shutdown: Box<dyn FnOnce() + Send> = Box::new(move || token.cancel());
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let _ = node.run().await;
        });
        Gateway {
            data,
            admin,
            shutdown,
            tasks,
            mock: mock_task,
            _dir: dir,
        }
    }

    async fn stop(mut self) {
        (self.shutdown)();
        while self.tasks.join_next().await.is_some() {}
        self.mock.abort_all();
    }

    /// An Admin API call with the bootstrap token: (status, body).
    async fn admin(&self, method: &str, path: &str, body: &str) -> (u16, Value) {
        let auth = format!("Bearer {TOKEN}");
        let (status, _, text) = exchange(
            self.admin,
            method,
            path,
            &[("authorization", auth.as_str())],
            body,
        )
        .await;
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// A new key with `spec`; its secret.
    async fn key(&self, name: &str, spec: Value) -> String {
        let (status, body) = self
            .admin(
                "POST",
                "/admin/v1/keys",
                &json!({"name": name, "spec": spec}).to_string(),
            )
            .await;
        assert_eq!(status, 201, "{body}");
        body["secret"].as_str().unwrap().to_owned()
    }

    /// A data-plane request: (status, headers, body).
    async fn send(
        &self,
        key: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
    ) -> (u16, Vec<(String, String)>, String) {
        let bearer = format!("Bearer {key}");
        let mut all = vec![("authorization", bearer.as_str())];
        all.extend_from_slice(headers);
        exchange(self.data, "POST", path, &all, &body.to_string()).await
    }

    /// The request-log record of `request_id`, once written.
    async fn record(&self, request_id: &str) -> Value {
        self.try_record(request_id, 100)
            .await
            .unwrap_or_else(|| panic!("no record for {request_id}"))
    }

    /// The request-log record of `request_id`, if one is written within `tries` polls.
    async fn try_record(&self, request_id: &str, tries: u32) -> Option<Value> {
        for _ in 0..tries {
            let (_, list) = self
                .admin("GET", &format!("/admin/v1/logs?request_id={request_id}"), "")
                .await;
            if let Some(id) = list["items"][0]["id"].as_str() {
                let (status, record) = self.admin("GET", &format!("/admin/v1/logs/{id}"), "").await;
                assert_eq!(status, 200, "{record}");
                return Some(record);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        None
    }
}

/// Merges `extra` into `base`, objects member by member.
fn merge(base: &mut Value, extra: Value) {
    match (base, extra) {
        (Value::Object(b), Value::Object(e)) => {
            for (k, v) in e {
                merge(b.entry(k).or_insert(Value::Null), v);
            }
        }
        (b, e) => *b = e,
    }
}

fn request_id(headers: &[(String, String)]) -> String {
    header(headers, "x-request-id").unwrap().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn embeddings_record_the_usage_and_cost_the_upstream_reported() {
    let gw = Gateway::start(
        "accounting-embeddings",
        json!({"models": {"cached-emb": {"target": {"deployment": "embed"},
            "policies": {"cache": {"enabled": true, "ttl": "10m"}}}}}),
    )
    .await;
    let key = gw.key("k", json!({})).await;

    for model in ["emb", "cached-emb"] {
        let body = json!({"model": model, "input": ["first text", "second text"]});
        let (status, headers, sent) = gw.send(&key, "/v1/embeddings", &[], &body).await;
        assert_eq!(status, 200, "{sent}");
        // The client receives the upstream's vectors.
        let sent: Value = serde_json::from_str(&sent).unwrap();
        assert_eq!(sent["data"][1]["embedding"], json!([0.25, -0.5, 0.75]), "{sent}");
        let record = gw.record(&request_id(&headers)).await;
        // Two inputs at two tokens each, as the upstream reported them, priced at $1 per million.
        assert_eq!(record["usage"]["input"], 4, "{model}: {record}");
        assert_eq!(record["cost"]["status"], "exact", "{model}: {record}");
        assert_eq!(nusd(&record["cost"]["total"]), 4_000, "{model}: {record}");
        assert_eq!(record["response_model"], "up-embed", "{model}: {record}");
        // The logged answer keeps the shape of the vectors, not their numbers.
        let logged = &record["content"]["response"];
        assert_eq!(logged["truncated"], false, "{record}");
        assert_eq!(
            logged["body"]["data"],
            json!({"vectors": 2, "dimensions": 3, "encoding_format": "float", "omitted": true}),
            "{model}: {record}"
        );
        assert_eq!(logged["body"]["usage"]["prompt_tokens"], 4, "{record}");
        assert!(logged["original_bytes"].as_u64().unwrap() > 0, "{record}");
    }

    // A hit of the cached Model replays the answer and its usage.
    let body = json!({"model": "cached-emb", "input": ["first text", "second text"]});
    let (status, headers, _) = gw.send(&key, "/v1/embeddings", &[], &body).await;
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "x-antirion-cache"), Some("hit"));
    let hit = gw.record(&request_id(&headers)).await;
    assert_eq!(hit["usage"]["input"], 4, "{hit}");
    assert_eq!(hit["content"]["response"]["body"]["data"]["vectors"], 2, "{hit}");

    gw.stop().await;
}

/// The ledger lines of the request whose record is `log_id`, once `n` are written.
async fn ledger(gw: &Gateway, log_id: &str, n: usize) -> Vec<Value> {
    let mut last = Value::Null;
    for _ in 0..100 {
        let (status, body) = gw
            .admin("GET", &format!("/admin/v1/reports/spend/requests/{log_id}"), "")
            .await;
        if status == 200 && body["items"].as_array().is_some_and(|l| l.len() >= n) {
            return body["items"].as_array().unwrap().clone();
        }
        last = body;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("fewer than {n} ledger lines for {log_id}: {last}");
}

fn chat(model: &str, prompt: &str) -> Value {
    json!({"model": model, "messages": [{"role": "user", "content": prompt}]})
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_costs_what_the_ledger_charged_for_it_gateway_calls_included() {
    let cache = json!({"enabled": true, "ttl": "10m",
        "semantic": {"enabled": true, "embedding_model": "emb", "threshold": 0.9}});
    let gw = Gateway::start(
        "accounting-auxiliary",
        json!({"cache": {"semantic_store": {"type": "memory"}}, "models": {"gpt": {"policies": {"cache": cache}}}}),
    )
    .await;
    let key = gw.key("k", json!({})).await;

    // A miss: the query is embedded (two tokens at $1 per million), then the chat answers (three
    // tokens in at $1, one out at $2 per million).
    let (status, headers, sent) = gw
        .send(&key, "/v1/chat/completions", &[], &chat("gpt", "Say hello"))
        .await;
    assert_eq!(status, 200, "{sent}");
    let miss_id = request_id(&headers);
    let miss = gw.record(&miss_id).await;
    let lines = ledger(&gw, miss["id"].as_str().unwrap(), 2).await;
    let purposes: Vec<&str> = lines.iter().map(|l| l["purpose"].as_str().unwrap()).collect();
    assert!(purposes.contains(&"cache_embedding"), "{lines:?}");
    let charged: i64 = lines.iter().map(|l| nusd(&l["billed_usd"])).sum();
    assert_eq!(charged, 7_000, "{lines:?}");
    assert_eq!(nusd(&miss["cost"]["total"]), charged, "{miss}");
    assert_eq!(nusd(&miss["cost"]["auxiliary"]), 2_000, "{miss}");
    assert_eq!(miss["cost"]["status"], "exact", "{miss}");
    // The client's header keeps the cost of its own call.
    assert_eq!(header(&headers, "x-antirion-cost"), Some("0.000005"));

    // A semantic hit: nothing upstream but the lookup, which the record still costs.
    let (status, headers, _) = gw
        .send(
            &key,
            "/v1/chat/completions",
            &[],
            &chat("gpt", "Say hello please"),
        )
        .await;
    assert_eq!(status, 200);
    let hit_id = request_id(&headers);
    let hit = gw.record(&hit_id).await;
    assert_eq!(hit["cache"]["status"], "hit", "{hit}");
    let lines = ledger(&gw, hit["id"].as_str().unwrap(), 2).await;
    let charged: i64 = lines.iter().map(|l| nusd(&l["billed_usd"])).sum();
    assert_eq!(charged, 2_000, "{lines:?}");
    assert_eq!(nusd(&hit["cost"]["total"]), charged, "{hit}");

    // A request without a lookup has no auxiliary cost.
    let (_, headers, _) = gw
        .send(&key, "/v1/chat/completions", &[], &chat("emb", "plain"))
        .await;
    let plain = gw.record(&request_id(&headers)).await;
    assert!(plain["cost"].get("auxiliary").is_none(), "{plain}");

    // The rollups behind Overview and Analytics sum to the ledger's total.
    let (status, usage) = gw.admin("GET", "/admin/v1/analytics/usage", "").await;
    assert_eq!(status, 200, "{usage}");
    let analytics: i64 = usage["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| nusd(&b["cost_usd"]))
        .sum();
    let (status, summary) = gw.admin("GET", "/admin/v1/reports/spend/summary", "").await;
    assert_eq!(status, 200, "{summary}");
    let spent: i64 = summary["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| nusd(&r["cost"]["billed_usd"]))
        .sum();
    assert_eq!(analytics, spent, "{usage} {summary}");
    assert_eq!(analytics, 7_000 + 2_000 + 5_000, "{usage}");

    gw.stop().await;
}

/// The budget events that report a refusal, oldest first (configuration changes and adjustments
/// left out).
async fn budget_events(gw: &Gateway) -> Vec<Value> {
    let (status, body) = gw.admin("GET", "/admin/v1/runtime/budgets/events", "").await;
    assert_eq!(status, 200, "{body}");
    let mut items: Vec<Value> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| !matches!(e["kind"].as_str(), Some("config_changed" | "adjusted")))
        .cloned()
        .collect();
    items.reverse();
    items
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_larger_than_what_remains_does_not_exhaust_the_budget() {
    let gw = Gateway::start("accounting-oversized", json!({})).await;
    let budget = json!({"daily": {"unit": "usd", "amount": "0.00005", "period": "daily"}});
    let key = gw.key("k", json!({"policies": {"budgets": budget}})).await;

    // Without max_tokens the request reserves far more than the $0.00005 the budget has: refused,
    // with nothing spent.
    let (status, _, sent) = gw
        .send(&key, "/v1/chat/completions", &[], &chat("gpt", "Say hello"))
        .await;
    assert_eq!(status, 402, "{sent}");
    let events = budget_events(&gw).await;
    let kinds: Vec<&str> = events.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["request_over_remaining"], "{events:?}");
    let event = &events[0];
    assert_eq!(event["level"], "key", "{event}");
    assert_eq!(event["name"], "daily", "{event}");
    assert_eq!(event["detail"]["remaining"], "0.00005", "{event}");
    assert!(event["detail"]["requested"].as_str().is_some(), "{event}");

    // Once per period: a second such refusal adds no event.
    let (status, _, _) = gw
        .send(&key, "/v1/chat/completions", &[], &chat("gpt", "Say hello again"))
        .await;
    assert_eq!(status, 402);
    assert_eq!(budget_events(&gw).await.len(), 1);

    // Spent to its amount, the budget is exhausted: the next refusal says so.
    let owner = event["owner"].as_str().unwrap();
    let (status, body) = gw
        .admin(
            "POST",
            &format!("/admin/v1/runtime/budgets/key/{owner}/daily:adjust"),
            r#"{"amount": "0.00005", "reason": "spent elsewhere"}"#,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let small = json!({"model": "gpt", "max_tokens": 1, "messages": [{"role": "user", "content": "hi"}]});
    let (status, _, sent) = gw.send(&key, "/v1/chat/completions", &[], &small).await;
    assert_eq!(status, 402, "{sent}");
    let kinds: Vec<String> = budget_events(&gw)
        .await
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["request_over_remaining", "exhausted"]);

    gw.stop().await;
}
