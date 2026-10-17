//! A route turned off with `PATCH /routes/{name}` `{spec: {enabled: false}}` serves no request: a
//! Model it serves answers 503 with reason `route_disabled`, a tree that references it goes on to
//! its next position, and turning it back on serves through it again. Routing decisions are read
//! with `routing.dry_run_all`, so nothing is sent upstream.

use std::net::SocketAddr;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

const TOKEN: &str = "route-enabled-bootstrap-token-0123456789abcdef";

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

struct Gateway {
    data: SocketAddr,
    admin: SocketAddr,
    key: String,
}

impl Gateway {
    async fn admin(&self, method: &str, path: &str, content_type: &str, body: &str) -> (u16, Value) {
        let bearer = format!("Bearer {TOKEN}");
        let headers = [("authorization", bearer.as_str()), ("content-type", content_type)];
        call(self.admin, method, path, &headers, body).await
    }

    async fn set_enabled(&self, route: &str, enabled: bool) {
        let body = json!({"spec": {"enabled": enabled}}).to_string();
        let (status, answer) = self
            .admin(
                "PATCH",
                &format!("/admin/v1/routes/{route}"),
                "application/merge-patch+json",
                &body,
            )
            .await;
        assert_eq!(status, 200, "{answer}");
        assert_eq!(answer["spec"]["enabled"], enabled, "{answer}");
    }

    /// The routing decision for `model`, or the error.
    async fn decide(&self, model: &str) -> (u16, Value) {
        let bearer = format!("Bearer {}", self.key);
        let body = json!({"model": model, "messages": [{"role": "user", "content": "hello"}]}).to_string();
        let headers = [
            ("authorization", bearer.as_str()),
            ("content-type", "application/json"),
        ];
        call(self.data, "POST", "/v1/chat/completions", &headers, &body).await
    }

    async fn deployments(&self, model: &str) -> Vec<String> {
        let (status, decision) = self.decide(model).await;
        assert_eq!(status, 200, "{model}: {decision}");
        decision["plan"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["deployment"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn refused(&self, model: &str) {
        let (status, error) = self.decide(model).await;
        assert_eq!(status, 503, "{model}: {error}");
        assert_eq!(error["antirion"]["reason"], "route_disabled", "{model}: {error}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disabled_route_serves_nothing_until_it_is_enabled_again() {
    let dir = ScratchDir::new("route-enabled");
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "providers": {"p": {"type": "openai", "base_url": "http://127.0.0.1:9/v1", "auth": {"api_key": "sk-up"}}},
        "deployments": {
            "a": {"provider": "p", "model": "up-a"},
            "b": {"provider": "p", "model": "up-b"}
        },
        "routes": {
            "primary": {"tree": "a"},
            "chain": {"tree": {"type": "fallback", "children": [{"route": "primary"}, "b"]}},
            "only": {"tree": {"type": "fallback", "children": [{"route": "primary"}]}}
        },
        "models": {
            "served": {"route": "primary"},
            "via": {"route": "chain"},
            "stuck": {"route": "only"}
        },
        "routing": {"dry_run_all": true}
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

    let mut gw = Gateway {
        data,
        admin,
        key: String::new(),
    };
    let (status, created) = gw
        .admin(
            "POST",
            "/admin/v1/keys",
            "application/json",
            r#"{"name":"k","spec":{}}"#,
        )
        .await;
    assert_eq!(status, 201, "{created}");
    gw.key = created["secret"].as_str().unwrap().to_owned();

    assert_eq!(gw.deployments("served").await, ["a"]);
    assert_eq!(gw.deployments("via").await, ["a", "b"]);
    assert_eq!(gw.deployments("stuck").await, ["a"]);

    gw.set_enabled("primary", false).await;
    gw.refused("served").await;
    assert_eq!(
        gw.deployments("via").await,
        ["b"],
        "a fallback goes on past the disabled route"
    );
    gw.refused("stuck").await;
    let (status, list) = call(
        data,
        "GET",
        "/v1/models",
        &[("authorization", &format!("Bearer {}", gw.key))],
        "",
    )
    .await;
    assert_eq!(status, 200, "{list}");
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "served"),
        "the Model stays listed: {list}"
    );

    gw.set_enabled("primary", true).await;
    assert_eq!(gw.deployments("served").await, ["a"]);
    assert_eq!(gw.deployments("via").await, ["a", "b"]);

    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
}
