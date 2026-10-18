//! MCP session revocations are shared through the counter store: a node writes each one it makes at
//! once (kept until the token would expire), and every request with a session token the node does
//! not know as revoked asks the store, so a revocation is exact on every node from its next request.
//! Every second each node also reads the revocations shared since its last read, from the store's
//! revocation log, and writes the ones of its own the store does not hold yet (those made while the
//! store could not be reached): the cost follows the revocations, not the traffic. While the counter
//! store is down, each node checks only the revocations it knows (upstreams still refuse deleted
//! sessions).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::pause;
use crate::Runtime;

const EVERY: Duration = Duration::from_secs(1);

fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if s.len() != 64 {
        return None;
    }
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// How long a request waits for the shared revocation set before it skips the check.
const CHECK_TIMEOUT: Duration = Duration::from_millis(250);

impl Runtime {
    /// Before a request carrying an MCP session token: learns whether another node revoked it.
    /// False when the shared set could not be read, so the check was skipped.
    pub async fn check_shared_revocation(&self, token: &str) -> bool {
        let engine = self.accounts.engine();
        let Some(valkey) = engine.store().valkey() else {
            return true;
        };
        let (digest, known) = self.state.mcp.revocation_of(token);
        if known.is_some() {
            return true;
        }
        if engine.is_store_down() {
            return false;
        }
        match tokio::time::timeout(CHECK_TIMEOUT, valkey.revocation(&hex(&digest))).await {
            Ok(Ok(Some(exp))) => {
                self.state.mcp.add_revocations([(digest, exp)]);
                true
            }
            Ok(Ok(None)) => true,
            _ => false,
        }
    }

    /// After a session was deleted on this node: shares its revocation at once, so the next
    /// request on any node is refused.
    pub async fn share_revocation(&self, token: &str) {
        let engine = self.accounts.engine();
        let Some(valkey) = engine.store().valkey() else {
            return;
        };
        if engine.is_store_down() {
            return;
        }
        if let (digest, Some(exp)) = self.state.mcp.revocation_of(token) {
            let item = [(hex(&digest), exp)];
            let shared = valkey.share_revocations(&item);
            if !matches!(tokio::time::timeout(CHECK_TIMEOUT, shared).await, Ok(Ok(_))) {
                tracing::debug!("a session revocation is shared with the next round");
            }
        }
    }
}

pub(crate) async fn run(rt: Arc<Runtime>) {
    let mut sync = Sync::default();
    while pause(&rt, EVERY).await {
        let engine = rt.accounts.engine();
        let Some(valkey) = engine.store().valkey() else {
            return;
        };
        if engine.is_store_down() {
            continue;
        }
        let Ok((upto, shared)) = valkey.revocations_since(sync.cursor).await else {
            continue;
        };
        if upto < sync.cursor {
            // The store lost its log: what this node knows is shared again.
            sync.stored.clear();
        }
        sync.cursor = upto;
        let shared: Vec<([u8; 32], i64)> = shared.iter().filter_map(|(d, e)| Some((unhex(d)?, *e))).collect();
        sync.stored.extend(shared.iter().copied());
        rt.state.mcp.add_revocations(shared);
        let mine = rt.state.mcp.revocations();
        let unshared: Vec<(String, i64)> = mine
            .iter()
            .filter(|(d, _)| !sync.stored.contains_key(d))
            .map(|(d, e)| (hex(d), *e))
            .collect();
        if unshared.is_empty() || valkey.share_revocations(&unshared).await.is_ok() {
            // What the node holds is in the store now; expired ones fall out with it.
            sync.stored = mine.into_iter().collect();
        }
    }
}

/// A node's position in the store's revocation log and the revocations it knows the store holds.
#[derive(Default)]
struct Sync {
    cursor: u64,
    stored: HashMap<[u8; 32], i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_round_trip_through_hex() {
        let d = [0xabu8; 32];
        assert_eq!(unhex(&hex(&d)), Some(d));
        assert_eq!(unhex("ab"), None);
    }
}
