//! The legacy `/v1/systemone` surface: validation, and the hint that an OpenAI-only decision model
//! belongs on `/v1/decisions`.

use std::net::SocketAddr;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

const TOKEN: &str = "systemone-bootstrap-token-0123456789abcdef";

async fn call(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, Value) {
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
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn systemone_validation_and_the_decisions_hint() {
    let dir = ScratchDir::new("systemone");
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "providers": {
            "oa": {"type": "openai", "base_url": "http://127.0.0.1:9/v1", "auth": {"api_key": "sk"}}
        },
        "deployments": {"oa": {"provider": "oa", "model": "gpt-6-luna"}},
        "models": {"gpt": {"target": {"deployment": "oa"}}}
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

    let bearer = format!("Bearer {TOKEN}");
    let (status, created) = call(
        admin,
        "POST",
        "/admin/v1/keys",
        &[
            ("authorization", bearer.as_str()),
            ("content-type", "application/json"),
        ],
        &json!({"name": "test", "spec": {}}).to_string(),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let key = created["secret"].as_str().unwrap().to_owned();
    let bearer = format!("Bearer {key}");

    let valid = json!({
        "model": "gpt",
        "state": "I was charged twice.",
        "questions": {"billing": {"type": "noul", "instructions": "Is this billing?"}}
    });
    let (status, answer) = call(
        data,
        "POST",
        "/v1/systemone",
        &[
            ("authorization", bearer.as_str()),
            ("content-type", "application/json"),
        ],
        &valid.to_string(),
    )
    .await;
    assert_eq!(status, 400, "{answer}");
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("/v1/decisions")),
        "{answer}"
    );

    let invalid = json!({
        "model": "gpt",
        "state": "x",
        "questions": {"q": {"type": "choice", "instructions": "?", "criteria": {"a": "b"}}}
    });
    let (status, answer) = call(
        data,
        "POST",
        "/v1/systemone",
        &[
            ("authorization", bearer.as_str()),
            ("content-type", "application/json"),
        ],
        &invalid.to_string(),
    )
    .await;
    assert_eq!(status, 400, "{answer}");
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("criteria")),
        "{answer}"
    );

    shutdown.cancel();
}
