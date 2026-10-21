//! The tool index a semantic `tool_search` ranks with: every tool text is keyed by the model that
//! embeds it, answered from the node's cache or the store, and embedded in batches as auxiliary
//! `mcp_tool_index` lines of the request. A Model that failed to embed is skipped for a minute
//! instead of costing every search a timeout, and every store error is a miss: a search still
//! answers, ranked by words.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use antirion_core::{RequestId, UtcMicros};
use antirion_mcp::{CallLog, Pending, ToolIndexBackend};
use antirion_providers::Native;
use antirion_runtime::{Runtime, Snapshot};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::account::Slot;
use crate::internal::{Answer, Asker, Question, ask_settled};
use crate::reqlog::Draft;

/// Field separator of a key's hash: `0x1f` never starts a UTF-8 sequence, so no two triples collide.
const SEP: u8 = 0x1f;
/// Tool texts one embedding call carries.
const BATCH: usize = 32;
/// How long a Model that failed to embed is skipped.
const DOWN: Duration = Duration::from_secs(60);
/// How long a tool text stays in the store without answering a search.
const RETAIN: Duration = Duration::from_secs(180 * 24 * 3600);
/// The least interval between two statements maintaining the store's rows.
const SWEEP: Duration = Duration::from_secs(3600);

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The key of one text: the model alias, the resolved upstream model and the text, hashed. A
/// repointed alias or a re-worded tool is a different key, so a model's vectors are never reused
/// for another's.
fn key(alias: &str, upstream: &str, text: &str) -> String {
    let mut h = Sha256::new();
    h.update(alias.as_bytes());
    h.update([SEP]);
    h.update(upstream.as_bytes());
    h.update([SEP]);
    h.update(text.as_bytes());
    hex(&h.finalize())
}

/// Whether `last` is old enough to repeat at `now`.
fn due(last: Option<UtcMicros>, now: UtcMicros, every: Duration) -> bool {
    last.is_none_or(|l| now.since(l) >= every)
}

/// `now` moved back by `d`, never before the epoch.
fn before(now: UtcMicros, d: Duration) -> UtcMicros {
    let micros = i64::try_from(d.as_micros()).unwrap_or(i64::MAX);
    UtcMicros(now.0.saturating_sub(micros))
}

/// The lock's guard, whatever another task did with it.
fn lock() -> std::sync::MutexGuard<'static, Maintenance> {
    MAINTENANCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What this node maintains in the vector table, whatever request is running.
#[derive(Debug, Default)]
struct Maintenance {
    /// When each Model's rows were last touched.
    touched: HashMap<String, UtcMicros>,
    /// When this node last pruned the table.
    pruned: Option<UtcMicros>,
}

static MAINTENANCE: LazyLock<Mutex<Maintenance>> = LazyLock::new(|| Mutex::new(Maintenance::default()));

/// The index of one request. The mcp crate asks for vectors; the embedding is settled on the
/// request, cached on the node and persisted in the store, so an unchanged tool is asked of the
/// Model once and read back after a restart.
#[derive(Debug)]
pub(crate) struct ToolIndex {
    rt: Arc<Runtime>,
    snap: Arc<Snapshot>,
    request_id: RequestId,
    log: Draft,
    facts: Arc<CallLog>,
    account: Slot,
    /// This request's embedding calls: ledger rows are idempotent by `(request_id, line)`, so the
    /// counter is what keeps two batches of one request from settling as one line.
    calls: AtomicU32,
    gone: CancellationToken,
}

impl ToolIndex {
    /// The index of one request, over the node's cache and the store it shares.
    #[must_use]
    pub(crate) fn new(
        rt: Arc<Runtime>,
        snap: Arc<Snapshot>,
        request_id: RequestId,
        log: Draft,
        facts: Arc<CallLog>,
        account: Slot,
        gone: CancellationToken,
    ) -> ToolIndex {
        ToolIndex {
            rt,
            snap,
            request_id,
            log,
            facts,
            account,
            calls: AtomicU32::new(0),
            gone,
        }
    }

    /// The model identity a text is keyed by: the alias as configured and the upstream model the
    /// embeddings are actually asked of.
    fn identity(&self, model: &str) -> Result<(String, String), String> {
        let resolved = self
            .snap
            .routing
            .resolve(model)
            .or_else(|| {
                self.snap
                    .routing
                    .models
                    .get(model)
                    .map(|m| antirion_router::Resolved {
                        model: Arc::clone(m),
                        alias_params: None,
                        captures: None,
                        via: "exact",
                    })
            })
            .ok_or_else(|| format!("model {model} does not exist"))?;
        let upstream =
            crate::authorize::candidates(&self.snap.providers, &self.snap.routing, &resolved.model)
                .into_iter()
                .find(|d| d.usable() && d.speaks(Native::OpenAi))
                .map(|d| d.model.clone())
                .ok_or_else(|| format!("model {model} has no deployment for this call"))?;
        Ok((model.to_owned(), upstream))
    }

    /// Whether the Model is known to be failing on this node, which saves a search its timeout.
    fn down(&self, model: &str) -> bool {
        self.rt
            .state
            .mcp
            .tool_index_down_until(model)
            .is_some_and(|until| until > self.rt.now())
    }

    /// Drops the cached rows a search answered from after `RETAIN` unused, at most once an hour.
    fn prune(&self) {
        let now = self.rt.now();
        {
            let mut m = lock();
            if !due(m.pruned, now, SWEEP) {
                return;
            }
            m.pruned = Some(now);
        }
        let store = Arc::clone(&self.rt.store);
        let before = before(now, RETAIN).0;
        self.rt.tasks.spawn(async move {
            match store.prune_tool_vectors(before).await {
                Ok(n) if n > 0 => tracing::debug!(rows = n, "old mcp tool vectors pruned"),
                Ok(_) => {}
                Err(e) => tracing::debug!(error = %e, "mcp tool vectors could not be pruned"),
            }
        });
    }

    /// Moves the stored rows a search answered from, at most once an hour per Model.
    fn touch(&self, model: &str, hashes: Vec<String>) {
        let now = self.rt.now();
        {
            let mut m = lock();
            if !due(m.touched.get(model).copied(), now, SWEEP) {
                return;
            }
            m.touched.insert(model.to_owned(), now);
        }
        let store = Arc::clone(&self.rt.store);
        let model = model.to_owned();
        self.rt.tasks.spawn(async move {
            if let Err(e) = store.touch_tool_vectors(&model, &hashes, now.0).await {
                tracing::debug!(model, error = %e, "mcp tool vectors could not be touched");
            }
        });
    }

    /// The stored vectors of `hashes`; a store error is no vectors, so the caller embeds again.
    async fn load(&self, alias: &str, upstream: &str, hashes: &[String]) -> Vec<(String, Vec<f32>)> {
        match self.rt.store.load_tool_vectors(alias, upstream, hashes).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(model = alias, error = %e, "mcp tool vectors could not be read");
                Vec::new()
            }
        }
    }

    /// Writes the vectors this search embedded; a store error leaves the node's cache to answer.
    async fn save(&self, alias: &str, upstream: &str, items: &[(String, usize, Vec<f32>)]) {
        let now = self.rt.now().0;
        if let Err(e) = self.rt.store.put_tool_vectors(alias, upstream, items, now).await {
            tracing::debug!(model = alias, error = %e, "mcp tool vectors could not be written");
        }
    }

    /// One embedding call, settled as the request's next `mcp_tool_index` line.
    async fn ask(&self, model: &str, texts: &[String], timeout: Duration) -> Result<Answer, String> {
        let line = self.calls.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        let asker = Asker::tool_index(line);
        let q = Question::Embed(texts);
        let call = ask_settled(
            &self.rt,
            &self.snap,
            &self.request_id,
            &self.log,
            &self.account,
            model,
            q,
            timeout,
            asker,
        );
        tokio::select! {
            _ = self.gone.cancelled() => Err("the caller went away".to_owned()),
            r = call => r,
        }
    }

    /// Vectors for `texts` in order, embedding what neither the node nor the store has.
    async fn vectors(
        &self,
        model: &str,
        texts: &[String],
        timeout: Duration,
    ) -> Result<Vec<Option<Arc<[f32]>>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let (alias, upstream) = self.identity(model)?;
        if self.down(model) {
            return Err(format!("model {model} is not embedding"));
        }
        self.prune();
        let state = &self.rt.state.mcp;
        let mut vectors: Vec<Option<Arc<[f32]>>> = vec![None; texts.len()];
        let mut missing: Vec<(usize, String, String)> = Vec::new();
        for (i, text) in texts.iter().enumerate() {
            let k = key(&alias, &upstream, text);
            match state.tool_vector(&k) {
                Some(v) => {
                    if let Some(slot) = vectors.get_mut(i) {
                        *slot = Some(v);
                    }
                }
                None => missing.push((i, k, text.clone())),
            }
        }
        let mut to_embed: Vec<(usize, String, String)> = Vec::new();
        if !missing.is_empty() {
            let hashes: Vec<String> = missing.iter().map(|(_, k, _)| k.clone()).collect();
            let mut found: HashMap<String, Vec<f32>> =
                self.load(&alias, &upstream, &hashes).await.into_iter().collect();
            let mut cached: Vec<(String, Arc<[f32]>)> = Vec::new();
            let mut used: Vec<String> = Vec::new();
            for (i, k, text) in missing {
                match found.remove(&k) {
                    Some(v) => {
                        let v: Arc<[f32]> = v.into();
                        if let Some(slot) = vectors.get_mut(i) {
                            *slot = Some(Arc::clone(&v));
                        }
                        used.push(k.clone());
                        cached.push((k, v));
                    }
                    None => to_embed.push((i, k, text)),
                }
            }
            state.put_tool_vectors(&cached);
            if !used.is_empty() {
                self.touch(&alias, used);
            }
        }
        let embedded = u32::try_from(to_embed.len()).unwrap_or(u32::MAX);
        self.embed(model, &alias, &upstream, &mut vectors, to_embed, timeout)
            .await;
        let misses = u32::try_from(vectors.iter().filter(|v| v.is_none()).count()).unwrap_or(u32::MAX);
        self.facts.tool_index(|f| {
            f.embedded = f.embedded.saturating_add(embedded);
            f.misses = f.misses.saturating_add(misses);
        });
        if vectors.iter().all(Option::is_none) {
            return Err(format!("model {model} embedded nothing"));
        }
        Ok(vectors)
    }

    /// Embeds `missing` in batches of [`BATCH`], keeping what answered: the texts a batch could not
    /// embed stay `None`, so the search ranks without them instead of failing.
    async fn embed(
        &self,
        model: &str,
        alias: &str,
        upstream: &str,
        vectors: &mut [Option<Arc<[f32]>>],
        missing: Vec<(usize, String, String)>,
        timeout: Duration,
    ) {
        if missing.is_empty() {
            return;
        }
        let state = &self.rt.state.mcp;
        let mut wrote: Vec<(String, usize, Vec<f32>)> = Vec::new();
        let mut cached: Vec<(String, Arc<[f32]>)> = Vec::new();
        let mut failed = false;
        let mut start = 0;
        while start < missing.len() {
            let end = (start + BATCH).min(missing.len());
            let batch = missing.get(start..end).unwrap_or_default();
            start = end;
            let texts: Vec<String> = batch.iter().map(|(_, _, t)| t.clone()).collect();
            match self.ask(model, &texts, timeout).await {
                Ok(Answer::Vectors(v)) if v.len() == batch.len() => {
                    let texts_embedded = u32::try_from(v.len()).unwrap_or(u32::MAX);
                    self.rt.metrics.add(
                        antirion_obs::Fam::McpToolIndexEmbeddings,
                        &[alias, "embedded"],
                        f64::from(texts_embedded),
                    );
                    for ((i, k, _), vector) in batch.iter().zip(v) {
                        let dim = vector.len();
                        let arc: Arc<[f32]> = vector.clone().into();
                        if let Some(slot) = vectors.get_mut(*i) {
                            *slot = Some(Arc::clone(&arc));
                        }
                        cached.push((k.clone(), arc));
                        wrote.push((k.clone(), dim, vector));
                    }
                }
                other => {
                    failed = true;
                    let texts_failed = u32::try_from(batch.len()).unwrap_or(u32::MAX);
                    self.rt.metrics.add(
                        antirion_obs::Fam::McpToolIndexEmbeddings,
                        &[alias, "error"],
                        f64::from(texts_failed),
                    );
                    let reason = match other {
                        Ok(Answer::Vectors(v)) => format!("{} vectors for {} texts", v.len(), batch.len()),
                        Ok(Answer::Text(_)) => "text instead of vectors".to_owned(),
                        Err(e) => e,
                    };
                    tracing::debug!(model, error = %reason, "a tool-index embedding batch failed");
                }
            }
        }
        state.put_tool_vectors(&cached);
        if !wrote.is_empty() {
            self.save(alias, upstream, &wrote).await;
        }
        if failed {
            state.tool_index_failed(model, self.rt.now().saturating_add(DOWN));
        }
    }

    /// The query's vector, from the node's cache or the Model; never stored.
    async fn query(&self, model: &str, text: &str, timeout: Duration) -> Result<Option<Arc<[f32]>>, String> {
        let (alias, upstream) = self.identity(model)?;
        let k = key(&alias, &upstream, text);
        let state = &self.rt.state.mcp;
        if let Some(v) = state.query_vector(&k) {
            return Ok(Some(v));
        }
        if self.down(model) {
            return Err(format!("model {model} is not embedding"));
        }
        let texts = [text.to_owned()];
        let vector = match self.ask(model, &texts, timeout).await {
            Ok(Answer::Vectors(v)) if v.len() == texts.len() => v.into_iter().next().map(Arc::<[f32]>::from),
            Ok(other) => {
                tracing::debug!(
                    model,
                    answer = ?std::mem::discriminant(&other),
                    "the query's embedding is malformed"
                );
                state.tool_index_failed(model, self.rt.now().saturating_add(DOWN));
                None
            }
            Err(e) => {
                state.tool_index_failed(model, self.rt.now().saturating_add(DOWN));
                return Err(e);
            }
        };
        if let Some(v) = &vector {
            state.put_query_vector(k, Arc::clone(v));
        }
        Ok(vector)
    }
}

impl ToolIndexBackend for ToolIndex {
    fn tool_vectors<'a>(
        &'a self,
        model: &'a str,
        texts: &'a [String],
        timeout: Duration,
    ) -> Pending<'a, Result<Vec<Option<Arc<[f32]>>>, String>> {
        Box::pin(async move { self.vectors(model, texts, timeout).await })
    }

    fn query_vector<'a>(
        &'a self,
        model: &'a str,
        text: &'a str,
        timeout: Duration,
    ) -> Pending<'a, Result<Option<Arc<[f32]>>, String>> {
        Box::pin(async move { self.query(model, text, timeout).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_separate_what_a_plain_concatenation_would_join() {
        assert_ne!(key("a", "bc", "d"), key("ab", "c", "d"));
        assert_eq!(key("m", "up", "text"), key("m", "up", "text"));
        assert_eq!(key("m", "up", "text").len(), 64);
    }

    #[test]
    fn maintenance_is_due_once_an_interval_passes() {
        let now = UtcMicros(10_000_000_000);
        assert!(due(None, now, SWEEP));
        assert!(!due(Some(now), now, SWEEP));
        assert!(due(Some(before(now, SWEEP)), now, SWEEP));
    }
}
