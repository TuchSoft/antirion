//! The request log keeps the response each client received wherever the gateway produced it: a
//! cache hit (served as JSON or as a stream), a streamed legacy Completions answer, and the usage a
//! hit of a translated stream reports.

use std::net::SocketAddr;
use std::time::Duration;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "logged-responses-bootstrap-token-0123456789abcdef";

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

/// The data of a chunked body; trailers after the last chunk are dropped.
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

fn sse(events: &[Value], done: bool) -> String {
    let mut out: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
    if done {
        out.push_str("data: [DONE]\n\n");
    }
    out
}

/// The upstream's answer to `path` with `body`: (content type, body).
fn answer(path: &str, body: &Value) -> (&'static str, String) {
    let stream = body["stream"] == true;
    let usage = json!({"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7});
    if path.ends_with("/messages") {
        if !stream {
            let v = json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "up-claude",
                "content": [{"type": "text", "text": "Hello there"}], "stop_reason": "end_turn",
                "stop_sequence": null, "usage": {"input_tokens": 5, "output_tokens": 2}});
            return ("application/json", v.to_string());
        }
        let events = [
            json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant",
                "model": "up-claude", "content": [], "stop_reason": null, "stop_sequence": null,
                "usage": {"input_tokens": 5, "output_tokens": 1}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hello"}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": " there"}}),
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                "usage": {"output_tokens": 2}}),
            json!({"type": "message_stop"}),
        ];
        let text = events
            .iter()
            .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
            .collect();
        return ("text/event-stream", text);
    }
    if path.ends_with("/v1/completions") {
        let chunk = |text: &str, finish: Value| {
            json!({"id": "cmpl-1", "object": "text_completion", "created": 1, "model": "up-gpt",
                "choices": [{"index": 0, "text": text, "finish_reason": finish}]})
        };
        if !stream {
            let mut v = chunk("Hello there", json!("stop"));
            v["usage"] = usage;
            return ("application/json", v.to_string());
        }
        let mut last = chunk("", Value::Null);
        last["choices"] = json!([]);
        last["usage"] = usage;
        let events = [chunk("Hello", Value::Null), chunk(" there", json!("stop")), last];
        return ("text/event-stream", sse(&events, true));
    }
    let chunk = |delta: Value, finish: Value| {
        json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "up-gpt",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    };
    if !stream {
        let v = json!({"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "up-gpt",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello there"},
            "finish_reason": "stop"}], "usage": usage});
        return ("application/json", v.to_string());
    }
    let events = [
        chunk(json!({"role": "assistant", "content": ""}), Value::Null),
        chunk(json!({"content": "Hello"}), Value::Null),
        chunk(json!({"content": " there"}), json!("stop")),
        json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "up-gpt",
            "choices": [], "usage": usage}),
    ];
    ("text/event-stream", sse(&events, true))
}

/// An upstream speaking Chat, Completions and Messages, streamed when the request asks.
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
            let (content_type, text) = answer(path, &serde_json::from_str(&body).unwrap_or(Value::Null));
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

struct Gateway {
    data: SocketAddr,
    admin: SocketAddr,
    key: String,
}

impl Gateway {
    /// A request; what the client received and the record the request log keeps of it.
    async fn send(&self, path: &str, body: &Value) -> (Vec<(String, String)>, String, Value) {
        let bearer = format!("Bearer {}", self.key);
        let (status, headers, text) = exchange(
            self.data,
            "POST",
            path,
            &[("authorization", bearer.as_str())],
            &body.to_string(),
        )
        .await;
        assert_eq!(status, 200, "{text}");
        let request_id = header(&headers, "x-request-id").unwrap().to_owned();
        let auth = format!("Bearer {TOKEN}");
        for _ in 0..100 {
            let (_, _, list) = exchange(
                self.admin,
                "GET",
                &format!("/admin/v1/logs?request_id={request_id}"),
                &[("authorization", auth.as_str())],
                "",
            )
            .await;
            let list: Value = serde_json::from_str(&list).unwrap_or(Value::Null);
            if let Some(id) = list["items"][0]["id"].as_str() {
                let (status, _, record) = exchange(
                    self.admin,
                    "GET",
                    &format!("/admin/v1/logs/{id}"),
                    &[("authorization", auth.as_str())],
                    "",
                )
                .await;
                assert_eq!(status, 200, "{record}");
                return (headers, text, serde_json::from_str(&record).unwrap());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no record for {request_id}");
    }
}

/// A gateway on `mock`, with a key; the closure stops it.
async fn gateway(dir: &ScratchDir, mock: SocketAddr) -> (Gateway, impl FnOnce(), JoinSet<()>) {
    let cache = json!({"cache": {"enabled": true, "ttl": "10m"}});
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "providers": {
            "oa": {"type": "openai", "base_url": format!("http://{mock}/v1"), "auth": {"api_key": "sk-up"}},
            "an": {"type": "anthropic", "base_url": format!("http://{mock}"), "auth": {"api_key": "sk-ant-up"}}
        },
        "deployments": {
            "oa": {"provider": "oa", "model": "up-gpt", "price": {"input": "1", "output": "2"}},
            "an": {"provider": "an", "model": "up-claude", "price": {"input": "1", "output": "2"}}
        },
        "models": {
            "gpt": {"target": {"deployment": "oa"}},
            "cached-gpt": {"target": {"deployment": "oa"}, "policies": cache},
            "cached-claude": {"target": {"deployment": "an"}, "policies": cache}
        },
        "policies": {"observability": {"content": {"enabled": true}}}
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
    tasks.spawn(async move {
        let _ = node.run().await;
    });
    let bearer = format!("Bearer {TOKEN}");
    let (status, _, body) = exchange(
        admin,
        "POST",
        "/admin/v1/keys",
        &[("authorization", bearer.as_str())],
        r#"{"name":"k","spec":{}}"#,
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let key = serde_json::from_str::<Value>(&body).unwrap()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    (Gateway { data, admin, key }, move || shutdown.cancel(), tasks)
}

fn chat(model: &str, prompt: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream, "messages": [{"role": "user", "content": prompt}]})
}

fn messages(model: &str, prompt: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream, "max_tokens": 64, "messages": [{"role": "user", "content": prompt}]})
}

/// The text of a logged Chat or Messages response body.
fn logged_text(record: &Value) -> Option<&str> {
    let body = &record["content"]["response"]["body"];
    body["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| body["content"][0]["text"].as_str())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cache_hit_logs_the_response_it_served() {
    let dir = ScratchDir::new("logged-hits");
    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock.local_addr().unwrap();
    let mut mock_task = JoinSet::new();
    mock_task.spawn(upstream(mock));
    let (gw, shutdown, mut tasks) = gateway(&dir, mock_addr).await;

    let cases = [
        ("/v1/chat/completions", chat("cached-gpt", "json hit", false)),
        ("/v1/chat/completions", chat("cached-gpt", "stream hit", true)),
        ("/v1/messages", messages("cached-claude", "json hit", false)),
        ("/v1/messages", messages("cached-claude", "stream hit", true)),
        (
            "/v1/chat/completions",
            chat("cached-claude", "translated json hit", false),
        ),
        (
            "/v1/chat/completions",
            chat("cached-claude", "translated stream hit", true),
        ),
    ];
    for (path, body) in &cases {
        let (_, _, miss) = gw.send(path, body).await;
        assert_eq!(miss["cache"]["status"], "miss", "{miss}");
        assert_eq!(logged_text(&miss), Some("Hello there"), "{miss}");
        let (headers, sent, hit) = gw.send(path, body).await;
        assert_eq!(header(&headers, "x-antirion-cache"), Some("hit"), "{body}");
        assert!(sent.contains("Hello") && sent.contains("there"), "{sent}");
        assert_eq!(hit["response_path"], "cache", "{hit}");
        // The drawer's Response and Messages tabs read this: the answer the hit served.
        assert_eq!(logged_text(&hit), Some("Hello there"), "{body}: {hit}");
        // The hit reports the original usage, which the savings are priced from.
        assert_eq!(hit["usage"]["input"], miss["usage"]["input"], "{body}: {hit}");
        assert_eq!(hit["usage"]["output"], 2, "{body}: {hit}");
    }

    // A stream-written entry answers a JSON request for the same prompt, logged as JSON.
    let (_, _, hit) = gw
        .send("/v1/chat/completions", &chat("cached-gpt", "stream hit", false))
        .await;
    assert_eq!(hit["cache"]["status"], "hit", "{hit}");
    assert_eq!(
        hit["content"]["response"]["body"]["object"], "chat.completion",
        "{hit}"
    );
    assert_eq!(logged_text(&hit), Some("Hello there"), "{hit}");

    shutdown();
    while tasks.join_next().await.is_some() {}
    mock_task.abort_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn streamed_completions_are_logged_as_completions_under_the_public_model() {
    let dir = ScratchDir::new("logged-completions");
    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock.local_addr().unwrap();
    let mut mock_task = JoinSet::new();
    mock_task.spawn(upstream(mock));
    let (gw, shutdown, mut tasks) = gateway(&dir, mock_addr).await;

    let (_, sent, record) = gw
        .send(
            "/v1/completions",
            &json!({"model": "gpt", "stream": true, "prompt": "Say hello"}),
        )
        .await;
    assert!(sent.contains("\"model\":\"gpt\""), "{sent}");
    let body = &record["content"]["response"]["body"];
    assert_eq!(body["object"], "text_completion", "{record}");
    assert_eq!(body["choices"][0]["text"], "Hello there", "{record}");
    assert_eq!(body["choices"][0]["finish_reason"], "stop", "{record}");
    assert_eq!(body["model"], "gpt", "{record}");

    // A streamed Chat answer is logged under the model name the client was sent, as a JSON one is.
    let (_, sent, record) = gw
        .send("/v1/chat/completions", &chat("gpt", "Say hello", true))
        .await;
    assert!(sent.contains("\"model\":\"gpt\""), "{sent}");
    assert_eq!(record["content"]["response"]["body"]["model"], "gpt", "{record}");
    assert_eq!(logged_text(&record), Some("Hello there"), "{record}");

    shutdown();
    while tasks.join_next().await.is_some() {}
    mock_task.abort_all();
}
