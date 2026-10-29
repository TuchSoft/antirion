//! What the gateway does with the upstream sessions of a client that went away: after
//! `mcp.session.idle_timeout` without a request the session is ended with `DELETE`, a session with a
//! request in flight or a client's server-to-client stream attached is left alone, and the client
//! that comes back afterwards is served on a session the gateway opens again.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

const TOKEN: &str = "session-reaper-bootstrap-token-0123456789";
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

fn event(msg: &Value) -> String {
    format!("event: message\ndata: {msg}\n\n")
}

/// A mock MCP upstream over Streamable HTTP: it keeps the sessions it still knows and counts the
/// `DELETE`s it is sent.
#[allow(missing_debug_implementations)]
struct Upstream {
    live: Mutex<HashSet<String>>,
    initializes: AtomicUsize,
    /// The upstream sessions ended with `DELETE`.
    deletes: AtomicUsize,
    /// A `tools/call` the mock answers only after a while, so a request stays in flight.
    slow_calls: AtomicUsize,
}

impl Upstream {
    fn new() -> Arc<Upstream> {
        Arc::new(Upstream {
            live: Mutex::new(HashSet::new()),
            initializes: AtomicUsize::new(0),
            deletes: AtomicUsize::new(0),
            slow_calls: AtomicUsize::new(0),
        })
    }

    fn deletes(&self) -> usize {
        self.deletes.load(Ordering::SeqCst)
    }

    fn live(&self) -> usize {
        self.live.lock().unwrap().len()
    }
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
                // The server-to-client stream: held open until the client goes away.
                let Some(sid) = head_header(&head, "mcp-session-id") else {
                    let _ = stream.write_all(GONE).await;
                    return;
                };
                if !up.live.lock().unwrap().contains(&sid) {
                    let _ = stream.write_all(GONE).await;
                    return;
                }
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\n\
                          transfer-encoding: chunked\r\nconnection: close\r\n\r\n",
                    )
                    .await;
                let mut buf = [0u8; 1024];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                }
                return;
            }
            if head.starts_with("DELETE") {
                if let Some(sid) = head_header(&head, "mcp-session-id")
                    && up.live.lock().unwrap().remove(&sid)
                {
                    up.deletes.fetch_add(1, Ordering::SeqCst);
                }
                let _ = stream
                    .write_all(b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
                return;
            }
            let msg: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let rpc = msg.get("method").and_then(Value::as_str).unwrap_or_default();
            if rpc.is_empty() {
                let _ = stream.write_all(ACCEPTED).await;
                return;
            }
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            if rpc == "initialize" {
                let n = up.initializes.fetch_add(1, Ordering::SeqCst) + 1;
                let sid = format!("s-{n}");
                up.live.lock().unwrap().insert(sid.clone());
                let result = json!({"jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": LEGACY,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "solo", "version": "1"}}});
                let _ = stream
                    .write_all(reply("200 OK", &[("mcp-session-id", &sid)], &result.to_string()).as_bytes())
                    .await;
                return;
            }
            if rpc == "notifications/initialized" {
                let _ = stream.write_all(ACCEPTED).await;
                return;
            }
            let sid = head_header(&head, "mcp-session-id");
            if let Some(sid) = &sid
                && !up.live.lock().unwrap().contains(sid)
            {
                let _ = stream.write_all(GONE).await;
                return;
            }
            match rpc {
                "tools/list" => {
                    let tools = json!([{"name": "echo", "description": "Echoes",
                                        "inputSchema": {"type": "object", "properties": {}}}]);
                    let result = json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}});
                    let _ = stream
                        .write_all(reply("200 OK", &[], &result.to_string()).as_bytes())
                        .await;
                }
                "tools/call" => {
                    if up.slow_calls.load(Ordering::SeqCst) > 0 {
                        up.slow_calls.fetch_sub(1, Ordering::SeqCst);
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
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        let result = json!({"jsonrpc": "2.0", "id": id, "result": {
                            "content": [{"type": "text", "text": "slow"}], "isError": false}});
                        let data = event(&result);
                        let _ = stream
                            .write_all(format!("{:x}\r\n{data}\r\n0\r\n\r\n", data.len()).as_bytes())
                            .await;
                        return;
                    }
                    let result = json!({"jsonrpc": "2.0", "id": id, "result": {
                        "content": [{"type": "text", "text": "called"}], "isError": false}});
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

/// A running node over one mock upstream, with a one-second idle timeout.
#[allow(missing_debug_implementations)]
struct Node {
    data: SocketAddr,
    key: String,
    _dir: ScratchDir,
    halt: Option<Box<dyn Fn() + Send + Sync>>,
    tasks: JoinSet<()>,
    mocks: JoinSet<()>,
}

/// The idle timeout every test runs with: short enough to observe, long enough not to race a
/// request.
const IDLE: Duration = Duration::from_secs(1);

impl Node {
    async fn start(up: Arc<Upstream>) -> Node {
        let dir = ScratchDir::new("mcp-session-reaper");
        let mut mocks = JoinSet::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        mocks.spawn(upstream(listener, Arc::clone(&up)));
        let config = json!({
            "version": 1,
            "server": {"listen": "127.0.0.1:0"},
            "admin": {"listen": "127.0.0.1:0"},
            "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
            "auth": {"admin": {"bootstrap_token": TOKEN}},
            "security": {"egress": {"operator": {"allow_cidrs": ["127.0.0.0/8"]}}},
            "mcp": {
                "session": {"idle_timeout": "1s"},
                "servers": {"solo": {"transport": "streamable_http",
                                     "url": format!("http://{addr}/mcp")}},
                "gateways": {"g": {"servers": ["solo"], "prefix_mode": "always"}}
            },
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
        let halt: Box<dyn Fn() + Send + Sync> = Box::new(move || shutdown.cancel());
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

    /// A legacy session: its client-facing id.
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

    async fn call(&self, session: &str) -> (u16, String) {
        let (status, _, body) = self
            .post(
                session,
                &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                        "params": {"name": "solo__echo", "arguments": {}}}),
            )
            .await;
        (status, body)
    }

    /// Opens the client's server-to-client stream and returns the connection once its head arrived.
    async fn open_client_stream(&self, session: &str) -> TcpStream {
        let mut stream = TcpStream::connect(self.data).await.unwrap();
        let request = format!(
            "GET /mcp/g HTTP/1.1\r\nhost: test\r\naccept: text/event-stream\r\n\
             authorization: Bearer {}\r\nmcp-session-id: {session}\r\n\
             mcp-protocol-version: {LEGACY}\r\n\r\n",
            self.key
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut byte))
                .await
                .expect("the stream head in time")
                .unwrap();
            assert_eq!(n, 1, "the stream head");
            head.push(byte[0]);
        }
        stream
    }

    async fn stop(mut self) {
        if let Some(halt) = self.halt.take() {
            halt();
        }
        while self.tasks.join_next().await.is_some() {}
        self.mocks.abort_all();
    }
}

/// Waits until `check` holds, or fails after `limit`.
async fn until(limit: Duration, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("a condition did not hold within {limit:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abandoned_session_is_ended_after_the_idle_timeout() {
    let up = Upstream::new();
    let node = Node::start(Arc::clone(&up)).await;
    let _session = node.initialize().await;
    assert_eq!(up.live(), 1, "the upstream session is open");
    // Nothing follows: no request, no `DELETE`, no stream.
    until(Duration::from_secs(5), || up.deletes() == 1).await;
    assert_eq!(up.live(), 0, "the server ended the session");
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_with_a_request_in_flight_is_not_ended() {
    let up = Upstream::new();
    let node = Node::start(Arc::clone(&up)).await;
    let session = node.initialize().await;
    // A call that takes three seconds: longer than the idle timeout, but not idle.
    up.slow_calls.store(1, Ordering::SeqCst);
    let watching = async {
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(up.deletes(), 0, "the session was in use");
    };
    let ((status, body), ()) = tokio::join!(node.call(&session), watching);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("slow"), "{body}");
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_with_a_client_stream_attached_is_not_ended() {
    let up = Upstream::new();
    let node = Node::start(Arc::clone(&up)).await;
    let session = node.initialize().await;
    let _client = node.open_client_stream(&session).await;
    // More than two idle timeouts with the client's stream attached and no request at all.
    tokio::time::sleep(IDLE * 2 + Duration::from_millis(500)).await;
    assert_eq!(up.deletes(), 0, "the client's stream keeps the session alive");
    assert_eq!(up.live(), 1);
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_comes_back_after_the_reap_is_served_again() {
    let up = Upstream::new();
    let node = Node::start(Arc::clone(&up)).await;
    let session = node.initialize().await;
    assert_eq!(up.initializes.load(Ordering::SeqCst), 1);
    until(Duration::from_secs(5), || up.deletes() == 1).await;

    // The same client session, its upstream session gone: the gateway opens one again and the call
    // is answered.
    let (status, body) = node.call(&session).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("called"), "{body}");
    assert!(
        up.initializes.load(Ordering::SeqCst) > 1,
        "the upstream session was opened again"
    );
    assert_eq!(up.live(), 1);
    node.stop().await;
}
