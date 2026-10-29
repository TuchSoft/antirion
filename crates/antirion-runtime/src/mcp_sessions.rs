//! The reaper of upstream MCP sessions: a sweep every quarter of `mcp.session.idle_timeout` ends
//! the upstream sessions no request has reached for that long, so a client that went away without a
//! `DELETE` stops costing the servers it opened sessions on.

use std::sync::Arc;

use antirion_core::RequestId;

use crate::Runtime;

/// Starts the task that ends abandoned upstream MCP sessions.
pub fn spawn(rt: &Arc<Runtime>) {
    rt.tasks.spawn(run(Arc::clone(rt)));
}

async fn run(rt: Arc<Runtime>) {
    loop {
        let snap = rt.gens.load();
        let idle = snap.mcp.session_idle_timeout();
        tokio::select! {
            () = rt.shutdown.cancelled() => return,
            () = tokio::time::sleep(antirion_mcp::reap_interval(idle)) => {}
        }
        // The egress of a request nobody made: the sweep is a request of the gateway's own, so it
        // goes out under the same rules as any other.
        let Some(egress) = rt.mcp_egress(&snap, &RequestId::generate()) else {
            continue;
        };
        let ended = antirion_mcp::reap(&rt.state.mcp, &egress, idle).await;
        if ended > 0 {
            tracing::debug!(ended, "reaped idle upstream MCP sessions");
        }
    }
}
