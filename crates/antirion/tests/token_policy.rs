//! `GET /auth/me` tells the caller which lifetimes a personal admin token may have, from the same
//! settings `POST /auth/me/tokens` enforces, so a client offers only lifetimes the gateway accepts.

use std::net::SocketAddr;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

const TOKEN: &str = "token-policy-bootstrap-token-0123456789abcdef";

async fn me(addr: SocketAddr) -> (u16, Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "GET /admin/v1/auth/me HTTP/1.1\r\nhost: test\r\nconnection: close\r\nauthorization: Bearer {TOKEN}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

async fn policy_of(name: &str, personal_tokens: Option<Value>) -> Value {
    let dir = ScratchDir::new(name);
    let mut admin = json!({"bootstrap_token": TOKEN});
    if let Some(p) = personal_tokens {
        admin["personal_tokens"] = p;
    }
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": admin}
    });
    let file = dir.path().join("gw.json");
    std::fs::write(&file, config.to_string()).unwrap();
    let node = start(ServeOptions {
        config: Some(file),
        test_mode: true,
    })
    .await
    .unwrap();
    let addr = node.admin_addr().unwrap();
    let shutdown = node.rt.shutdown.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(node.run());
    let (status, body) = me(addr).await;
    assert_eq!(status, 200, "{body}");
    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
    body["token_policy"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn me_reports_the_personal_token_policy() {
    assert_eq!(
        policy_of("token-policy-default", None).await,
        json!({"max_lifetime": "90d", "default_lifetime": null}),
        "the default limit"
    );
    assert_eq!(
        policy_of("token-policy-set", Some(json!({"max_lifetime": "720h"}))).await,
        json!({"max_lifetime": "30d", "default_lifetime": null}),
        "the configured limit, in the gateway's duration format"
    );
}
