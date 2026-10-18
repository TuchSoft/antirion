//! With content logging on, an MCP request's record keeps the JSON-RPC call the client sent and the
//! JSON answer it received, like a model request's record keeps its prompt and response.

use std::net::SocketAddr;
use std::time::Duration;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "mcp-content-bootstrap-token-0123456789abcdef0123";

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

/// The REST API behind the `openapi` server: every request gets the same customer list.
async fn api(listener: TcpListener) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            let mut seen = 0;
            loop {
                let Ok(n) = stream.read(&mut buf[seen..]).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                seen += n;
                if buf[..seen].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let body = r#"{"customers":[{"name":"Ada","plan":"team"}]}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes()).await;
        });
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_mcp_tool_call_keeps_its_call_and_answer_as_content() {
    let dir = ScratchDir::new("mcp-content");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_addr = listener.local_addr().unwrap();
    let mut api_task = JoinSet::new();
    api_task.spawn(api(listener));

    let spec = json!({"openapi": "3.1.0", "info": {"title": "crm", "version": "1"},
        "paths": {"/customers": {"get": {"operationId": "search_customers", "summary": "Find customers.",
            "responses": {"200": {"description": "ok"}}}}}})
    .to_string();
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "security": {"egress": {"operator": {"allow_cidrs": ["127.0.0.0/8"]}}},
        "mcp": {
            "servers": {"crm": {"transport": "openapi", "url": format!("http://{api_addr}"), "openapi": {"spec": spec}}},
            "gateways": {"g": {"servers": ["crm"]}}
        },
        "policies": {"mcp": {"servers": ["*"]}, "observability": {"content": {"enabled": true}}}
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
    let auth = format!("Bearer {key}");
    let accept = ("accept", "application/json, text/event-stream");

    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}});
    let (status, headers, body) = exchange(
        data,
        "POST",
        "/mcp/g",
        &[("authorization", auth.as_str()), accept],
        &init.to_string(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let session = headers
        .iter()
        .find(|(k, _)| k == "mcp-session-id")
        .map(|(_, v)| v.clone())
        .expect("a session id");

    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "search_customers", "arguments": {}}});
    let (status, headers, body) = exchange(
        data,
        "POST",
        "/mcp/g",
        &[
            ("authorization", auth.as_str()),
            accept,
            ("mcp-session-id", session.as_str()),
            ("mcp-protocol-version", "2025-06-18"),
        ],
        &call.to_string(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "content-type" && v.starts_with("application/json")),
        "{headers:?}"
    );
    assert!(body.contains("Ada"), "{body}");

    let mut record = Value::Null;
    for _ in 0..100 {
        let (_, _, logs) = exchange(
            admin,
            "GET",
            "/admin/v1/logs?limit=20",
            &[("authorization", bearer.as_str())],
            "",
        )
        .await;
        let logs: Value = serde_json::from_str(&logs).unwrap();
        if let Some(r) = logs["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["mcp"]["tool"] == "search_customers")
        {
            record = r.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        record["content"]["request"]["body"]["method"], "tools/call",
        "{record}"
    );
    assert_eq!(
        record["content"]["request"]["body"]["params"]["name"], "search_customers",
        "{record}"
    );
    let answer = record["content"]["response"]["body"]["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(answer.contains("Ada"), "{record}");

    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
    api_task.abort_all();
}
