//! The spend summary sums the lines the ledger's filters select, so a filtered ledger's total does
//! not depend on how many of its pages were loaded.

use std::net::SocketAddr;
use std::time::Duration;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "spend-summary-bootstrap-token-0123456789abcdef";

async fn call(addr: SocketAddr, method: &str, path: &str, auth: &str, body: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nhost: test\r\nconnection: close\r\nauthorization: Bearer {auth}\r\n\
         content-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let rest = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        dechunk(rest)
    } else {
        rest.to_owned()
    };
    (status, serde_json::from_str(&rest).unwrap_or(Value::Null))
}

fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&rest[..n]);
        s = &rest[n + 2..];
    }
    out
}

/// An upstream that answers every chat completion.
async fn upstream(listener: TcpListener) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        connections.spawn(async move {
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen);
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
                        break;
                    }
                }
            }
            let reply = r#"{"id":"c1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

fn requests(summary: &Value) -> u64 {
    summary["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["requests"].as_u64().unwrap())
        .sum()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_summary_takes_the_ledger_filters() {
    let dir = ScratchDir::new("spend-summary");
    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mock_addr = mock.local_addr().unwrap();
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "providers": {"p": {"type": "openai", "base_url": format!("http://{mock_addr}/v1"), "auth": {"api_key": "sk-up"}}},
        "deployments": {"p": {"provider": "p", "model": "up-model"}},
        "models": {"m": {"target": {"deployment": "p"}}}
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
    let mut mock_task = JoinSet::new();
    mock_task.spawn(upstream(mock));

    let mut secrets = Vec::new();
    for name in ["one", "two"] {
        let (status, key) = call(
            admin,
            "POST",
            "/admin/v1/keys",
            TOKEN,
            &json!({"name": name, "spec": {}}).to_string(),
        )
        .await;
        assert_eq!(status, 201, "{key}");
        secrets.push(key["secret"].as_str().unwrap().to_owned());
    }
    let chat = r#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#;
    for secret in [&secrets[0], &secrets[0], &secrets[1]] {
        let (status, body) = call(data, "POST", "/v1/chat/completions", secret, chat).await;
        assert_eq!(status, 200, "{body}");
    }

    let mut all = Value::Null;
    for _ in 0..50 {
        all = call(admin, "GET", "/admin/v1/reports/spend/summary", TOKEN, "")
            .await
            .1;
        if requests(&all) == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(requests(&all), 3, "{all}");

    let (status, one) = call(admin, "GET", "/admin/v1/reports/spend/summary?key=one", TOKEN, "").await;
    assert_eq!(status, 200, "{one}");
    assert_eq!(requests(&one), 2, "{one}");
    let (_, ledger) = call(admin, "GET", "/admin/v1/reports/spend/ledger?key=one", TOKEN, "").await;
    let listed: std::collections::BTreeSet<&str> = ledger["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["request_id"].as_str().unwrap())
        .collect();
    assert_eq!(listed.len(), 2, "the ledger and the summary agree: {ledger}");

    let (_, grouped) = call(
        admin,
        "GET",
        "/admin/v1/reports/spend/summary?key=two&group_by=key",
        TOKEN,
        "",
    )
    .await;
    assert_eq!(grouped["items"].as_array().unwrap().len(), 1, "{grouped}");
    assert_eq!(grouped["items"][0]["group"], json!({"key": "two"}));
    assert_eq!(grouped["items"][0]["requests"], 1);

    let (status, none) = call(
        admin,
        "GET",
        "/admin/v1/reports/spend/summary?model=elsewhere",
        TOKEN,
        "",
    )
    .await;
    assert_eq!(status, 200, "{none}");
    assert_eq!(none["items"], json!([]));
    let (status, bad) = call(
        admin,
        "GET",
        "/admin/v1/reports/spend/summary?colour=red",
        TOKEN,
        "",
    )
    .await;
    assert_eq!(status, 400, "{bad}");

    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
}
