//! The request log records the response a streaming client received, assembled in the client's own
//! dialect: across a translation in both directions, in passthrough, with content logging off and
//! with redaction on.

use std::net::SocketAddr;
use std::time::Duration;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "stream-content-bootstrap-token-0123456789abcdef";

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

/// The answer's text: a plain greeting, or one carrying an email address when the prompt asks how
/// to get in touch.
fn answer(request: &str) -> [&'static str; 2] {
    if request.contains("contact") {
        ["Write to ", "alice.smith@example.com today"]
    } else {
        ["Hello", " there"]
    }
}

fn openai_stream(parts: [&str; 2]) -> String {
    let chunk = |delta: Value, finish: Value| {
        json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "up-gpt",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    };
    let events = [
        chunk(json!({"role": "assistant", "content": ""}), Value::Null),
        chunk(json!({"content": parts[0]}), Value::Null),
        chunk(json!({"content": parts[1]}), json!("stop")),
        json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "up-gpt",
            "choices": [], "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}}),
    ];
    let mut out: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn anthropic_stream(parts: [&str; 2]) -> String {
    let events = [
        json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant",
            "model": "up-claude", "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": {"input_tokens": 5, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": parts[0]}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": parts[1]}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
            "usage": {"output_tokens": 2}}),
        json!({"type": "message_stop"}),
    ];
    events
        .iter()
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap()))
        .collect()
}

/// An upstream that streams every answer, in Chat or Messages form by the path it is called on.
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
            let events = if path.ends_with("/messages") {
                anthropic_stream(answer(&body))
            } else {
                openai_stream(answer(&body))
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{events}",
                events.len()
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
    /// A streaming request; the record the request log keeps of it once the stream has settled.
    async fn stream(&self, path: &str, body: &Value) -> (String, Value) {
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
                return (text, serde_json::from_str(&record).unwrap());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no record for {request_id}");
    }
}

fn chat(model: &str, prompt: &str) -> Value {
    json!({"model": model, "stream": true, "messages": [{"role": "user", "content": prompt}]})
}

fn messages(model: &str, prompt: &str) -> Value {
    json!({"model": model, "stream": true, "max_tokens": 64, "messages": [{"role": "user", "content": prompt}]})
}

#[tokio::test(flavor = "multi_thread")]
async fn streamed_responses_are_logged_as_the_client_received_them() {
    let dir = ScratchDir::new("stream-content");
    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock.local_addr().unwrap();
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "providers": {
            "oa": {"type": "openai", "base_url": format!("http://{mock_addr}/v1"), "auth": {"api_key": "sk-up"}},
            "an": {"type": "anthropic", "base_url": format!("http://{mock_addr}"), "auth": {"api_key": "sk-ant-up"}}
        },
        "deployments": {
            "oa": {"provider": "oa", "model": "up-gpt"},
            "an": {"provider": "an", "model": "up-claude"}
        },
        "models": {
            "gpt": {"target": {"deployment": "oa"}},
            "claude": {"target": {"deployment": "an"}},
            "up-gpt": {"target": {"deployment": "oa"}},
            "up-claude": {"target": {"deployment": "an"}},
            "claude-quiet": {"target": {"deployment": "an"},
                "policies": {"observability": {"content": {"enabled": false}}}}
        },
        "policies": {"observability": {"content": {"enabled": true, "redact_pii": true}}}
    });
    let file = dir.path().join("gw.json");
    std::fs::write(&file, config.to_string()).unwrap();
    let node = start(ServeOptions {
        config: Some(file),
        test_mode: true,
    })
    .await
    .unwrap();
    let (data, admin) = (node.data_addr().unwrap(), node.admin_addr().unwrap());
    let shutdown = node.rt.shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(node.run());
    let mut mock_task = JoinSet::new();
    mock_task.spawn(upstream(mock));

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
    let gw = Gateway { data, admin, key };

    // A Chat client of a Messages upstream: the Chat body it was sent, with the usage reported.
    let (sent, record) = gw.stream("/v1/chat/completions", &chat("claude", "hi")).await;
    assert!(sent.contains("chat.completion.chunk"), "{sent}");
    assert_eq!(record["stream"]["relay_mode"], "translated", "{record}");
    let response = &record["content"]["response"]["body"];
    assert_eq!(response["object"], "chat.completion", "{record}");
    assert_eq!(
        response["choices"][0]["message"]["content"], "Hello there",
        "{record}"
    );
    assert_eq!(response["choices"][0]["finish_reason"], "stop", "{record}");
    assert_eq!(
        record["content"]["request"]["body"]["model"], "claude",
        "{record}"
    );
    assert_eq!(record["usage"]["input"], 5, "{record}");
    assert_eq!(record["usage"]["output"], 2, "{record}");

    // A Messages client of a Chat upstream: the Messages body it was sent.
    let (sent, record) = gw.stream("/v1/messages", &messages("gpt", "hi")).await;
    assert!(sent.contains("content_block_delta"), "{sent}");
    assert_eq!(record["stream"]["relay_mode"], "translated", "{record}");
    let response = &record["content"]["response"]["body"];
    assert_eq!(response["type"], "message", "{record}");
    assert_eq!(response["role"], "assistant", "{record}");
    assert_eq!(response["content"][0]["type"], "text", "{record}");
    assert_eq!(response["content"][0]["text"], "Hello there", "{record}");
    assert_eq!(response["stop_reason"], "end_turn", "{record}");
    assert_eq!(record["usage"]["input"], 5, "{record}");
    assert_eq!(record["usage"]["output"], 2, "{record}");

    // Passthrough Chat: assembled from the upstream's chunks, as before.
    let (_, record) = gw.stream("/v1/chat/completions", &chat("up-gpt", "hi")).await;
    assert_eq!(record["stream"]["relay_mode"], "passthrough", "{record}");
    let response = &record["content"]["response"]["body"];
    assert_eq!(response["object"], "chat.completion", "{record}");
    assert_eq!(response["model"], "up-gpt", "{record}");
    assert_eq!(
        response["choices"][0]["message"]["content"], "Hello there",
        "{record}"
    );
    assert_eq!(response["usage"]["completion_tokens"], 2, "{record}");
    assert_eq!(record["usage"]["input"], 5, "{record}");
    assert_eq!(record["usage"]["output"], 2, "{record}");

    // Passthrough Messages: logged in Messages form too.
    let (_, record) = gw.stream("/v1/messages", &messages("up-claude", "hi")).await;
    assert_eq!(record["stream"]["relay_mode"], "passthrough", "{record}");
    let response = &record["content"]["response"]["body"];
    assert_eq!(response["content"][0]["text"], "Hello there", "{record}");
    assert_eq!(
        response["usage"],
        json!({"input_tokens": 5, "output_tokens": 2}),
        "{record}"
    );

    // Content logging off: neither side is kept, the usage still is.
    let (sent, record) = gw
        .stream("/v1/chat/completions", &chat("claude-quiet", "hi"))
        .await;
    assert!(sent.contains("Hello"), "{sent}");
    assert!(record["content"].is_null(), "{record}");
    assert_eq!(record["usage"]["output"], 2, "{record}");

    // Redaction: the client got the address, the record keeps it masked.
    let (sent, record) = gw
        .stream("/v1/chat/completions", &chat("claude", "contact?"))
        .await;
    assert!(sent.contains("alice.smith@example.com"), "{sent}");
    let response = &record["content"]["response"]["body"];
    let text = response["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(text.starts_with("Write to "), "{record}");
    assert!(!text.contains("alice.smith@example.com"), "{record}");
    assert!(
        !record.to_string().contains("alice.smith@example.com"),
        "{record}"
    );

    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
}
