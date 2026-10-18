//! Reversible tokens: the provider sees `[PHONE_1]`, the client gets its number back, on Chat,
//! Messages and Responses, streamed and not, translated and passthrough, with the token cut across
//! stream chunks and wrapped in quotes and brackets; the request log keeps the answer the client
//! received, with the restored values masked as its redaction says.

use std::net::SocketAddr;
use std::time::Duration;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "pii-restore-bootstrap-token-0123456789abcdef";
const PHONE: &str = "+5511978399280";
const EMAIL: &str = "ana@acme.example";

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

/// Every `[LABEL_N]` token in `text`, in order.
fn tokens_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find('[') {
        rest = &rest[at + 1..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[..end];
        let numbered = inner
            .rsplit_once('_')
            .is_some_and(|(l, n)| !l.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty());
        if numbered
            && inner
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            let t = format!("[{inner}]");
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// The answer the model gives: each token it was sent, quoted then bracketed, as a model that
/// repeats a placeholder does; and a mark when it was sent a value in clear.
fn answer(request: &str) -> String {
    let mut text = String::from("It came through as just a placeholder");
    for t in tokens_in(request) {
        text.push_str(&format!(", \"{t}\" ([{t}])"));
    }
    if request.contains(PHONE) || request.contains(EMAIL) {
        text.push_str(" LEAKED");
    }
    text
}

/// The answer cut into 4-byte pieces, so every token is split across chunks.
fn pieces(text: &str) -> Vec<String> {
    text.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect()
}

fn sse(events: &[Value], named: bool) -> String {
    events
        .iter()
        .map(|e| match (named, e["type"].as_str()) {
            (true, Some(t)) => format!("event: {t}\ndata: {e}\n\n"),
            _ => format!("data: {e}\n\n"),
        })
        .collect()
}

fn chat_reply(text: &str, stream: bool) -> (&'static str, String) {
    if !stream {
        let body = json!({"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": "up-gpt",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}});
        return ("application/json", body.to_string());
    }
    let chunk = |delta: Value, finish: Value| {
        json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "up-gpt",
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
    };
    let mut events = vec![chunk(json!({"role": "assistant", "content": ""}), Value::Null)];
    events.extend(
        pieces(text)
            .iter()
            .map(|p| chunk(json!({"content": p}), Value::Null)),
    );
    events.push(chunk(json!({}), json!("stop")));
    events.push(
        json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "up-gpt",
        "choices": [], "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}}),
    );
    let mut out = sse(&events, false);
    out.push_str("data: [DONE]\n\n");
    ("text/event-stream", out)
}

fn messages_reply(text: &str, stream: bool) -> (&'static str, String) {
    let message = |content: Value| {
        json!({"id": "msg_1", "type": "message", "role": "assistant", "model": "up-claude", "content": content,
            "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 5, "output_tokens": 1}})
    };
    if !stream {
        let mut m = message(json!([{"type": "text", "text": text}]));
        m["stop_reason"] = json!("end_turn");
        return ("application/json", m.to_string());
    }
    let mut events = vec![
        json!({"type": "message_start", "message": message(json!([]))}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
    ];
    events.extend(pieces(text).iter().map(
        |p| json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": p}}),
    ));
    events.extend([
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
            "usage": {"output_tokens": 2}}),
        json!({"type": "message_stop"}),
    ]);
    ("text/event-stream", sse(&events, true))
}

fn responses_reply(text: &str, stream: bool) -> (&'static str, String) {
    let item = |text: &str, status: &str| {
        json!({"type": "message", "id": "msg_1", "status": status, "role": "assistant",
            "content": [{"type": "output_text", "text": text, "annotations": []}]})
    };
    let response = |status: &str, output: Value| {
        json!({"id": "resp_1", "object": "response", "created_at": 1, "status": status, "model": "up-gpt",
            "output": output, "usage": {"input_tokens": 5, "output_tokens": 2, "total_tokens": 7}})
    };
    if !stream {
        return (
            "application/json",
            response("completed", json!([item(text, "completed")])).to_string(),
        );
    }
    let mut events = vec![
        json!({"type": "response.created", "response": response("in_progress", json!([]))}),
        json!({"type": "response.output_item.added", "output_index": 0,
            "item": {"type": "message", "id": "msg_1", "status": "in_progress", "role": "assistant", "content": []}}),
        json!({"type": "response.content_part.added", "item_id": "msg_1", "output_index": 0, "content_index": 0,
            "part": {"type": "output_text", "text": "", "annotations": []}}),
    ];
    events.extend(pieces(text).iter().map(|p| {
        json!({"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0,
            "delta": p})
    }));
    events.extend([
        json!({"type": "response.output_text.done", "item_id": "msg_1", "output_index": 0, "content_index": 0,
            "text": text}),
        json!({"type": "response.content_part.done", "item_id": "msg_1", "output_index": 0, "content_index": 0,
            "part": {"type": "output_text", "text": text, "annotations": []}}),
        json!({"type": "response.output_item.done", "output_index": 0, "item": item(text, "completed")}),
        json!({"type": "response.completed", "response": response("completed", json!([item(text, "completed")]))}),
    ]);
    let events: Vec<Value> = events
        .into_iter()
        .enumerate()
        .map(|(i, mut e)| {
            e["sequence_number"] = json!(i);
            e
        })
        .collect();
    ("text/event-stream", sse(&events, true))
}

/// An upstream that answers Chat, Messages and Responses by the path it is called on, streamed
/// when the request asks for it.
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
            let streamed = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|b| b.get("stream").and_then(Value::as_bool))
                .unwrap_or(false);
            let text = answer(&body);
            let (ct, reply) = if path.ends_with("/messages") {
                messages_reply(&text, streamed)
            } else if path.ends_with("/responses") {
                responses_reply(&text, streamed)
            } else {
                chat_reply(&text, streamed)
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {ct}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
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
    async fn send(&self, path: &str, body: &Value) -> (String, Value) {
        let bearer = format!("Bearer {}", self.key);
        let (status, headers, text) = exchange(
            self.data,
            "POST",
            path,
            &[("authorization", bearer.as_str())],
            &body.to_string(),
        )
        .await;
        assert_eq!(status, 200, "{path} {body}: {text}");
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

/// The text a client received, streamed or not, in any of the three dialects.
fn received(sent: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(sent) {
        return collect_text(&v);
    }
    let mut out = String::new();
    for line in sent.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        if let Some(t) = v.pointer("/choices/0/delta/content").and_then(Value::as_str) {
            out.push_str(t);
        }
        if v["type"] == "content_block_delta"
            && let Some(t) = v.pointer("/delta/text").and_then(Value::as_str)
        {
            out.push_str(t);
        }
        if v["type"] == "response.output_text.delta"
            && let Some(t) = v["delta"].as_str()
        {
            out.push_str(t);
        }
    }
    out
}

fn collect_text(v: &Value) -> String {
    let paths = [
        "/choices/0/message/content",
        "/content/0/text",
        "/output/0/content/0/text",
    ];
    paths
        .iter()
        .find_map(|p| v.pointer(p).and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn chat(model: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream,
        "messages": [{"role": "user", "content": format!("my phone is {PHONE}, mail {EMAIL}")}]})
}

fn messages(model: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream, "max_tokens": 64,
        "messages": [{"role": "user", "content": format!("my phone is {PHONE}, mail {EMAIL}")}]})
}

fn responses(model: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream, "input": format!("my phone is {PHONE}, mail {EMAIL}")})
}

#[tokio::test(flavor = "multi_thread")]
async fn restored_values_reach_the_client_on_every_path_and_stay_masked_in_the_log() {
    let dir = ScratchDir::new("pii-restore");
    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock.local_addr().unwrap();
    // The reversible rule is attached to the input stage only, as a new rule is by default.
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
            "claude": {"target": {"deployment": "an"}}
        },
        "guardrails": {"definitions": {
            "personal": {"type": "pii", "action": "redact",
                "pii": {"entities": ["phone", "email"], "reversible": true}}
        }},
        "policies": {
            "guardrails": {"input": ["personal"]},
            "observability": {"content": {"enabled": true, "redact_pii": true}}
        }
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

    let cases = [
        (
            "chat to messages, streamed",
            "/v1/chat/completions",
            chat("claude", true),
        ),
        ("chat to messages", "/v1/chat/completions", chat("claude", false)),
        (
            "chat passthrough, streamed",
            "/v1/chat/completions",
            chat("gpt", true),
        ),
        ("chat passthrough", "/v1/chat/completions", chat("gpt", false)),
        (
            "messages passthrough, streamed",
            "/v1/messages",
            messages("claude", true),
        ),
        ("messages passthrough", "/v1/messages", messages("claude", false)),
        (
            "messages to chat, streamed",
            "/v1/messages",
            messages("gpt", true),
        ),
        ("messages to chat", "/v1/messages", messages("gpt", false)),
        (
            "responses to messages, streamed",
            "/v1/responses",
            responses("claude", true),
        ),
        (
            "responses to messages",
            "/v1/responses",
            responses("claude", false),
        ),
        (
            "responses passthrough, streamed",
            "/v1/responses",
            responses("gpt", true),
        ),
        ("responses passthrough", "/v1/responses", responses("gpt", false)),
    ];
    for (case, path, body) in cases {
        let (sent, record) = gw.send(path, &body).await;
        let text = received(&sent);
        assert!(
            !text.contains("LEAKED"),
            "{case}: the provider saw a value: {text}"
        );
        assert_eq!(
            text,
            format!(
                "It came through as just a placeholder, \"{PHONE}\" ([{PHONE}]), \"{EMAIL}\" ([{EMAIL}])"
            ),
            "{case}: {sent}"
        );
        assert!(
            !sent.contains("[PHONE_1]") && !sent.contains("[EMAIL_1]"),
            "{case}: {sent}"
        );
        let logged = &record["content"]["response"]["body"];
        let logged_text = collect_text(logged);
        assert_eq!(
            logged_text,
            "It came through as just a placeholder, \"[PHONE]\" ([[PHONE]]), \"[EMAIL]\" ([[EMAIL]])",
            "{case}: {record}"
        );
        let all = record.to_string();
        assert!(!all.contains(PHONE) && !all.contains(EMAIL), "{case}: {record}");
    }

    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
}
