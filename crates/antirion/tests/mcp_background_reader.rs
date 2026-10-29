//! What the gateway does with a server that talks to it while no request is in flight: it holds the
//! session's server-to-client stream itself, answers the `ping`s sent there, forgets no message the
//! server sends (a `list_changed` invalidates the cache, anything else reaches the client's stream),
//! opens one stream per session and no more, and stops the reader when the session ends.
//!
//! The mock upstream answers `initialize`, `tools/list` and `tools/call` over Streamable HTTP, keeps
//! a set of live sessions, and serves the server-to-client `GET` stream by pushing chunks: pings on
//! a timer, notifications on demand.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use antirion::startup::{ServeOptions, start};
use antirion_testkit::ScratchDir;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

const TOKEN: &str = "background-reader-bootstrap-token-0123456789";
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

/// Writes one chunk of a chunked body.
async fn chunk(stream: &mut TcpStream, data: &str) {
    let _ = stream
        .write_all(format!("{:x}\r\n{data}\r\n", data.len()).as_bytes())
        .await;
}

/// One SSE event carrying `msg`.
fn event(msg: &Value) -> String {
    format!("event: message\ndata: {msg}\n\n")
}

/// A mock MCP upstream over Streamable HTTP, with the server-to-client `GET` stream.
#[allow(missing_debug_implementations)]
struct Upstream {
    name: &'static str,
    /// The sessions it still knows.
    live: Mutex<HashSet<String>>,
    initializes: AtomicUsize,
    /// Every `tools/list` it was asked for, answered or refused.
    list_requests: AtomicUsize,
    /// Answers it received to the pings it sent (the gateway's answer to a `ping`).
    ping_answers: AtomicUsize,
    /// The `GET` streams it opened.
    gets: AtomicUsize,
    /// The `GET` streams that ended because the gateway closed them.
    get_closes: AtomicUsize,
    /// A second `GET` on a session already streaming: the gateway must never ask for one.
    extra_gets: AtomicUsize,
    /// How often it pings on the stream, when it does.
    ping_every: Mutex<Option<Duration>>,
    /// The push ends of the streams it serves, by session.
    streams: Mutex<HashMap<String, mpsc::UnboundedSender<String>>>,
    /// Whether the next `tools/call` pings the client before answering.
    ping_on_call: AtomicBool,
    /// Whether the next `tools/call` is answered as a stream of two events.
    stream_call: AtomicBool,
    /// Identifies the pings it sends.
    pings: AtomicUsize,
}

impl Upstream {
    fn new(name: &'static str) -> Arc<Upstream> {
        Arc::new(Upstream {
            name,
            live: Mutex::new(HashSet::new()),
            initializes: AtomicUsize::new(0),
            list_requests: AtomicUsize::new(0),
            ping_answers: AtomicUsize::new(0),
            gets: AtomicUsize::new(0),
            get_closes: AtomicUsize::new(0),
            extra_gets: AtomicUsize::new(0),
            ping_every: Mutex::new(None),
            streams: Mutex::new(HashMap::new()),
            ping_on_call: AtomicBool::new(false),
            stream_call: AtomicBool::new(false),
            pings: AtomicUsize::new(0),
        })
    }

    fn tool(&self) -> String {
        format!("echo_{}", self.name)
    }

    /// The upstream session whose stream the gateway holds (the mock issues one per test).
    fn streamed_session(&self) -> String {
        self.streams
            .lock()
            .unwrap()
            .keys()
            .next()
            .cloned()
            .expect("an open stream")
    }

    /// Pushes one message on the session's stream; `false` when no stream is open for it.
    fn notify(&self, sid: &str, msg: &Value) -> bool {
        let streams = self.streams.lock().unwrap();
        streams.get(sid).is_some_and(|tx| tx.send(event(msg)).is_ok())
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
                serve_stream(&mut stream, &head, &up).await;
                return;
            }
            if head.starts_with("DELETE") {
                if let Some(sid) = head_header(&head, "mcp-session-id") {
                    up.live.lock().unwrap().remove(&sid);
                    up.streams.lock().unwrap().remove(&sid);
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
            let sid = head_header(&head, "mcp-session-id");
            if let Some(sid) = &sid
                && !up.live.lock().unwrap().contains(sid)
            {
                let _ = stream.write_all(GONE).await;
                return;
            }
            match rpc {
                "tools/list" => {
                    up.list_requests.fetch_add(1, Ordering::SeqCst);
                    let tools = json!([{"name": up.tool(), "description": "Echoes its name",
                                        "inputSchema": {"type": "object", "properties": {}}}]);
                    let result = json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}});
                    let _ = stream
                        .write_all(reply("200 OK", &[], &result.to_string()).as_bytes())
                        .await;
                }
                "tools/call" => {
                    let name = msg["params"]["name"].as_str().unwrap_or_default().to_owned();
                    let result = json!({"jsonrpc": "2.0", "id": id, "result": {
                        "content": [{"type": "text", "text": format!("called {name} on {}", up.name)}],
                        "isError": false}});
                    if up.stream_call.swap(false, Ordering::SeqCst) {
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
                        chunk(
                            &mut stream,
                            &event(&json!({"jsonrpc": "2.0", "method": "notifications/progress",
                                           "params": {"progressToken": "p1", "progress": 1}})),
                        )
                        .await;
                        // The call answers slowly: the session reader must keep answering pings
                        // meanwhile without touching this stream.
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        chunk(&mut stream, &event(&result)).await;
                        let _ = stream.write_all(b"0\r\n\r\n").await;
                        return;
                    }
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
                        chunk(&mut stream, &event(&ping)).await;
                        for _ in 0..300 {
                            if up.ping_answers.load(Ordering::SeqCst) > 0 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        chunk(&mut stream, &event(&result)).await;
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

/// Serves one server-to-client stream until the gateway closes it.
async fn serve_stream(stream: &mut TcpStream, head: &str, up: &Upstream) {
    let Some(sid) = head_header(head, "mcp-session-id") else {
        let _ = stream.write_all(GONE).await;
        return;
    };
    if !up.live.lock().unwrap().contains(&sid) {
        let _ = stream.write_all(GONE).await;
        return;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let registered = {
        let mut streams = up.streams.lock().unwrap();
        if streams.contains_key(&sid) {
            false
        } else {
            streams.insert(sid.clone(), tx);
            true
        }
    };
    if !registered {
        up.extra_gets.fetch_add(1, Ordering::SeqCst);
        let _ = stream
            .write_all(b"HTTP/1.1 409 Conflict\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await;
        return;
    }
    up.gets.fetch_add(1, Ordering::SeqCst);
    let _ = stream
        .write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\n\
              transfer-encoding: chunked\r\nconnection: close\r\n\r\n",
        )
        .await;
    let every = *up.ping_every.lock().unwrap();
    loop {
        let next = match every {
            Some(every) => tokio::select! {
                next = rx.recv() => next,
                () = tokio::time::sleep(every) => {
                    let n = up.pings.fetch_add(1, Ordering::SeqCst) + 1;
                    chunk(
                        stream,
                        &event(&json!({"jsonrpc": "2.0", "id": format!("srv-{n}"), "method": "ping"})),
                    )
                    .await;
                    continue;
                }
            },
            None => rx.recv().await,
        };
        match next {
            Some(text) => chunk(stream, &text).await,
            None => break,
        }
    }
    up.streams.lock().unwrap().remove(&sid);
    up.get_closes.fetch_add(1, Ordering::SeqCst);
    let _ = stream.write_all(b"0\r\n\r\n").await;
}

/// A running node and the mock upstreams behind it.
#[allow(missing_debug_implementations)]
struct Node {
    data: SocketAddr,
    key: String,
    _dir: ScratchDir,
    halt: Option<Box<dyn Fn() + Send + Sync>>,
    tasks: JoinSet<()>,
    mocks: JoinSet<()>,
}

impl Node {
    async fn start(upstreams: Vec<Arc<Upstream>>) -> Node {
        let dir = ScratchDir::new("mcp-background-reader");
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
        let config = json!({
            "version": 1,
            "server": {"listen": "127.0.0.1:0"},
            "admin": {"listen": "127.0.0.1:0"},
            "store": {"embedded": {"path": dir.path().join("data").to_string_lossy()}},
            "auth": {"admin": {"bootstrap_token": TOKEN}},
            "security": {"egress": {"operator": {"allow_cidrs": ["127.0.0.0/8"]}}},
            "mcp": {"servers": servers, "gateways": {"g": {"servers": names, "prefix_mode": "always"}}},
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

    /// A legacy session: its id.
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
async fn an_idle_link_answers_the_pings_its_server_sends() {
    let up = Upstream::new("solo");
    *up.ping_every.lock().unwrap() = Some(Duration::from_millis(100));
    let node = Node::start(vec![Arc::clone(&up)]).await;
    let _session = node.initialize().await;
    until(Duration::from_secs(5), || up.gets.load(Ordering::SeqCst) == 1).await;
    // No request is in flight: only the session's reader can answer.
    until(Duration::from_secs(5), || {
        up.ping_answers.load(Ordering::SeqCst) >= 2
    })
    .await;
    assert_eq!(up.extra_gets.load(Ordering::SeqCst), 0, "one stream per session");
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notification_reaches_the_clients_stream_and_a_list_change_is_consumed() {
    let up = Upstream::new("solo");
    let node = Node::start(vec![Arc::clone(&up)]).await;
    let session = node.initialize().await;
    until(Duration::from_secs(5), || up.gets.load(Ordering::SeqCst) == 1).await;

    // Fill the list cache, then change it upstream while nothing is in flight.
    let (status, _, body) = node
        .post(
            &session,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(up.list_requests.load(Ordering::SeqCst), 1);
    let upstream_session = up.streamed_session();
    assert!(up.notify(
        &upstream_session,
        &json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (status, _, body) = node
        .post(
            &session,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        up.list_requests.load(Ordering::SeqCst),
        2,
        "the notification invalidated the list even with no request in flight"
    );

    // The client's stream is subscribed to the reader, not a second stream of its own.
    let mut client = node.open_client_stream(&session).await;
    assert!(up.notify(
        &upstream_session,
        &json!({"jsonrpc": "2.0", "method": "notifications/message",
                "params": {"level": "info", "logger": "test", "data": "hello-from-upstream"}})
    ));
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !String::from_utf8_lossy(&seen).contains("hello-from-upstream") {
        let n = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .expect("a message on the client stream")
            .unwrap();
        assert_ne!(
            n,
            0,
            "the client stream ended: {}",
            String::from_utf8_lossy(&seen)
        );
        seen.extend_from_slice(&buf[..n]);
    }
    assert!(
        String::from_utf8_lossy(&seen).contains("hello-from-upstream"),
        "{}",
        String::from_utf8_lossy(&seen)
    );
    assert_eq!(
        up.extra_gets.load(Ordering::SeqCst),
        0,
        "no second upstream stream"
    );
    assert_eq!(up.gets.load(Ordering::SeqCst), 1, "one upstream stream");
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_streamed_call_is_relayed_while_the_reader_answers_pings() {
    let up = Upstream::new("solo");
    *up.ping_every.lock().unwrap() = Some(Duration::from_millis(100));
    let node = Node::start(vec![Arc::clone(&up)]).await;
    let session = node.initialize().await;
    until(Duration::from_secs(5), || up.gets.load(Ordering::SeqCst) == 1).await;

    let answered = up.ping_answers.load(Ordering::SeqCst);
    up.stream_call.store(true, Ordering::SeqCst);
    let (status, _, body) = node
        .post(
            &session,
            &json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
                    "params": {"name": "solo__echo_solo", "arguments": {}}}),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("called echo_solo on solo"), "{body}");
    assert!(body.contains("notifications/progress"), "{body}");
    assert!(
        up.ping_answers.load(Ordering::SeqCst) >= answered + 2,
        "the reader answered the pings sent during the call"
    );
    assert_eq!(up.extra_gets.load(Ordering::SeqCst), 0, "one stream per session");
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ending_the_session_stops_the_reader() {
    let up = Upstream::new("solo");
    *up.ping_every.lock().unwrap() = Some(Duration::from_millis(100));
    let node = Node::start(vec![Arc::clone(&up)]).await;
    let session = node.initialize().await;
    until(Duration::from_secs(5), || up.gets.load(Ordering::SeqCst) == 1).await;
    until(Duration::from_secs(5), || {
        up.ping_answers.load(Ordering::SeqCst) >= 1
    })
    .await;

    let auth = format!("Bearer {}", node.key);
    let (status, _, _) = exchange(
        node.data,
        "DELETE",
        "/mcp/g",
        &[
            ("authorization", auth.as_str()),
            ("mcp-session-id", &session),
            ("mcp-protocol-version", LEGACY),
        ],
        "",
    )
    .await;
    assert_eq!(status, 204);
    until(Duration::from_secs(5), || {
        up.get_closes.load(Ordering::SeqCst) == 1
    })
    .await;
    let answered = up.ping_answers.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        up.ping_answers.load(Ordering::SeqCst),
        answered,
        "no ping is answered once the session ended"
    );
    assert_eq!(up.gets.load(Ordering::SeqCst), 1);
    node.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_reader_opens_one_stream_and_does_not_poll() {
    let up = Upstream::new("solo");
    let node = Node::start(vec![Arc::clone(&up)]).await;
    let _session = node.initialize().await;
    until(Duration::from_secs(5), || up.gets.load(Ordering::SeqCst) == 1).await;
    // Two seconds of quiet: a poller would open more streams.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(up.gets.load(Ordering::SeqCst), 1, "one stream, kept open");
    assert_eq!(
        up.get_closes.load(Ordering::SeqCst),
        0,
        "the stream is still open"
    );
    assert_eq!(up.extra_gets.load(Ordering::SeqCst), 0);
    node.stop().await;
}

/// Not an assertion: prints what the node costs while only a background reader is working, so the
/// report can quote a measured number for the idle cost of a link.
#[tokio::test(flavor = "multi_thread")]
async fn idle_cost_of_a_link() {
    fn cpu_time() -> Option<f64> {
        let out = std::process::Command::new("ps")
            .args(["-o", "time=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut parts = text.trim().split(':').rev();
        let seconds: f64 = parts.next()?.parse().ok()?;
        let minutes: f64 = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0.0);
        let hours: f64 = parts.next().and_then(|h| h.parse().ok()).unwrap_or(0.0);
        Some(hours * 3600.0 + minutes * 60.0 + seconds)
    }

    let up = Upstream::new("solo");
    let node = Node::start(vec![Arc::clone(&up)]).await;

    // The node on its own, before any session exists.
    let base_before = cpu_time();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let base_after = cpu_time();

    let _session = node.initialize().await;
    until(Duration::from_secs(5), || up.gets.load(Ordering::SeqCst) == 1).await;
    let before = cpu_time();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = cpu_time();
    if let (Some(base_before), Some(base_after), Some(before), Some(after)) =
        (base_before, base_after, before, after)
    {
        eprintln!(
            "idle cost: node alone {:.3} s of CPU over 2 s; with one session reader {:.3} s over 2 s \
             (delta {:.3} s), {} GET stream(s), {} ping answer(s)",
            base_after - base_before,
            after - before,
            (after - before) - (base_after - base_before),
            up.gets.load(Ordering::SeqCst),
            up.ping_answers.load(Ordering::SeqCst)
        );
    }
    assert_eq!(up.gets.load(Ordering::SeqCst), 1);
    node.stop().await;
}
