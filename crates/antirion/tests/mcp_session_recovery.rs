//! What the gateway does on its own when a server talks to it or forgets it: it answers the `ping` a
//! server sends while it relays that server's answer, it re-opens an upstream session the server has
//! forgotten and retries the request once (for lists and for calls), a server whose session is gone
//! degrades a fan-out instead of failing it, and a `ping` from the client is answered.
//!
//! The mock upstreams answer `initialize`, `tools/list` and `tools/call` over Streamable HTTP, keep a
//! set of live sessions, and can forget them on demand.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "mcp-session-recovery-bootstrap-token-0123456789";
const LEGACY: &str = "2025-06-18";
const ACCEPT: &str = "application/json, text/event-stream";
const ACCEPTED: &[u8] = b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
const GONE: &[u8] = b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

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
                return Some((head.to_owned(), body[..len].to_owned()));
            }
        }
    }
}

fn head_header(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(name)).then(|| v.trim().to_owned())
    })
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

/// A mock MCP upstream over Streamable HTTP.
#[allow(missing_debug_implementations)]
struct Upstream {
    name: &'static str,
    /// The sessions it still knows.
    live: Mutex<HashSet<String>>,
    initializes: AtomicUsize,
    /// Every `tools/list` it was asked for, answered or refused.
    list_requests: AtomicUsize,
    /// The `tools/list` answers it sent.
    lists: AtomicUsize,
    calls: AtomicUsize,
    /// Answers it posted back to the gateway (the gateway's answer to a `ping`).
    ping_answers: AtomicUsize,
    /// How many times it forgets every session before the next request is answered.
    forget: AtomicUsize,
    /// Whether every request carrying a session is answered as a session it does not know.
    always_forget: AtomicBool,
    /// Whether the next `tools/call` pings the client before answering.
    ping_on_call: AtomicBool,
}

impl Upstream {
    fn new(name: &'static str) -> Arc<Upstream> {
        Arc::new(Upstream {
            name,
            live: Mutex::new(HashSet::new()),
            initializes: AtomicUsize::new(0),
            list_requests: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            ping_answers: AtomicUsize::new(0),
            forget: AtomicUsize::new(0),
            always_forget: AtomicBool::new(false),
            ping_on_call: AtomicBool::new(false),
        })
    }

    fn tool(&self) -> String {
        format!("echo_{}", self.name)
    }

    fn tools(&self) -> Value {
        json!([{"name": self.tool(), "description": "Echoes its name",
                "inputSchema": {"type": "object", "properties": {}}}])
    }
}

/// Writes one chunk of a chunked body.
async fn chunk(stream: &mut TcpStream, data: &str) {
    let _ = stream
        .write_all(format!("{:x}\r\n{data}\r\n", data.len()).as_bytes())
        .await;
}

async fn upstream(listener: TcpListener, up: Arc<Upstream>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let up = Arc::clone(&up);
        connections.spawn(async move {
            let Some((head, body)) = read_request(&mut stream).await else {
                return;
            };
            if head.starts_with("GET") {
                // This upstream offers no server-to-client stream, so the gateway keeps none.
                let _ = stream
                    .write_all(b"HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
                return;
            }
            if head.starts_with("DELETE") {
                if let Some(sid) = head_header(&head, "mcp-session-id") {
                    up.live.lock().unwrap().remove(&sid);
                }
                let _ = stream
                    .write_all(b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
                return;
            }
            let msg: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let rpc = msg.get("method").and_then(Value::as_str).unwrap_or_default();
            if rpc.is_empty() {
                // A message the gateway posted back: the answer to a ping.
                up.ping_answers.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(ACCEPTED).await;
                return;
            }
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            if rpc == "initialize" {
                let n = up.initializes.fetch_add(1, Ordering::SeqCst) + 1;
                let sid = format!("{}-{n}", up.name);
                up.live.lock().unwrap().insert(sid.clone());
                let result = json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": LEGACY,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": up.name, "version": "1"}}});
                let _ = stream
                    .write_all(reply("200 OK", &[("mcp-session-id", &sid)], &result.to_string()).as_bytes())
                    .await;
                return;
            }
            if rpc == "notifications/initialized" {
                let _ = stream.write_all(ACCEPTED).await;
                return;
            }
            if rpc == "tools/list" {
                up.list_requests.fetch_add(1, Ordering::SeqCst);
            }
            let sid = head_header(&head, "mcp-session-id");
            let mut gone = up.always_forget.load(Ordering::SeqCst);
            if !gone && up.forget.load(Ordering::SeqCst) > 0 {
                up.forget.fetch_sub(1, Ordering::SeqCst);
                up.live.lock().unwrap().clear();
                gone = true;
            }
            if !gone && let Some(sid) = &sid {
                let known = up.live.lock().unwrap().contains(sid);
                gone = !known;
            }
            if gone {
                let _ = stream.write_all(GONE).await;
                return;
            }
            match rpc {
                "tools/list" => {
                    up.lists.fetch_add(1, Ordering::SeqCst);
                    let result = json!({"jsonrpc": "2.0", "id": id, "result": {"tools": up.tools()}});
                    let _ = stream
                        .write_all(reply("200 OK", &[], &result.to_string()).as_bytes())
                        .await;
                }
                "tools/call" => {
                    up.calls.fetch_add(1, Ordering::SeqCst);
                    let name = msg["params"]["name"].as_str().unwrap_or_default().to_owned();
                    let result = json!({"jsonrpc": "2.0", "id": id, "result": {
                        "content": [{"type": "text", "text": format!("called {name} on {}", up.name)}],
                        "isError": false}});
                    if up.ping_on_call.swap(false, Ordering::SeqCst) {
                        let _ = stream
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                                     mcp-session-id: {}\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
                                    sid.as_deref().unwrap_or("none")
                                )
                                .as_bytes(),
                            )
                            .await;
                        let ping = json!({"jsonrpc": "2.0", "id": "srv-1", "method": "ping"});
                        chunk(&mut stream, &format!("event: message\ndata: {ping}\n\n")).await;
                        // The gateway must answer the ping before the result is written.
                        for _ in 0..300 {
                            if up.ping_answers.load(Ordering::SeqCst) > 0 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        chunk(&mut stream, &format!("event: message\ndata: {result}\n\n")).await;
                        let _ = stream.write_all(b"0\r\n\r\n").await;
                        return;
                    }
                    let _ = stream
                        .write_all(reply("200 OK", &[], &result.to_string()).as_bytes())
                        .await;
                }
                _ => {
                    let result = json!({"jsonrpc": "2.0", "id": id, "error": {
                        "code": -32601, "message": "method not found"}});
                    let _ = stream
                        .write_all(reply("200 OK", &[], &result.to_string()).as_bytes())
                        .await;
                }
            }
        });
    }
}

/// A running node and the mock upstreams behind it.
#[allow(missing_debug_implementations)]
struct Node {
    data: SocketAddr,
    key: String,
    /// The node's working directory, removed with it.
    _dir: ScratchDir,
    /// Stops the node's tasks.
    halt: Option<Box<dyn Fn() + Send + Sync>>,
    tasks: JoinSet<()>,
    mocks: JoinSet<()>,
}

impl Node {
    /// Starts a node whose gateway serves `upstreams`, with `prefix_mode: always`.
    async fn start(upstreams: Vec<Arc<Upstream>>, fail_closed: bool) -> Node {
        let dir = ScratchDir::new("mcp-session-recovery");
        let mut mocks = JoinSet::new();
        let mut servers = serde_json::Map::new();
        let mut names = Vec::new();
        for up in &upstreams {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            mocks.spawn(upstream(listener, Arc::clone(up)));
            servers.insert(
                up.name.to_owned(),
                json!({"transport": "streamable_http", "url": format!("http://{addr}/mcp")}),
            );
            names.push(Value::from(up.name));
        }
        let mut gateway = json!({"servers": names, "prefix_mode": "always"});
        if fail_closed {
            gateway["on_server_failure"] = Value::from("fail");
        }
        let config = json!({
            "version": 1,
            "server": {"listen": "127.0.0.1:0"},
            "admin": {"listen": "127.0.0.1:0"},
            "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
            "auth": {"admin": {"bootstrap_token": TOKEN}},
            "security": {"egress": {"operator": {"allow_cidrs": ["127.0.0.0/8"]}}},
            "mcp": {"servers": servers, "gateways": {"g": gateway}},
            "policies": {"mcp": {"servers": ["*"]}}
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
        let halt: Box<dyn Fn() + Send + Sync> = {
            let shutdown = shutdown.clone();
            Box::new(move || shutdown.cancel())
        };
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
        Node {
            data,
            key,
            _dir: dir,
            halt: Some(halt),
            tasks,
            mocks,
        }
    }

    /// A legacy session: (session id, initialize body).
    async fn initialize(&self) -> String {
        let auth = format!("Bearer {}", self.key);
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": LEGACY, "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}});
        let (status, headers, body) = exchange(
            self.data,
            "POST",
            "/mcp/g",
            &[("authorization", auth.as_str()), ("accept", ACCEPT)],
            &init.to_string(),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        header(&headers, "mcp-session-id")
            .expect("a session id")
            .to_owned()
    }

    /// A request on an existing session.
    async fn post(&self, session: &str, body: &Value) -> (u16, Vec<(String, String)>, String) {
        let auth = format!("Bearer {}", self.key);
        exchange(
            self.data,
            "POST",
            "/mcp/g",
            &[
                ("authorization", auth.as_str()),
                ("accept", ACCEPT),
                ("mcp-session-id", session),
                ("mcp-protocol-version", LEGACY),
            ],
            &body.to_string(),
        )
        .await
    }

    async fn stop(mut self) {
        if let Some(halt) = self.halt.take() {
            halt();
        }
        while self.tasks.join_next().await.is_some() {}
        self.mocks.abort_all();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ping_from_the_client_is_answered() {
    let up = Upstream::new("solo");
    let node = Node::start(vec![Arc::clone(&up)], false).await;
    let session = node.initialize().await;
    let (status, _, body) = node
        .post(&session, &json!({"jsonrpc": "2.0", "id": 7, "method": "ping"}))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"jsonrpc": "2.0", "id": 7, "result": {}})
    );
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ping_a_server_sends_while_it_relays_is_answered_by_the_gateway() {
    let up = Upstream::new("solo");
    up.ping_on_call.store(true, Ordering::SeqCst);
    let node = Node::start(vec![Arc::clone(&up)], false).await;
    let session = node.initialize().await;
    let (status, _, body) = node
        .post(
            &session,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {"name": "solo__echo_solo", "arguments": {}}}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("called echo_solo on solo"), "{body}");
    assert_eq!(up.ping_answers.load(Ordering::SeqCst), 1, "{body}");
    assert!(
        !body.contains("srv-1"),
        "the ping must not reach the client: {body}"
    );
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forgotten_session_reopens_and_the_list_is_retried_once() {
    let up = Upstream::new("solo");
    let node = Node::start(vec![Arc::clone(&up)], false).await;
    let session = node.initialize().await;
    up.forget.store(1, Ordering::SeqCst);
    let (status, _, body) = node
        .post(
            &session,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let answer: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(answer["result"]["tools"][0]["name"], "solo__echo_solo", "{body}");
    assert_eq!(up.initializes.load(Ordering::SeqCst), 2, "one re-initialize");
    assert_eq!(up.list_requests.load(Ordering::SeqCst), 2, "one retried read");
    assert_eq!(
        up.lists.load(Ordering::SeqCst),
        1,
        "the retried read is the only answer"
    );
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forgotten_session_reopens_and_the_call_is_retried_once() {
    let up = Upstream::new("solo");
    let node = Node::start(vec![Arc::clone(&up)], false).await;
    let session = node.initialize().await;
    // Read the list first, so routing the call needs no upstream read and the call is the only
    // exchange that meets the forgotten session.
    let (status, _, body) = node
        .post(
            &session,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    up.forget.store(1, Ordering::SeqCst);
    let (status, _, body) = node
        .post(
            &session,
            &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": {"name": "solo__echo_solo", "arguments": {}}}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("called echo_solo on solo"), "{body}");
    assert_eq!(up.initializes.load(Ordering::SeqCst), 2, "one re-initialize");
    assert_eq!(up.calls.load(Ordering::SeqCst), 1, "the call runs once");
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_whose_session_is_gone_degrades_a_fanout() {
    let good = Upstream::new("alpha");
    let bad = Upstream::new("beta");
    bad.always_forget.store(true, Ordering::SeqCst);
    let node = Node::start(vec![Arc::clone(&good), Arc::clone(&bad)], true).await;
    let session = node.initialize().await;
    let (status, headers, body) = node
        .post(
            &session,
            &json!({"jsonrpc": "2.0", "id": 5, "method": "tools/list"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        header(&headers, "x-antirion-degraded"),
        Some("mcp_servers"),
        "{headers:?}"
    );
    assert_eq!(header(&headers, "x-antirion-mcp-unavailable"), Some("beta"));
    let answer: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        answer["result"]["tools"][0]["name"], "alpha__echo_alpha",
        "{body}"
    );
    assert_eq!(
        answer["result"]["_meta"]["antirion/unavailable_servers"],
        json!(["beta"])
    );
    assert_eq!(bad.initializes.load(Ordering::SeqCst), 2, "one re-initialize");
    node.stop().await;
}
