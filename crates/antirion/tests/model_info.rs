//! `GET /v1/model/info`: the model metadata shaped after LiteLLM's endpoint, aggregated over the
//! deployments a key's Models can reach. No request goes upstream.

use std::net::SocketAddr;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

const TOKEN: &str = "model-info-bootstrap-token-0123456789abcdef";

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
}

impl Gateway {
    async fn admin(&self, method: &str, path: &str, body: &str) -> (u16, Value) {
        let bearer = format!("Bearer {TOKEN}");
        let headers = [
            ("authorization", bearer.as_str()),
            ("content-type", "application/json"),
        ];
        call(self.admin, method, path, &headers, body).await
    }

    /// A data-plane GET with `key`.
    async fn get(&self, path: &str, key: &str) -> (u16, Value) {
        let bearer = format!("Bearer {key}");
        call(self.data, "GET", path, &[("authorization", bearer.as_str())], "").await
    }

    /// A new data key, with `spec` as its body.
    async fn key(&self, name: &str, spec: Value) -> String {
        let body = json!({"name": name, "spec": spec}).to_string();
        let (status, created) = self.admin("POST", "/admin/v1/keys", &body).await;
        assert_eq!(status, 201, "{created}");
        created["secret"].as_str().unwrap().to_owned()
    }
}

fn entry<'a>(info: &'a Value, name: &str) -> &'a Value {
    &info["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["model_name"] == name)
        .unwrap_or_else(|| panic!("{name} is missing from {info}"))["model_info"]
}

fn name_has(info: &Value, name: &str) -> bool {
    info["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["model_name"] == name)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_info_aggregates_the_deployments_a_key_can_reach() {
    let dir = ScratchDir::new("model-info");
    let config = json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "providers": {
            "p": {
                "type": "openai_compatible",
                "base_url": "http://127.0.0.1:9/v1",
                "auth": {"api_key": "sk-up"}
            },
            "other": {
                "type": "openai_compatible",
                "base_url": "http://127.0.0.1:9/v1",
                "auth": {"api_key": "sk-up"}
            }
        },
        "deployments": {
            "wide": {
                "provider": "p",
                "model": "wide-model",
                "capabilities": {
                    "context_window": 200000,
                    "max_output_tokens": 8192,
                    "vision": true,
                    "tools": true,
                    "reasoning": true,
                    "operations": ["generate", "embed"]
                }
            },
            "narrow": {
                "provider": "p",
                "model": "narrow-model",
                "capabilities": {
                    "context_window": 8192,
                    "max_output_tokens": 2048,
                    "vision": false,
                    "tools": true,
                    "reasoning": false,
                    "operations": ["generate"]
                }
            },
            "embedder": {
                "provider": "p",
                "model": "embedder-model",
                "capabilities": {
                    "context_window": 4096,
                    "max_output_tokens": 512,
                    "vision": false,
                    "tools": false,
                    "reasoning": false,
                    "operations": ["embed"]
                }
            },
            "other-wide": {
                "provider": "other",
                "model": "other-wide-model",
                "capabilities": {
                    "context_window": 1000,
                    "max_output_tokens": 100,
                    "vision": true,
                    "tools": true,
                    "reasoning": true,
                    "operations": ["generate"]
                }
            },
            "disabled-wide": {
                "provider": "p",
                "model": "disabled-model",
                "enabled": false,
                "capabilities": {
                    "context_window": 500,
                    "max_output_tokens": 50,
                    "vision": true,
                    "tools": true,
                    "reasoning": true,
                    "operations": ["generate"]
                }
            },
            "bare": {"provider": "p", "model": "bare-model"}
        },
        "routes": {
            "mixed": {
                "tree": {
                    "type": "fallback",
                    "children": ["wide", "narrow", "other-wide", "disabled-wide"]
                }
            },
            "vectors": {"tree": "embedder"},
            "direct": {
                "tree": {"type": "provider_model", "provider": "p", "upstream_model": "gpt-4o"}
            },
            "bare": {"tree": "bare"},
            "partial": {"tree": {"type": "fallback", "children": ["wide", "bare"]}},
            "off": {"tree": "disabled-wide"},
            "other-only": {"tree": "other-wide"}
        },
        "models": {
            "chat": {"route": "mixed", "aliases": {"chat-fast": {"description": "the fast name"}}},
            "vectors": {"route": "vectors"},
            "direct": {"route": "direct"},
            "bare": {"route": "bare"},
            "partial": {"route": "partial"},
            "off": {"route": "off"},
            "other-only": {"route": "other-only"},
            "hidden": {"route": "mixed"}
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

    let gw = Gateway { data, admin };
    let key = gw
        .key(
            "restricted",
            json!({
                "models": ["chat", "chat-fast", "vectors", "direct", "bare", "partial", "off", "other-only"],
                "providers": ["p"]
            }),
        )
        .await;

    let (status, info) = gw.get("/v1/model/info", &key).await;
    assert_eq!(status, 200, "{info}");
    let names: Vec<&str> = info["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["model_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["bare", "chat", "chat-fast", "direct", "partial", "vectors"],
        "{info}"
    );
    assert!(!name_has(&info, "hidden"), "the key's model list filters: {info}");
    assert!(
        !name_has(&info, "other-only"),
        "the key's provider list filters: {info}"
    );
    assert!(
        !name_has(&info, "off"),
        "a Model with no enabled deployment is not described: {info}"
    );

    let (status, list) = gw.get("/v1/models", &key).await;
    assert_eq!(status, 200, "{list}");
    let listed: std::collections::BTreeSet<&str> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    let expected: std::collections::BTreeSet<&str> = names.iter().copied().chain(["off"]).collect();
    assert_eq!(listed, expected, "{list}");

    let chat = entry(&info, "chat");
    assert_eq!(
        chat["max_input_tokens"], 8192,
        "the smallest input bound; the other provider and the disabled deployment are excluded: {chat}"
    );
    assert_eq!(
        chat["max_output_tokens"], 2048,
        "the smallest output bound: {chat}"
    );
    assert_eq!(chat["supports_vision"], true, "{chat}");
    assert_eq!(chat["supports_function_calling"], true, "{chat}");
    assert_eq!(chat["supports_reasoning"], true, "{chat}");
    assert_eq!(chat["mode"], "chat", "{chat}");
    assert_eq!(
        entry(&info, "chat-fast"),
        chat,
        "an alias carries its Model's metadata"
    );

    let vectors = entry(&info, "vectors");
    assert_eq!(vectors["max_input_tokens"], 4096, "{vectors}");
    assert_eq!(vectors["max_output_tokens"], 512, "{vectors}");
    assert_eq!(vectors["supports_vision"], false, "{vectors}");
    assert_eq!(vectors["supports_function_calling"], false, "{vectors}");
    assert_eq!(vectors["supports_reasoning"], false, "{vectors}");
    assert_eq!(vectors["mode"], "embedding", "{vectors}");

    let direct = entry(&info, "direct");
    assert_eq!(
        direct["max_input_tokens"], 128000,
        "a provider/model leaf reads models.dev: {direct}"
    );
    assert_eq!(direct["max_output_tokens"], 16384, "{direct}");
    assert_eq!(
        direct["supports_vision"], true,
        "models.dev's input modalities decide vision: {direct}"
    );
    assert_eq!(
        direct["supports_function_calling"], false,
        "a type default is not a capability claim: {direct}"
    );
    assert_eq!(direct["supports_reasoning"], false, "{direct}");
    assert_eq!(direct["mode"], "chat", "{direct}");

    let bare = entry(&info, "bare");
    assert!(
        bare.get("max_input_tokens").is_none() && bare.get("max_output_tokens").is_none(),
        "unknown limits are omitted: {bare}"
    );
    assert_eq!(bare["supports_vision"], false, "{bare}");
    assert_eq!(bare["supports_function_calling"], false, "{bare}");
    assert_eq!(bare["supports_reasoning"], false, "{bare}");
    assert_eq!(bare["mode"], "chat", "{bare}");

    let partial = entry(&info, "partial");
    assert!(
        partial.get("max_input_tokens").is_none() && partial.get("max_output_tokens").is_none(),
        "a bound is reported only when every deployment states it: {partial}"
    );
    assert_eq!(partial["supports_vision"], true, "{partial}");
    assert_eq!(partial["supports_function_calling"], true, "{partial}");
    assert_eq!(partial["supports_reasoning"], true, "{partial}");
    assert_eq!(partial["mode"], "chat", "{partial}");

    let (status, _) = call(
        data,
        "POST",
        "/v1/model/info",
        &[("authorization", &format!("Bearer {key}"))],
        "{}",
    )
    .await;
    assert_eq!(status, 405, "the endpoint is read-only");

    let (status, same) = gw.get("/v1/model/info?litellm_model_id=anything", &key).await;
    assert_eq!(status, 200, "{same}");
    assert_eq!(
        same["data"].as_array().unwrap().len(),
        names.len(),
        "a LiteLLM parameter the endpoint does not use is ignored: {same}"
    );

    let all = gw.key("all", json!({})).await;
    let (status, info) = gw.get("/v1/model/info", &all).await;
    assert_eq!(status, 200, "{info}");
    assert!(name_has(&info, "hidden"), "an unrestricted key sees it: {info}");
    assert!(
        name_has(&info, "other-only"),
        "an unrestricted key sees the other provider: {info}"
    );
    assert!(
        !name_has(&info, "off"),
        "a disabled-only Model is not described: {info}"
    );
    let other = entry(&info, "other-only");
    assert_eq!(other["max_input_tokens"], 1000, "{other}");
    assert_eq!(other["max_output_tokens"], 100, "{other}");
    assert_eq!(other["supports_vision"], true, "{other}");
    assert_eq!(other["mode"], "chat", "{other}");

    shutdown.cancel();
    while tasks.join_next().await.is_some() {}
}
