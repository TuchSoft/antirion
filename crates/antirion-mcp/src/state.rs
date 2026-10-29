//! Node-local MCP state that outlives requests and generations: the generated session key, list,
//! discovery and card caches, the revocation set, the upstream sessions this node holds open,
//! requests in flight, and tool fingerprints with their approvals. Nothing here is needed to resume
//! a session on another node.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use antirion_core::UtcMicros;
use parking_lot::{Mutex, RwLock};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::compile::SessionKey;
use crate::link::Link;
use crate::ssetransport::SseConn;

/// Caching hints one server attached to a modern result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hints {
    pub(crate) ttl_ms: i64,
    pub(crate) public: bool,
}

impl Hints {
    pub(crate) fn of(result: &Value) -> Hints {
        Hints {
            ttl_ms: result.get("ttlMs").and_then(Value::as_i64).unwrap_or(0).max(0),
            public: result.get("cacheScope").and_then(Value::as_str) == Some("public"),
        }
    }
}

/// One server's complete list of one kind, read across all upstream pages.
#[derive(Clone, Debug)]
pub(crate) struct ListEntry {
    pub(crate) items: Arc<Vec<Value>>,
    /// Size of the first upstream page, when the server paged its list.
    pub(crate) page: Option<usize>,
    pub(crate) hints: Hints,
    pub(crate) at: UtcMicros,
}

/// Cache key of a list: the server's generation, the credential identity and the method.
pub(crate) type ListKey = (u64, u64, &'static str);

/// What discovery learned about a server.
#[derive(Clone, Debug)]
pub(crate) struct Discovered {
    pub(crate) modern: bool,
    pub(crate) at: UtcMicros,
}

/// Per-server facts for the status endpoint.
#[derive(Clone, Debug, Default)]
pub(crate) struct ServerFacts {
    pub(crate) last_discovery_at: Option<UtcMicros>,
    pub(crate) last_error: Option<String>,
    pub(crate) modern: Option<bool>,
    /// The last attempt to initialize the server or read its tools, and why it failed.
    pub(crate) last_contact_at: Option<UtcMicros>,
    pub(crate) last_contact_error: Option<String>,
}

/// Approved tool fingerprints of one server.
#[derive(Clone, Debug)]
pub(crate) struct Approval {
    pub(crate) tools: BTreeMap<String, String>,
    pub(crate) approved_at: UtcMicros,
}

/// A tool definition that changed upstream since the server's previous list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionChange {
    pub server: String,
    pub tool: String,
    /// `added`, `removed` or `changed`.
    pub change: &'static str,
    pub old_fingerprint: Option<String>,
    pub new_fingerprint: Option<String>,
}

/// An upstream session this node re-opened for a client session whose server had forgotten it.
///
/// The session token is sealed and carries the upstream session ids it was issued with, so a
/// re-opened session has no place in it: it is kept here until the client session ends, and the
/// next request of that session reuses it instead of opening another one.
#[derive(Clone, Debug)]
pub(crate) struct Reopened {
    pub(crate) sid: String,
    /// The version negotiated with the server on the re-opened session.
    pub(crate) pv: String,
    /// The server generation the session was re-opened at; a reloaded server starts a new one.
    pub(crate) generation: u64,
    pub(crate) at: UtcMicros,
}

/// An upstream session this node holds open between requests: the link that ends it, when a request
/// last used it, and how many requests use it right now.
///
/// A server session outlives the request that opened it, and a client that goes away without a
/// `DELETE` leaves it open for as long as the server keeps it. The registry is what a reaper needs
/// to end such a session: the link carries the server, its headers and its protocol version.
#[derive(Debug)]
pub(crate) struct Held {
    pub(crate) link: Link,
    /// When a request last used the session.
    seen: Mutex<Instant>,
    /// The requests using the session right now.
    busy: AtomicUsize,
}

impl Held {
    fn new(link: &Link, at: Instant) -> Held {
        Held {
            link: link.clone(),
            seen: Mutex::new(at),
            busy: AtomicUsize::new(0),
        }
    }

    fn touch(&self, at: Instant) {
        *self.seen.lock() = at;
    }

    /// How long the session has gone without a request.
    pub(crate) fn idle(&self, at: Instant) -> Duration {
        at.saturating_duration_since(*self.seen.lock())
    }

    /// Whether a request uses the session right now.
    pub(crate) fn busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed) > 0
    }

    /// Counts one request on the session until the guard is dropped.
    fn enter(self: &Arc<Self>) -> Active {
        self.busy.fetch_add(1, Ordering::Relaxed);
        Active(Arc::clone(self))
    }
}

/// One request using a held session; dropping it ends the count, also when the request unwinds.
#[derive(Debug)]
pub(crate) struct Active(#[allow(dead_code)] Arc<Held>);

impl Drop for Active {
    fn drop(&mut self) {
        self.0.busy.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Session counters of one gateway.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GatewayCounts {
    pub(crate) issued: u64,
    pub(crate) rejected: u64,
}

/// The vectors of a bounded number of keys; the ones inserted first are evicted first.
#[derive(Debug, Default)]
struct Vectors {
    items: HashMap<String, Arc<[f32]>>,
    order: VecDeque<String>,
}

impl Vectors {
    fn get(&self, key: &str) -> Option<Arc<[f32]>> {
        self.items.get(key).cloned()
    }

    fn put(&mut self, key: String, vector: Arc<[f32]>, max: usize) {
        if self.items.insert(key.clone(), vector).is_none() {
            self.order.push_back(key);
        }
        while self.items.len() > max {
            let Some(oldest) = self.order.pop_front() else {
                return;
            };
            self.items.remove(&oldest);
        }
    }

    fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
    }
}

/// How many tool vectors one node keeps in memory.
const MAX_TOOL_VECTORS: usize = 100_000;
/// How many query vectors one node keeps in memory.
const MAX_QUERY_VECTORS: usize = 4096;

/// The MCP state of one node.
#[derive(Debug, Default)]
pub struct McpState {
    generated_key: RwLock<Option<[u8; 32]>>,
    pub(crate) lists: Mutex<HashMap<ListKey, ListEntry>>,
    pub(crate) discovery: Mutex<HashMap<u64, Discovered>>,
    pub(crate) facts: Mutex<HashMap<String, ServerFacts>>,
    pub(crate) cards: Mutex<HashMap<u64, (UtcMicros, Value)>>,
    revoked: Mutex<HashMap<[u8; 32], i64>>,
    /// Re-opened upstream sessions, by client-session digest and server name.
    reopened: Mutex<HashMap<(String, String), Reopened>>,
    inflight: Mutex<HashMap<(String, String), String>>,
    pub(crate) sse: Mutex<HashMap<String, Arc<SseConn>>>,
    /// The background readers of upstream sessions, by upstream session id.
    pub(crate) watches: Mutex<HashMap<String, Arc<crate::watch::Watch>>>,
    /// Upstream sessions held open, by server name and upstream session id.
    sessions: Mutex<HashMap<String, HashMap<String, Arc<Held>>>>,
    pub(crate) fingerprints: Mutex<HashMap<String, BTreeMap<String, String>>>,
    pub(crate) approvals: Mutex<HashMap<String, Approval>>,
    pub(crate) definition_changes: Mutex<BTreeMap<(String, &'static str), u64>>,
    /// Changes not yet reported as metrics and alert events.
    pub(crate) unreported: Mutex<Vec<DefinitionChange>>,
    pub(crate) counts: Mutex<HashMap<String, GatewayCounts>>,
    /// Client-facing HTTP+SSE streams open on this node, by handle.
    pub(crate) client_streams: Mutex<HashMap<String, Arc<crate::clientsse::ClientStream>>>,
    /// Authorization-server metadata by issuer, with when it was read.
    pub(crate) issuers: Mutex<HashMap<String, (UtcMicros, Value)>>,
    /// The process pools of `stdio` servers, by server generation.
    stdio: Mutex<HashMap<u64, Arc<crate::stdio::StdioServer>>>,
    /// Tool vectors, keyed as the pipeline keys them. Content-addressed, so a new generation does
    /// not disown them.
    tool_vectors: Mutex<Vectors>,
    /// The vectors of recent searches' queries; a query is never the key of a stored tool.
    query_vectors: Mutex<Vectors>,
    /// Until when an embedding Model is known to fail on this node, by model alias.
    tool_index_down: Mutex<HashMap<String, UtcMicros>>,
}

/// The id byte of the node's generated session key.
const GENERATED_KEY_ID: u8 = 0;

/// How many re-opened upstream sessions one node keeps; the oldest goes first.
const MAX_REOPENED: usize = 4096;

impl McpState {
    /// Records the outcome of an attempt to initialize `server` or read its tools.
    pub(crate) fn record_contact(&self, server: &str, at: UtcMicros, error: Option<String>) {
        let mut facts = self.facts.lock();
        let f = facts.entry(server.to_owned()).or_default();
        f.last_contact_at = Some(at);
        f.last_contact_error = error;
    }

    /// Installs the session key generated for this deployment (read or created in the store at start).
    pub fn set_generated_key(&self, key: [u8; 32]) {
        *self.generated_key.write() = Some(key);
    }

    /// The keys of a request: the configured ones, else the generated one.
    pub(crate) fn keys(&self, configured: &[SessionKey]) -> Vec<SessionKey> {
        if !configured.is_empty() {
            return configured.to_vec();
        }
        self.generated_key
            .read()
            .map(|secret| {
                vec![SessionKey {
                    id: GENERATED_KEY_ID,
                    secret,
                }]
            })
            .unwrap_or_default()
    }

    /// Drops the list, discovery and agent-card caches, what tool fingerprints were seen and
    /// approved, and the tool vectors: every cached vector is re-read or embedded again.
    pub fn clear_caches(&self) {
        self.issuers.lock().clear();
        self.fingerprints.lock().clear();
        self.unreported.lock().clear();
        self.approvals.lock().clear();
        self.lists.lock().clear();
        self.discovery.lock().clear();
        self.cards.lock().clear();
        self.tool_vectors.lock().clear();
        self.query_vectors.lock().clear();
    }

    /// The vector cached for a tool text under `key`.
    #[must_use]
    pub fn tool_vector(&self, key: &str) -> Option<Arc<[f32]>> {
        self.tool_vectors.lock().get(key)
    }

    /// Caches vectors of tool texts under the keys the pipeline computed.
    pub fn put_tool_vectors(&self, items: &[(String, Arc<[f32]>)]) {
        let mut vectors = self.tool_vectors.lock();
        for (key, vector) in items {
            vectors.put(key.clone(), Arc::clone(vector), MAX_TOOL_VECTORS);
        }
    }

    /// The vector cached for a query under `key`.
    #[must_use]
    pub fn query_vector(&self, key: &str) -> Option<Arc<[f32]>> {
        self.query_vectors.lock().get(key)
    }

    /// Caches the vector of a query. Queries are never persisted.
    pub fn put_query_vector(&self, key: String, vector: Arc<[f32]>) {
        self.query_vectors.lock().put(key, vector, MAX_QUERY_VECTORS);
    }

    /// Until when Model `model` is known to fail on this node, when it is.
    #[must_use]
    pub fn tool_index_down_until(&self, model: &str) -> Option<UtcMicros> {
        self.tool_index_down.lock().get(model).copied()
    }

    /// Records that Model `model` failed to embed until `until`; a broken model must not cost every
    /// search a timeout.
    pub fn tool_index_failed(&self, model: &str, until: UtcMicros) {
        self.tool_index_down.lock().insert(model.to_owned(), until);
    }

    /// The background reader of the upstream session with `sid`, when one is running.
    pub(crate) fn watch(&self, sid: &str) -> Option<Arc<crate::watch::Watch>> {
        self.watches.lock().get(sid).cloned()
    }

    /// Registers a background reader unless the session already has one; `true` when registered.
    pub(crate) fn set_watch(&self, sid: String, watch: Arc<crate::watch::Watch>) -> bool {
        let mut watches = self.watches.lock();
        if watches.contains_key(&sid) {
            return false;
        }
        watches.insert(sid, watch);
        true
    }

    /// Stops the background reader of the upstream session `sid`.
    pub(crate) fn stop_watch(&self, sid: &str) {
        if let Some(watch) = self.watches.lock().remove(sid) {
            watch.stop();
        }
    }

    /// Forgets a background reader that ended on its own, when it is still the one registered.
    pub(crate) fn drop_watch(&self, sid: &str, watch: &Arc<crate::watch::Watch>) {
        let mut watches = self.watches.lock();
        if watches.get(sid).is_some_and(|w| Arc::ptr_eq(w, watch)) {
            watches.remove(sid);
        }
    }

    /// Whether a client's server-to-client stream is attached to the session's reader.
    fn subscribed(&self, sid: &str) -> bool {
        self.watches.lock().get(sid).is_some_and(|w| w.subscribers() > 0)
    }

    /// Records the upstream session of `link`, or refreshes it. A link without an upstream session
    /// holds nothing.
    pub(crate) fn hold(&self, link: &Link, at: Instant) {
        let Some(sid) = &link.sid else {
            return;
        };
        let mut sessions = self.sessions.lock();
        let held = sessions.entry(link.server.name.clone()).or_default();
        match held.get(sid) {
            Some(existing) => existing.touch(at),
            None => {
                held.insert(sid.clone(), Arc::new(Held::new(link, at)));
            }
        }
    }

    /// Counts one request on the link's session, and marks it used; `None` when the session is not
    /// held (no upstream session, or one this node does not keep track of).
    pub(crate) fn active(&self, link: &Link, at: Instant) -> Option<Active> {
        let sid = link.sid.as_deref()?;
        let held = self.sessions.lock().get(&link.server.name)?.get(sid).cloned()?;
        held.touch(at);
        Some(held.enter())
    }

    /// Forgets the upstream session of `link`, which a `DELETE` ended.
    pub(crate) fn forget(&self, link: &Link) {
        let Some(sid) = &link.sid else {
            return;
        };
        let mut sessions = self.sessions.lock();
        if let Some(held) = sessions.get_mut(&link.server.name) {
            held.remove(sid);
            if held.is_empty() {
                sessions.remove(&link.server.name);
            }
        }
    }

    /// The sessions that have gone `idle` without a request, with no request using them and no
    /// client stream attached; they are removed from the registry, so each is returned once.
    pub(crate) fn due(&self, at: Instant, idle: Duration) -> Vec<Arc<Held>> {
        let mut sessions = self.sessions.lock();
        let mut due = Vec::new();
        sessions.retain(|_, held| {
            held.retain(|sid, h| {
                if h.busy() || h.idle(at) < idle || self.subscribed(sid) {
                    return true;
                }
                due.push(Arc::clone(h));
                false
            });
            !held.is_empty()
        });
        due
    }

    /// How many upstream sessions this node holds open.
    pub fn held_sessions(&self) -> usize {
        self.sessions.lock().values().map(HashMap::len).sum()
    }

    /// Closes held HTTP+SSE upstream sessions and forgets revocations and session counters.
    pub fn clear_sessions(&self) {
        for conn in self.sse.lock().drain().map(|(_, c)| c) {
            conn.close();
        }
        for watch in self.watches.lock().drain().map(|(_, w)| w) {
            watch.stop();
        }
        self.sessions.lock().clear();
        self.revoked.lock().clear();
        self.reopened.lock().clear();
        self.counts.lock().clear();
    }

    /// The tool definition changes seen since the last call.
    pub fn take_definition_changes(&self) -> Vec<DefinitionChange> {
        std::mem::take(&mut *self.unreported.lock())
    }

    /// The process pool of a `stdio` server. Pools of generations no server of `snap` has any more
    /// are dropped, which stops their processes.
    pub(crate) fn stdio_pool(
        &self,
        snap: &crate::compile::McpSnapshot,
        server: &crate::compile::ServerRt,
        spec: &crate::compile::StdioSpec,
    ) -> Arc<crate::stdio::StdioServer> {
        let mut pools = self.stdio.lock();
        if let Some(p) = pools.get(&server.generation) {
            return Arc::clone(p);
        }
        pools.retain(|g, _| {
            snap.servers
                .values()
                .any(|s| s.generation == *g && s.stdio.is_some())
        });
        let pool = Arc::new(crate::stdio::StdioServer::new(
            spec.command.clone(),
            spec.args.clone(),
            spec.env.clone(),
            spec.processes,
        ));
        pools.insert(server.generation, Arc::clone(&pool));
        pool
    }

    /// How many processes the pool of the `stdio` server with `generation` has started.
    pub(crate) fn stdio_started(&self, generation: u64) -> Option<(u64, usize)> {
        let pools = self.stdio.lock();
        let p = pools.get(&generation)?;
        Some((p.started.load(std::sync::atomic::Ordering::Relaxed), p.running()))
    }

    pub(crate) fn revoke(&self, token: &str, exp: i64, now: UtcMicros) {
        let mut set = self.revoked.lock();
        let now_s = now.0 / 1_000_000;
        set.retain(|_, e| *e > now_s);
        set.insert(digest(token), exp);
    }

    pub(crate) fn is_revoked(&self, token: &str) -> bool {
        self.revoked.lock().contains_key(&digest(token))
    }

    /// The upstream session this node re-opened for the client session `token` on `server`, when the
    /// server has not been reloaded since.
    pub(crate) fn reopened(&self, token: &str, server: &str, generation: u64) -> Option<Reopened> {
        let map = self.reopened.lock();
        map.get(&(hex(&digest(token)), server.to_owned()))
            .filter(|r| r.generation == generation)
            .cloned()
    }

    /// Records the upstream session re-opened for the client session `token` on `server`, replacing
    /// any one it held before.
    pub(crate) fn remember_reopened(
        &self,
        token: &str,
        server: &str,
        generation: u64,
        pv: &str,
        sid: &str,
        at: UtcMicros,
    ) {
        let key = (hex(&digest(token)), server.to_owned());
        let previous = {
            let mut map = self.reopened.lock();
            if map.len() >= MAX_REOPENED
                && !map.contains_key(&key)
                && let Some(oldest) = map.iter().min_by_key(|(_, r)| r.at.0).map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
            map.insert(
                key,
                Reopened {
                    sid: sid.to_owned(),
                    pv: pv.to_owned(),
                    generation,
                    at,
                },
            )
        };
        if let Some(previous) = previous
            && previous.sid != sid
        {
            self.stop_watch(&previous.sid);
        }
    }

    /// Forgets the sessions re-opened for the client session `token`, the client session having
    /// ended.
    pub(crate) fn forget_reopened(&self, token: &str) {
        let session = hex(&digest(token));
        self.reopened.lock().retain(|(t, _), _| *t != session);
    }

    /// Every unexpired revocation (the token's digest and its expiry in seconds), to share with
    /// other nodes.
    #[must_use]
    pub fn revocations(&self) -> Vec<([u8; 32], i64)> {
        self.revoked.lock().iter().map(|(d, e)| (*d, *e)).collect()
    }

    /// The digest a session token is revoked under, and the revocation's expiry when this node
    /// knows it.
    #[must_use]
    pub fn revocation_of(&self, token: &str) -> ([u8; 32], Option<i64>) {
        let d = digest(token);
        let exp = self.revoked.lock().get(&d).copied();
        (d, exp)
    }

    /// Adds revocations another node made.
    pub fn add_revocations(&self, items: impl IntoIterator<Item = ([u8; 32], i64)>) {
        self.revoked.lock().extend(items);
    }

    pub(crate) fn revoked_count(&self) -> usize {
        self.revoked.lock().len()
    }

    /// Records that a client request (by session and id) is executing on `server`.
    pub(crate) fn begin(self: &Arc<Self>, session: &str, id: &Value, server: &str) -> InFlight {
        let key = (hex(&digest(session)), id.to_string());
        self.inflight.lock().insert(key.clone(), server.to_owned());
        InFlight {
            state: Arc::clone(self),
            key,
        }
    }

    pub(crate) fn executing(&self, session: &str, id: &Value) -> Option<String> {
        self.inflight
            .lock()
            .get(&(hex(&digest(session)), id.to_string()))
            .cloned()
    }

    pub(crate) fn list(&self, key: &ListKey, now: UtcMicros, ttl: Duration) -> Option<ListEntry> {
        let lists = self.lists.lock();
        let e = lists.get(key)?;
        (now.since(e.at) < ttl && now >= e.at).then(|| e.clone())
    }

    pub(crate) fn invalidate(&self, generation: u64, method: &'static str) {
        self.lists
            .lock()
            .retain(|(g, _, m), _| !(*g == generation && *m == method));
    }

    pub(crate) fn count(&self, gateway: &str, issued: bool) {
        let mut counts = self.counts.lock();
        let c = counts.entry(gateway.to_owned()).or_default();
        if issued {
            c.issued += 1;
        } else {
            c.rejected += 1;
        }
    }
}

/// Removes its in-flight record when the request ends, however it ends.
#[derive(Debug)]
pub(crate) struct InFlight {
    state: Arc<McpState>,
    key: (String, String),
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.state.inflight.lock().remove(&self.key);
    }
}

pub(crate) fn digest(s: &str) -> [u8; 32] {
    Sha256::digest(s.as_bytes()).into()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link to one server, with an upstream session and no client behind it.
    fn held_link(server: &str, sid: &str) -> Link {
        let snap = crate::compile::compile(&serde_json::json!({"mcp": {"servers": {
            server: {"transport": "streamable_http", "url": "http://127.0.0.1:1/mcp"}
        }}}))
        .expect("a snapshot");
        Link {
            server: Arc::clone(&snap.servers[server]),
            era: crate::upstream::Era::Legacy,
            pv: "2024-11-05".to_owned(),
            sid: Some(sid.to_owned()),
            sse: None,
            headers: http::HeaderMap::new(),
            meta: None,
        }
    }

    #[test]
    fn a_held_session_is_reaped_once_it_is_idle_and_not_in_use() {
        let state = McpState::default();
        let link = held_link("a", "sid-1");
        let start = Instant::now();
        state.hold(&link, start);
        state.hold(&link, start);
        assert_eq!(state.held_sessions(), 1, "a session is held once");

        // In use: not due however long it has been held.
        let guard = state.active(&link, start).expect("a held session");
        assert!(
            state
                .due(start + Duration::from_secs(3600), Duration::from_secs(1))
                .is_empty()
        );
        drop(guard);

        // Idle for less than the timeout: not due.
        assert!(
            state
                .due(start + Duration::from_millis(500), Duration::from_secs(1))
                .is_empty()
        );

        // Idle for the timeout: due, and forgotten, so only one sweep ends it.
        let due = state.due(start + Duration::from_secs(2), Duration::from_secs(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].link.sid.as_deref(), Some("sid-1"));
        assert_eq!(state.held_sessions(), 0);
        assert!(
            state
                .due(start + Duration::from_secs(3), Duration::from_secs(1))
                .is_empty()
        );
        assert!(
            state.active(&link, start).is_none(),
            "no guard without a held session"
        );

        // An explicit end forgets it too.
        state.hold(&link, start);
        state.forget(&link);
        assert_eq!(state.held_sessions(), 0);
    }

    #[test]
    fn revocation_and_inflight() {
        let s = Arc::new(McpState::default());
        s.revoke("tok", 100, UtcMicros(0));
        assert!(s.is_revoked("tok") && !s.is_revoked("other"));
        {
            let _g = s.begin("tok", &Value::from(77), "a");
            assert_eq!(s.executing("tok", &Value::from(77)).as_deref(), Some("a"));
        }
        assert!(s.executing("tok", &Value::from(77)).is_none());
    }

    #[test]
    fn generated_key_is_used_only_without_configured_keys() {
        let s = McpState::default();
        assert!(s.keys(&[]).is_empty());
        s.set_generated_key([9; 32]);
        assert_eq!(s.keys(&[])[0].id, 0);
        let configured = [SessionKey {
            id: 4,
            secret: [1; 32],
        }];
        assert_eq!(s.keys(&configured)[0].id, 4);
    }
}
