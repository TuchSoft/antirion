//! A semantic `tool_search` gateway: `tools/list` answers three meta-tools, `search_tools` ranks
//! the granted tools by the cosine similarity of their embeddings, a caller's `limit` is capped by
//! `top_k`, an unchanged tool is embedded once (in the node and in the store, across a restart), a
//! failing embedding model degrades to the lexical ranking with a degraded response, and the
//! caller's query never reaches the durable vector table.
//!
//! The upstream mock answers `initialize`, `tools/list` and `tools/call`, and the embeddings mock
//! maps each input text to a small vector by content, so the ranking is deterministic:
//!
//! | text contains | vector       | cosine with the query `"weather"` |
//! |---------------|--------------|-----------------------------------|
//! | `forecast`    | `[1, 0, 0]`  | 1.0                               |
//! | `alerts`      | `[0.8, 0.6, 0]` | 0.8                            |
//! | `weather`     | `[1, 0, 0]`  | 1.0                               |
//! | `file`        | `[0, 1, 0]`  | 0.0 (below the 0.3 threshold)     |
//! | otherwise     | `[0, 0, 1]`  | 0.0                               |

use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "mcp-tool-search-bootstrap-token-0123456789abcdef";
const LEGACY: &str = "2025-06-18";
const MODERN: &str = "2026-07-28";
const ACCEPT: &str = "application/json, text/event-stream";
const META_PV: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPS: &str = "io.modelcontextprotocol/clientCapabilities";

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

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// Reads one request: its head and body.
async fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut seen = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        seen.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&seen).into_owned();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let len: usize = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap_or(0))
                })
                .unwrap_or(0);
            if body.len() >= len {
                return Some((head.to_owned(), body.to_owned()));
            }
        }
    }
}

fn reply(status: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut out = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    out
}

/// The three tools the upstream offers, in the order it lists them.
fn tools() -> Value {
    json!([
        {
            "name": "weather_forecast",
            "description": "Daily weather forecast for a city",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "weather_alerts",
            "description": "Severe weather alerts for a region",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "files_read",
            "description": "Read a file from disk",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

/// The mock MCP upstream (Streamable HTTP): `initialize`, `tools/list`, `tools/call`, plus the
/// notification and `DELETE` a session ends with.
async fn mcp_upstream(listener: TcpListener) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        connections.spawn(async move {
            let Some((head, body)) = read_request(&mut stream).await else {
                return;
            };
            let method = head.split(' ').next().unwrap_or_default();
            if method == "DELETE" {
                let _ = stream
                    .write_all(reply("204 No Content", &[], "").as_bytes())
                    .await;
                return;
            }
            let msg: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            let rpc = msg.get("method").and_then(Value::as_str).unwrap_or_default();
            let text = match rpc {
                "initialize" => json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": LEGACY,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "mock", "version": "1"}}}),
                "notifications/initialized" => Value::Null,
                "tools/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools()}}),
                "tools/call" => {
                    let name = msg["params"]["name"].as_str().unwrap_or_default();
                    json!({"jsonrpc": "2.0", "id": id, "result": {
                        "content": [{"type": "text", "text": format!("called {name}")}],
                        "isError": false}})
                }
                _ => json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": -32601, "message": "method not found"}}),
            };
            if text.is_null() {
                let _ = stream
                    .write_all(
                        "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".as_bytes(),
                    )
                    .await;
                return;
            }
            let response = reply(
                "200 OK",
                &[("mcp-session-id", "mock-session-1")],
                &text.to_string(),
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

/// A small deterministic embedding of `text`, by what it says.
fn vector_of(text: &str) -> Vec<f64> {
    let t = text.to_lowercase();
    if t.contains("forecast") {
        vec![1.0, 0.0, 0.0]
    } else if t.contains("alerts") {
        vec![0.8, 0.6, 0.0]
    } else if t.contains("weather") {
        vec![1.0, 0.0, 0.0]
    } else if t.contains("file") {
        vec![0.0, 1.0, 0.0]
    } else {
        vec![0.0, 0.0, 1.0]
    }
}

/// The mock embeddings upstream: answers `/embeddings` from the input text, records every input it
/// was asked to embed, and fails on demand.
async fn embeddings_upstream(
    listener: TcpListener,
    seen: Arc<Mutex<Vec<Vec<String>>>>,
    fail: Arc<AtomicBool>,
) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let seen = Arc::clone(&seen);
        let fail = Arc::clone(&fail);
        connections.spawn(async move {
            let Some((head, body)) = read_request(&mut stream).await else {
                return;
            };
            let path = head.split(' ').nth(1).unwrap_or_default();
            if !path.ends_with("/embeddings") {
                let _ = stream
                    .write_all(reply("404 Not Found", &[], r#"{"error":{"message":"not found"}}"#).as_bytes())
                    .await;
                return;
            }
            if fail.load(Ordering::SeqCst) {
                let _ = stream
                    .write_all(
                        reply(
                            "500 Internal Server Error",
                            &[],
                            r#"{"error":{"message":"down"}}"#,
                        )
                        .as_bytes(),
                    )
                    .await;
                return;
            }
            let request: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let inputs: Vec<String> = match &request["input"] {
                Value::Array(items) => items
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default().to_owned())
                    .collect(),
                Value::String(s) => vec![s.clone()],
                _ => Vec::new(),
            };
            {
                let mut seen = seen.lock().unwrap();
                seen.push(inputs.clone());
            }
            let data: Vec<Value> = inputs
                .iter()
                .enumerate()
                .map(|(i, text)| json!({"object": "embedding", "index": i, "embedding": vector_of(text)}))
                .collect();
            let tokens = 2 * inputs.len();
            let text = json!({"object": "list", "data": data, "model": "up-embed",
                "usage": {"prompt_tokens": tokens, "total_tokens": tokens}})
            .to_string();
            let _ = stream.write_all(reply("200 OK", &[], &text).as_bytes()).await;
        });
    }
}

/// The two upstreams a test talks to.
struct Mocks {
    mcp: SocketAddr,
    embeddings: SocketAddr,
    seen: Arc<Mutex<Vec<Vec<String>>>>,
    fail: Arc<AtomicBool>,
    tasks: JoinSet<()>,
}

impl Mocks {
    async fn start() -> Mocks {
        let mcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mcp_addr = mcp.local_addr().unwrap();
        let embeddings = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let embeddings_addr = embeddings.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let fail = Arc::new(AtomicBool::new(false));
        let mut tasks = JoinSet::new();
        tasks.spawn(mcp_upstream(mcp));
        tasks.spawn(embeddings_upstream(
            embeddings,
            Arc::clone(&seen),
            Arc::clone(&fail),
        ));
        Mocks {
            mcp: mcp_addr,
            embeddings: embeddings_addr,
            seen,
            fail,
            tasks,
        }
    }

    /// Every embedding call so far, each with the texts it carried.
    fn calls(&self) -> Vec<Vec<String>> {
        self.seen.lock().unwrap().clone()
    }

    fn call_count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    fn failing(&self, on: bool) {
        self.fail.store(on, Ordering::SeqCst);
    }
}

impl Drop for Mocks {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

/// The node's configuration: a chat-less OpenAI provider for embeddings, an MCP server on the mock,
/// and a gateway whose semantic tool search uses `top_k`.
fn config(mocks: &Mocks, dir: &Path) -> Value {
    json!({
        "version": 1,
        "server": {"listen": "127.0.0.1:0"},
        "admin": {"listen": "127.0.0.1:0"},
        "store": {"embedded": {"path": dir.join("data").to_string_lossy()}},
        "auth": {"admin": {"bootstrap_token": TOKEN}},
        "security": {"egress": {"operator": {"allow_cidrs": ["127.0.0.0/8"]}}},
        "providers": {
            "oa": {"type": "openai", "base_url": format!("http://{}/v1", mocks.embeddings),
                   "auth": {"api_key": "sk-up"}}
        },
        "deployments": {
            "embed": {"provider": "oa", "model": "up-embed", "price": {"input": "1", "output": "1"}}
        },
        "models": {"embed": {"target": {"deployment": "embed"}}},
        "mcp": {
            "servers": {
                "mock": {"transport": "streamable_http", "url": format!("http://{}/mcp", mocks.mcp),
                         "protocol": "legacy"}
            },
            "gateways": {
                "g": {
                    "servers": ["mock"],
                    "modern_bridge": true,
                    "tool_search": {
                        "embedding_model": "embed",
                        "threshold": 0.3,
                        "top_k": 2,
                        "timeout": "5s"
                    }
                }
            }
        },
        "policies": {"mcp": {"servers": ["*"]}}
    })
}

/// A running node.
struct Node {
    data: SocketAddr,
    admin: SocketAddr,
    shutdown: Box<dyn FnOnce() + Send>,
    tasks: JoinSet<()>,
}

impl Node {
    async fn start(file: &Path) -> Node {
        let node = start(ServeOptions {
            config: Some(file.to_owned()),
            test_mode: true,
            no_config: None,
        })
        .await
        .unwrap();
        let (data, admin) = (node.data_addr().unwrap(), node.admin_addr().unwrap());
        let token = node.rt.shutdown.clone();
        let shutdown: Box<dyn FnOnce() + Send> = Box::new(move || token.cancel());
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let _ = node.run().await;
        });
        Node {
            data,
            admin,
            shutdown,
            tasks,
        }
    }

    async fn stop(mut self) {
        (self.shutdown)();
        while self.tasks.join_next().await.is_some() {}
    }

    /// A new key `name` with no spec of its own; its secret.
    async fn key(&self, name: &str) -> String {
        let bearer = format!("Bearer {TOKEN}");
        let body = json!({"name": name, "spec": {}}).to_string();
        let (status, _, body) = exchange(
            self.admin,
            "POST",
            "/admin/v1/keys",
            &[("authorization", bearer.as_str())],
            &body,
        )
        .await;
        assert_eq!(status, 201, "{body}");
        serde_json::from_str::<Value>(&body).unwrap()["secret"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

/// A legacy session on the gateway; its session id.
async fn open_session(node: &Node, key: &str) -> String {
    let bearer = format!("Bearer {key}");
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": LEGACY, "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}});
    let (status, headers, body) = exchange(
        node.data,
        "POST",
        "/mcp/g",
        &[
            ("authorization", bearer.as_str()),
            ("accept", ACCEPT),
            ("mcp-protocol-version", LEGACY),
        ],
        &init.to_string(),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    header(&headers, "mcp-session-id")
        .expect("a session id")
        .to_owned()
}

/// The JSON answer of a response, whether it was a JSON body or one SSE event.
fn answer(headers: &[(String, String)], body: &str) -> Value {
    if header(headers, "content-type").is_some_and(|v| v.starts_with("text/event-stream")) {
        let data = body
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or_default();
        return serde_json::from_str(data).unwrap_or(Value::Null);
    }
    serde_json::from_str(body).unwrap_or(Value::Null)
}

/// A `tools/call` in a legacy session.
async fn legacy_call(
    node: &Node,
    key: &str,
    session: &str,
    id: u64,
    method: &str,
    params: Value,
) -> (u16, Vec<(String, String)>, Value) {
    let bearer = format!("Bearer {key}");
    let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let (status, headers, text) = exchange(
        node.data,
        "POST",
        "/mcp/g",
        &[
            ("authorization", bearer.as_str()),
            ("accept", ACCEPT),
            ("mcp-protocol-version", LEGACY),
            ("mcp-session-id", session),
        ],
        &body.to_string(),
    )
    .await;
    let answer = answer(&headers, &text);
    (status, headers, answer)
}

/// A request in the modern era: no session, the routing headers and the client's `_meta`.
async fn modern_call(
    node: &Node,
    key: &str,
    id: u64,
    method: &str,
    params: Value,
) -> (u16, Vec<(String, String)>, Value) {
    let bearer = format!("Bearer {key}");
    let mut params = params;
    if let Some(o) = params.as_object_mut() {
        o.insert("_meta".into(), json!({META_PV: MODERN, META_CAPS: {}}));
    }
    let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let name = body["params"]["name"].as_str().unwrap_or_default();
    let (status, headers, text) = exchange(
        node.data,
        "POST",
        "/mcp/g",
        &[
            ("authorization", bearer.as_str()),
            ("accept", ACCEPT),
            ("mcp-protocol-version", MODERN),
            ("mcp-method", method),
            ("mcp-name", name),
        ],
        &body.to_string(),
    )
    .await;
    let answer = answer(&headers, &text);
    (status, headers, answer)
}

/// The `tools` of a `search_tools` answer.
fn hits(answer: &Value) -> Vec<Value> {
    answer["result"]["structuredContent"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn names(hits: &[Value]) -> Vec<String> {
    hits.iter()
        .map(|h| h["name"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn search_params(query: &str, limit: Option<u64>) -> Value {
    let mut arguments = json!({"query": query});
    if let Some(l) = limit {
        arguments["limit"] = json!(l);
    }
    json!({"name": "search_tools", "arguments": arguments})
}

/// The rows of `mcp_tool_vectors` in the node's store: (model, dim, vector).
fn stored_rows(dir: &Path) -> Vec<(String, i64, Vec<f64>)> {
    let db = rusqlite::Connection::open(dir.join("data").join("state.db")).expect("the store");
    let mut statement = db
        .prepare("SELECT model, dim, vector FROM mcp_tool_vectors")
        .expect("the vector table");
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .expect("the rows");
    rows.map(|r| {
        let (model, dim, body) = r.expect("a row");
        (
            model,
            dim,
            serde_json::from_str::<Vec<f64>>(&body).expect("a JSON vector"),
        )
    })
    .collect()
}

fn vector_set(rows: &[(String, i64, Vec<f64>)]) -> Vec<String> {
    let mut out: Vec<String> = rows
        .iter()
        .map(|(_, _, v)| v.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(","))
        .collect();
    out.sort();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn a_semantic_search_ranks_by_meaning_and_never_re_embeds_an_unchanged_tool() {
    let dir = ScratchDir::new("mcp-tool-search-legacy");
    let mocks = Mocks::start().await;
    let file = dir.path().join("config.json");
    std::fs::write(&file, config(&mocks, dir.path()).to_string()).unwrap();
    let node = Node::start(&file).await;
    let key = node.key("k").await;
    let session = open_session(&node, &key).await;

    // 1. `tools/list` answers exactly the three meta-tools.
    let (status, _, listed) = legacy_call(&node, &key, &session, 2, "tools/list", json!({})).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(
        names(&listed["result"]["tools"].as_array().cloned().unwrap_or_default()),
        ["search_tools", "get_tool_schema", "call_tool"]
    );

    // 2. `search_tools` returns the weather tools in rank order, each with a score, and drops the
    //    file tool below the threshold.
    let (status, _, found) = legacy_call(
        &node,
        &key,
        &session,
        3,
        "tools/call",
        search_params("weather", None),
    )
    .await;
    assert_eq!(status, 200, "{found}");
    let top = hits(&found);
    assert_eq!(names(&top), ["weather_forecast", "weather_alerts"], "{found}");
    assert!(!names(&top).iter().any(|n| n == "files_read"), "{found}");
    for hit in &top {
        assert!(
            hit["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{hit}"
        );
        assert!(hit["score"].as_f64().is_some(), "{hit}");
    }
    let scores: Vec<f64> = top.iter().filter_map(|h| h["score"].as_f64()).collect();
    assert!((scores[0] - 1.0).abs() < 1e-6, "{scores:?}");
    assert!((scores[1] - 0.8).abs() < 1e-6, "{scores:?}");

    // 3. `limit` caps the results, but never above `top_k` (2 here).
    let (_, _, no_limit) = legacy_call(
        &node,
        &key,
        &session,
        4,
        "tools/call",
        search_params("weather", None),
    )
    .await;
    assert_eq!(hits(&no_limit).len(), 2, "{no_limit}");
    let (_, _, one) = legacy_call(
        &node,
        &key,
        &session,
        5,
        "tools/call",
        search_params("weather", Some(1)),
    )
    .await;
    assert_eq!(names(&hits(&one)), ["weather_forecast"], "{one}");
    let (_, _, ten) = legacy_call(
        &node,
        &key,
        &session,
        6,
        "tools/call",
        search_params("weather", Some(10)),
    )
    .await;
    assert_eq!(hits(&ten).len(), 2, "{ten}");

    // 4. A second identical search embeds nothing at all: the node caches the tool vectors and the
    //    query vector too, so the mock sees no further call (neither tool texts nor the query).
    let calls = mocks.calls();
    assert_eq!(calls.len(), 2, "one batch of tool texts and one query: {calls:?}");
    assert_eq!(
        calls[0].len(),
        3,
        "one call embeds the three tool texts: {calls:?}"
    );
    assert_eq!(calls[1], ["weather"], "{calls:?}");
    let (_, _, again) = legacy_call(
        &node,
        &key,
        &session,
        7,
        "tools/call",
        search_params("weather", None),
    )
    .await;
    assert_eq!(names(&hits(&again)), ["weather_forecast", "weather_alerts"]);
    assert_eq!(
        mocks.call_count(),
        2,
        "the second search embedded nothing: {:?}",
        mocks.calls()
    );

    // 7. The store holds one row per tool text and nothing for a query: searching with a query the
    //    tools do not share adds no row.
    let rows = stored_rows(dir.path());
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(
        rows.iter().all(|(model, dim, _)| model == "embed" && *dim == 3),
        "{rows:?}"
    );
    assert_eq!(
        vector_set(&rows),
        ["0.000,1.000,0.000", "0.800,0.600,0.000", "1.000,0.000,0.000"],
        "{rows:?}"
    );
    let (_, _, none) = legacy_call(
        &node,
        &key,
        &session,
        8,
        "tools/call",
        search_params("storm", None),
    )
    .await;
    assert_eq!(hits(&none), Vec::<Value>::new(), "{none}");
    assert_eq!(stored_rows(dir.path()).len(), 3, "a query is never written");

    // 5. A restart on the same store embeds the query only: the rows answer for the tool texts.
    node.stop().await;
    let calls_before = mocks.call_count();
    let node = Node::start(&file).await;
    let key = node.key("k2").await;
    let session = open_session(&node, &key).await;
    let (status, _, found) = legacy_call(
        &node,
        &key,
        &session,
        9,
        "tools/call",
        search_params("weather", None),
    )
    .await;
    assert_eq!(status, 200, "{found}");
    assert_eq!(
        names(&hits(&found)),
        ["weather_forecast", "weather_alerts"],
        "the durable rows still rank: {found}"
    );
    let calls = mocks.calls();
    assert_eq!(
        calls.len(),
        calls_before + 1,
        "only the query was embedded after the restart: {calls:?}"
    );
    assert_eq!(calls.last(), Some(&vec!["weather".to_owned()]), "{calls:?}");

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_modern_client_sees_the_same_three_tools_and_the_same_ranking() {
    let dir = ScratchDir::new("mcp-tool-search-modern");
    let mocks = Mocks::start().await;
    let file = dir.path().join("config.json");
    std::fs::write(&file, config(&mocks, dir.path()).to_string()).unwrap();
    let node = Node::start(&file).await;
    let key = node.key("k").await;

    // 1. The three meta-tools, in the modern era.
    let (status, _, listed) = modern_call(&node, &key, 1, "tools/list", json!({})).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(
        names(&listed["result"]["tools"].as_array().cloned().unwrap_or_default()),
        ["search_tools", "get_tool_schema", "call_tool"]
    );

    // 2. The same ranking and the same score, in rank order.
    let (status, _, found) = modern_call(&node, &key, 2, "tools/call", search_params("weather", None)).await;
    assert_eq!(status, 200, "{found}");
    let top = hits(&found);
    assert_eq!(names(&top), ["weather_forecast", "weather_alerts"], "{found}");
    assert!(!names(&top).iter().any(|n| n == "files_read"), "{found}");
    assert!(
        top.iter()
            .all(|h| h["score"].as_f64().is_some() && h["description"].as_str().is_some())
    );

    // 3. `limit` and `top_k` interact as in the legacy era.
    let (_, _, one) = modern_call(&node, &key, 3, "tools/call", search_params("weather", Some(1))).await;
    assert_eq!(names(&hits(&one)), ["weather_forecast"], "{one}");
    let (_, _, ten) = modern_call(&node, &key, 4, "tools/call", search_params("weather", Some(10))).await;
    assert_eq!(hits(&ten).len(), 2, "{ten}");

    // 4. The node's cache answers the second search without an embedding call.
    let before = mocks.call_count();
    let (_, _, again) = modern_call(&node, &key, 5, "tools/call", search_params("weather", None)).await;
    assert_eq!(names(&hits(&again)), ["weather_forecast", "weather_alerts"]);
    assert_eq!(mocks.call_count(), before, "{:?}", mocks.calls());

    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_embedding_model_falls_back_to_the_lexical_ranking() {
    let dir = ScratchDir::new("mcp-tool-search-fallback");
    let mocks = Mocks::start().await;
    let file = dir.path().join("config.json");
    std::fs::write(&file, config(&mocks, dir.path()).to_string()).unwrap();
    let node = Node::start(&file).await;
    let key = node.key("k").await;
    let session = open_session(&node, &key).await;

    mocks.failing(true);
    let (status, headers, found) = legacy_call(
        &node,
        &key,
        &session,
        2,
        "tools/call",
        search_params("file", None),
    )
    .await;
    assert_eq!(status, 200, "{found}");
    let top = hits(&found);
    assert_eq!(
        names(&top),
        ["files_read"],
        "the lexical ranking answered: {found}"
    );
    assert!(
        top.iter().all(|h| h.get("score").is_none()),
        "a lexical answer carries no score: {found}"
    );
    let degraded = header(&headers, "x-antirion-degraded").unwrap_or_default();
    assert!(
        degraded.split(',').any(|c| c.trim() == "mcp_tool_search"),
        "the response says the search degraded: {degraded:?} {headers:?}"
    );

    // A degraded search still never writes the query's vector.
    assert!(stored_rows(dir.path()).is_empty(), "nothing was embedded");

    node.stop().await;
}
