//! Decision endpoints over a whole node: the OpenAI shape on `/v1/decisions`, identity for an
//! OpenAI deployment, translation to System One for TypeSafe/Microsoft deployments, and the legacy
//! `/v1/systemone` identity surface.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "decisions-bootstrap-token-0123456789abcdef";

#[derive(Clone, Debug)]
struct Record {
    path: String,
    body: Value,
    headers: Vec<(String, String)>,
}

async fn call(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, Value, Vec<(String, String)>) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\ncontent-length: {}\r\n",
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
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let headers: Vec<(String, String)> = head
        .lines()
        .skip(1)
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let chunked = headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));
    let rest = if chunked { dechunk(rest) } else { rest.to_owned() };
    (
        status,
        serde_json::from_str(&rest).unwrap_or(Value::Null),
        headers,
    )
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

async fn upstream(listener: TcpListener, records: Arc<Mutex<Vec<Record>>>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let records = Arc::clone(&records);
        connections.spawn(async move {
            serve_connection(stream, records).await;
        });
    }
}

async fn serve_connection(mut stream: TcpStream, records: Arc<Mutex<Vec<Record>>>) {
    let mut seen: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let text = String::from_utf8_lossy(&seen).into_owned();
        let Some((head, rest)) = text.split_once("\r\n\r\n") else {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => seen.extend_from_slice(&buf[..n]),
            }
            continue;
        };
        let lower = head.to_ascii_lowercase();
        let len: usize = lower
            .lines()
            .find_map(|l| {
                l.strip_prefix("content-length:")
                    .map(|v| v.trim().parse().unwrap())
            })
            .unwrap_or(0);
        if rest.len() < len {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => seen.extend_from_slice(&buf[..n]),
            }
            continue;
        }
        let consumed = head.len() + 4 + len;
        let path = head.split(' ').nth(1).unwrap_or_default().to_owned();
        let headers: Vec<(String, String)> = head
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        let body: Value = serde_json::from_str(&rest[..len]).unwrap_or(Value::Null);
        seen.drain(..consumed);
        records.lock().unwrap().push(Record {
            path: path.clone(),
            body: body.clone(),
            headers,
        });
        let reply = respond(&path, &body);
        let bytes = serde_json::to_vec(&reply).unwrap_or_default();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: keep-alive\r\n\r\n",
            bytes.len()
        );
        if stream.write_all(response.as_bytes()).await.is_err() {
            return;
        }
        if stream.write_all(&bytes).await.is_err() {
            return;
        }
    }
}

fn respond(path: &str, body: &Value) -> Value {
    if path.ends_with("/decisions") {
        let answers: Vec<Value> = body
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|q| match q.get("type").and_then(Value::as_str) {
                Some("choice") => {
                    let choices = q
                        .get("choices")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let value = choices
                        .first()
                        .and_then(|c| c.get("value"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let probabilities: Vec<Value> = choices
                        .iter()
                        .map(|c| {
                            json!({
                                "value": c.get("value").cloned().unwrap_or(Value::Null),
                                "probability": if c.get("value") == Some(&value) { 0.9 } else { 0.1 }
                            })
                        })
                        .collect();
                    json!({
                        "type": "choice",
                        "name": q.get("name").cloned().unwrap_or(Value::Null),
                        "choice": value,
                        "probabilities": probabilities,
                        "confidence": 0.8
                    })
                }
                Some("score") => {
                    let levels = q
                        .get("levels")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let probabilities: Vec<Value> = levels
                        .iter()
                        .enumerate()
                        .map(|(i, level)| {
                            json!({
                                "value": i,
                                "label": level.get("label").cloned().unwrap_or(Value::Null),
                                "probability": if i == 0 { 1.0 } else { 0.0 }
                            })
                        })
                        .collect();
                    json!({
                        "type": "score",
                        "name": q.get("name").cloned().unwrap_or(Value::Null),
                        "score": 0.0,
                        "probabilities": probabilities,
                        "confidence": 1.0
                    })
                }
                _ => json!({
                    "type": "predicate",
                    "name": q.get("name").cloned().unwrap_or(Value::Null),
                    "probability": 0.9
                }),
            })
            .collect();
        return json!({
            "model": "gpt-6-luna",
            "answers": answers,
            "usage": {
                "input_tokens": 10,
                "input_tokens_details": {"cached_tokens": 0, "cache_write_tokens": 0},
                "output_tokens": 0,
                "output_tokens_details": {"reasoning_tokens": 0},
                "total_tokens": 10
            }
        });
    }
    if path.ends_with("/systemone") {
        if body.get("model").and_then(Value::as_str) == Some("empty") {
            return json!({});
        }
        let mut answers = serde_json::Map::new();
        if let Some(questions) = body.get("questions").and_then(Value::as_object) {
            for (name, q) in questions {
                let answer = match q.get("type").and_then(Value::as_str) {
                    Some("choice") => {
                        let criteria = q.get("criteria").and_then(Value::as_object);
                        let choice = criteria
                            .and_then(|c| c.keys().next())
                            .cloned()
                            .unwrap_or_default();
                        let probabilities = criteria
                            .map(|c| {
                                c.keys()
                                    .map(|k| (k.clone(), if *k == choice { json!(0.9) } else { json!(0.1) }))
                                    .collect::<serde_json::Map<String, Value>>()
                            })
                            .unwrap_or_default();
                        json!({
                            "type": "choice",
                            "choice": choice,
                            "probabilities": probabilities,
                            "confidence": 0.8
                        })
                    }
                    Some("score") => {
                        let levels = q
                            .get("criteria")
                            .and_then(Value::as_array)
                            .map(Vec::len)
                            .unwrap_or(0);
                        let probabilities = (0..levels)
                            .map(|i| (i.to_string(), if i == 0 { json!(1.0) } else { json!(0.0) }))
                            .collect::<serde_json::Map<String, Value>>();
                        json!({
                            "type": "score",
                            "score": 0.0,
                            "probabilities": probabilities,
                            "confidence": 1.0
                        })
                    }
                    _ => json!({"type": "noul", "noul": 0.9}),
                };
                answers.insert(name.clone(), answer);
            }
        }
        return json!({
            "model": "jev-1.13.0",
            "answers": answers,
            "usage": {"input_tokens": 20, "output_tokens": 5}
        });
    }
    json!({})
}

struct Gateway {
    data: SocketAddr,
    admin: SocketAddr,
    key: String,
}

impl Gateway {
    async fn admin(&self, method: &str, path: &str, body: &str) -> (u16, Value) {
        let bearer = format!("Bearer {TOKEN}");
        let (status, value, _) = call(
            self.admin,
            method,
            path,
            &[
                ("authorization", bearer.as_str()),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await;
        (status, value)
    }

    async fn post(&self, path: &str, body: &Value) -> (u16, Value, Vec<(String, String)>) {
        let bearer = format!("Bearer {}", self.key);
        call(
            self.data,
            "POST",
            path,
            &[
                ("authorization", bearer.as_str()),
                ("content-type", "application/json"),
            ],
            &body.to_string(),
        )
        .await
    }
}

async fn gateway(dir: &ScratchDir, mock: SocketAddr) -> (Gateway, impl FnOnce(), JoinSet<()>) {
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "providers": {
            "oa": {"type": "openai", "base_url": format!("http://{mock}/v1"), "auth": {"api_key": "sk-oa"}},
            "az": {"type": "azure_openai", "base_url": format!("http://{mock}"), "auth": {"api_key": "sk-az"}},
            "ts": {
                "type": "openai_compatible",
                "base_url": format!("http://{mock}/v1"),
                "auth": {"api_key": "sk-ts"},
                "capabilities": {"dialects": ["systemone"], "operations": ["decide"]}
            },
            "ms": {
                "type": "openai_compatible",
                "base_url": format!("http://{mock}"),
                "auth": {"method": "api_key", "api_key": "sk-ms", "header": "api-key", "prefix": ""},
                "operations": {"decide": {"path": "/providers/microsoft/v1/systemone"}},
                "capabilities": {"dialects": ["systemone"], "operations": ["decide"], "vision": false}
            }
        },
        "deployments": {
            "oa": {"provider": "oa", "model": "gpt-6-luna"},
            "az": {"provider": "az", "model": "gpt-6-luna"},
            "ts": {"provider": "ts", "model": "jev-1.13.0"},
            "ms": {"provider": "ms", "model": "Decision-1"},
            "empty": {"provider": "ts", "model": "empty"}
        },
        "models": {
            "gpt": {"target": {"deployment": "oa"}},
            "azure": {"target": {"deployment": "az"}},
            "jev": {"target": {"deployment": "ts"}},
            "microsoft": {"target": {"deployment": "ms"}},
            "empty": {"target": {"deployment": "empty"}}
        }
    });
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
    let shutdown = node.rt.shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(node.run());
    let mut gw = Gateway {
        data,
        admin,
        key: String::new(),
    };
    let (status, created) = gw
        .admin(
            "POST",
            "/admin/v1/keys",
            &json!({"name": "test", "spec": {}}).to_string(),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    gw.key = created["secret"].as_str().unwrap().to_owned();
    (gw, move || shutdown.cancel(), tasks)
}

fn body() -> Value {
    json!({
        "model": "gpt",
        "input": "I was charged twice.",
        "questions": [
            {"type": "predicate", "name": "billing", "instructions": "Is this billing?"},
            {"type": "choice", "name": "department", "instructions": "Which team?",
             "choices": [{"value": "billing"}, {"value": true}]},
            {"type": "score", "name": "severity", "instructions": "How bad?",
             "levels": [{"label": "Low"}, {"label": "High"}]}
        ]
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decisions_identity_translation_and_legacy_systemone() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock = listener.local_addr().unwrap();
    let records = Arc::new(Mutex::new(Vec::new()));
    let mut mock_tasks = JoinSet::new();
    mock_tasks.spawn(upstream(listener, Arc::clone(&records)));

    let dir = ScratchDir::new("decisions");
    let (gw, stop, _tasks) = gateway(&dir, mock).await;

    // OpenAI identity.
    let mut openai = body();
    openai["model"] = json!("gpt");
    let (status, answer, headers) = gw.post("/v1/decisions", &openai).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["answers"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        header(&headers, "x-antirion-client-dialect"),
        Some("openai_decisions")
    );
    assert_eq!(
        header(&headers, "x-antirion-upstream-dialect"),
        Some("openai_decisions")
    );
    {
        let records = records.lock().unwrap();
        let last = records.last().unwrap();
        assert_eq!(last.path, "/v1/decisions");
        assert_eq!(last.body["model"], "gpt-6-luna");
    }

    // Azure OpenAI identity: the resource-root base URL and the v1 path.
    let mut azure = body();
    azure["model"] = json!("azure");
    let (status, answer, _) = gw.post("/v1/decisions", &azure).await;
    assert_eq!(status, 200, "{answer}");
    {
        let records = records.lock().unwrap();
        let last = records.last().unwrap();
        assert_eq!(last.path, "/openai/v1/decisions");
        assert_eq!(
            last.headers
                .iter()
                .find(|(k, _)| k == "api-key")
                .map(|(_, v)| v.as_str()),
            Some("sk-az")
        );
    }

    // OpenAI shape translated to System One (TypeSafe).
    let mut jev = body();
    jev["model"] = json!("jev");
    let (status, answer, headers) = gw.post("/v1/decisions", &jev).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["answers"].as_array().map(Vec::len), Some(3));
    assert_eq!(answer["answers"][1]["choice"], "billing");
    assert_eq!(answer["answers"][1]["probabilities"][1]["value"], true);
    assert_eq!(answer["answers"][2]["probabilities"][0]["label"], "Low");
    assert_eq!(answer["usage"]["input_tokens"], 20);
    assert_eq!(answer["usage"]["output_tokens"], 5);
    assert_eq!(header(&headers, "x-antirion-upstream-dialect"), Some("systemone"));
    {
        let records = records.lock().unwrap();
        let last = records.last().unwrap();
        assert_eq!(last.path, "/v1/systemone");
        assert_eq!(last.body["model"], "jev-1.13.0");
        assert_eq!(last.body["state"], "I was charged twice.");
        assert_eq!(last.body["questions"]["billing"]["type"], "noul");
        assert_eq!(
            last.body["questions"]["department"]["criteria"],
            json!({"billing": null, "true": null})
        );
        assert_eq!(
            last.body["questions"]["severity"]["criteria"],
            json!(["Low", "High"])
        );
    }

    // Microsoft-Decision-1: the Foundry path and the api-key header.
    let mut microsoft = body();
    microsoft["model"] = json!("microsoft");
    let (status, answer, _) = gw.post("/v1/decisions", &microsoft).await;
    assert_eq!(status, 200, "{answer}");
    {
        let records = records.lock().unwrap();
        let last = records.last().unwrap();
        assert_eq!(last.path, "/providers/microsoft/v1/systemone");
        assert_eq!(
            last.headers
                .iter()
                .find(|(k, _)| k == "api-key")
                .map(|(_, v)| v.as_str()),
            Some("sk-ms")
        );
    }

    // Legacy System One identity.
    let systemone = json!({
        "model": "jev",
        "state": "I was charged twice.",
        "questions": {
            "billing": {"type": "noul", "instructions": "Is this billing?"},
            "department": {"type": "choice", "instructions": "Which team?",
                           "criteria": {"billing": "Payments", "technical": null}},
            "severity": {"type": "score", "instructions": "How bad?",
                         "criteria": ["Low", "High"]}
        }
    });
    let (status, answer, _) = gw.post("/v1/systemone", &systemone).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["answers"]["billing"]["type"], "noul");
    assert_eq!(answer["answers"]["department"]["type"], "choice");
    assert_eq!(answer["usage"]["output_tokens"], 5);
    {
        let records = records.lock().unwrap();
        let last = records.last().unwrap();
        assert_eq!(last.path, "/v1/systemone");
        assert_eq!(last.body["state"], "I was charged twice.");
    }

    // System One assets set the TypeSafe image header on the upstream request.
    let assets = json!({
        "model": "jev",
        "state": "x",
        "assets": {"img": {"type": "image", "format": "png", "data_base64": "AA=="}},
        "questions": {"q": {"type": "noul", "instructions": "?"}}
    });
    let (status, answer, _) = gw.post("/v1/systemone", &assets).await;
    assert_eq!(status, 200, "{answer}");
    {
        let records = records.lock().unwrap();
        let last = records.last().unwrap();
        assert_eq!(
            last.headers
                .iter()
                .find(|(k, _)| k == "typesafe-danger-zone-images")
                .map(|(_, v)| v.as_str()),
            Some("true")
        );
    }

    // A missing upstream answer becomes an OpenAI refusal, not a 502.
    let mut empty = body();
    empty["model"] = json!("empty");
    empty["questions"] = json!([{"type": "predicate", "name": "x", "instructions": "?"}]);
    let (status, answer, _) = gw.post("/v1/decisions", &empty).await;
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["answers"][0], json!({"type": "refusal", "name": "x"}));

    // `/v1/systemone` points an OpenAI-only model at `/v1/decisions`.
    let mut hint = systemone.clone();
    hint["model"] = json!("gpt");
    let (status, answer, _) = gw.post("/v1/systemone", &hint).await;
    assert_eq!(status, 400, "{answer}");
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("/v1/decisions")),
        "{answer}"
    );

    // Validation: duplicate names, images on a System One target, and a bad stream flag.
    let duplicate = json!({
        "model": "gpt",
        "input": "x",
        "questions": [
            {"type": "predicate", "name": "a", "instructions": "?"},
            {"type": "predicate", "name": "a", "instructions": "?"}
        ]
    });
    let (status, answer, _) = gw.post("/v1/decisions", &duplicate).await;
    assert_eq!(status, 400, "{answer}");

    let image = json!({
        "model": "jev",
        "input": [{"role": "user", "content": [
            {"type": "input_text", "text": "x"},
            {"type": "input_image", "image_url": "https://example.com/x.png"}
        ]}],
        "questions": [{"type": "predicate", "instructions": "?"}]
    });
    let (status, answer, _) = gw.post("/v1/decisions", &image).await;
    assert_eq!(status, 400, "{answer}");
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("input_text")),
        "{answer}"
    );

    let streamed = json!({
        "model": "gpt",
        "input": "x",
        "stream": true,
        "questions": [{"type": "predicate", "instructions": "?"}]
    });
    let (status, answer, _) = gw.post("/v1/decisions", &streamed).await;
    assert_eq!(status, 400, "{answer}");

    stop();
    mock_tasks.abort_all();
}
