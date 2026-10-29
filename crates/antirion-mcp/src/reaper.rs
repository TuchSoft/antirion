//! Ending the upstream sessions clients abandoned.
//!
//! A client that stops sending requests without ending its session leaves the upstream session it
//! opened to the server: the session lives for as long as the server keeps it, and on a server that
//! isolates its clients (a browser, say) every such session costs a process tree. The registry in
//! [`crate::state::McpState`] holds each session this node opened together with the link that ends
//! it, and a sweep ends the ones that have gone quiet.
//!
//! A session is quiet when no request has used it for the idle timeout, no request is using it
//! right now, and no client's server-to-client stream is attached to it. Ending it does not lose the
//! client: its next request fails with `SessionGone`, which the gateway answers by opening the
//! session again and re-sending the request once.

use std::time::{Duration, Instant};

use crate::egress::Egress;
use crate::state::McpState;

/// How long between two sweeps: a quarter of the idle timeout, between 50 ms and a minute, so a
/// session is ended within the timeout plus a minute of the request that last reached it.
#[must_use]
pub fn reap_interval(idle: Duration) -> Duration {
    (idle / 4).clamp(Duration::from_millis(50), Duration::from_secs(60))
}

/// Ends every held upstream session that has gone `idle`, and returns how many were ended.
pub async fn reap(state: &McpState, egress: &Egress, idle: Duration) -> usize {
    let due = state.due(Instant::now(), idle);
    for held in &due {
        tracing::debug!(
            server = %held.link.server.name,
            "ending an upstream session no request reached for the idle timeout"
        );
        held.link.close(egress, state).await;
    }
    due.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sweep_runs_at_least_every_minute() {
        assert_eq!(reap_interval(Duration::from_secs(600)), Duration::from_secs(60));
        assert_eq!(reap_interval(Duration::from_secs(60)), Duration::from_secs(15));
        assert_eq!(
            reap_interval(Duration::from_millis(100)),
            Duration::from_millis(50)
        );
    }
}
