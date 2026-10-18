//! What nodes read from each other through the store besides configuration and the request log
//! (which every node queries in the store): the ledger lines stored since this node started (into
//! its window) and the cache epochs. Rows commit in a different order than their store timestamps,
//! so the last few seconds are read again every few ticks; what is read twice is recognised by its
//! id. Lines are listed by key first and only the bodies of the lines the window lacks are read: a
//! node's own lines, and the ones it read already, never cross the network again.

use std::sync::Arc;
use std::time::Duration;

use antirion_limits::ledger::LedgerLine;
use serde_json::Value;

use super::pause;
use crate::Runtime;

const EVERY: Duration = Duration::from_millis(250);
const OVERLAP_US: i64 = 5_000_000;
/// Ticks between two reads of the overlap.
const OVERLAP_EVERY: u64 = 8;
const READ_LIMIT: i64 = 5000;
/// Keys of the overlap read at most at once.
const OVERLAP_LIMIT: i64 = 100_000;
/// Bodies read in one statement.
const BODIES_PER_READ: usize = 1000;

/// Reads the bodies of the lines among `keys` the window does not hold, into the window; false when
/// the store did not answer.
async fn fetch_missing(rt: &Runtime, pg: &antirion_store::pg::PgStore, keys: Vec<(String, u32)>) -> bool {
    let book = rt.accounts.book();
    let missing: Vec<(String, u32)> = keys
        .into_iter()
        .filter(|(id, line)| !book.holds(id, *line))
        .collect();
    for chunk in missing.chunks(BODIES_PER_READ) {
        let Ok(rows) = pg.ledger_lines(chunk).await else {
            return false;
        };
        let lines: Vec<LedgerLine> = rows
            .iter()
            .filter_map(|r| serde_json::from_str::<Value>(&r.body).ok())
            .filter_map(|v| LedgerLine::from_json(&v))
            .collect();
        rt.record_stored(lines);
    }
    true
}

pub(crate) async fn run(rt: Arc<Runtime>) {
    if rt.cluster.is_none() {
        return;
    }
    let Some(pg) = rt.store.postgres() else { return };
    let start = loop {
        match pg.now_us().await {
            Ok(t) => break t,
            Err(_) if pause(&rt, EVERY).await => {}
            Err(_) => return,
        }
    };
    // The store time of the newest line read.
    let mut at = start;
    let mut tick = 0u64;
    while pause(&rt, EVERY).await {
        tick += 1;
        if pg.is_down() {
            continue;
        }
        // New lines; a read cut by the limit continues from its newest line at the next tick.
        if let Ok((keys, newest)) = pg.ledger_keys_since(at, None, READ_LIMIT).await
            && fetch_missing(&rt, pg, keys).await
        {
            at = at.max(newest);
        }
        // Lines that committed after a newer one was read, or that shared the store time a cut read
        // stopped at.
        if tick.is_multiple_of(OVERLAP_EVERY)
            && let Ok((keys, _)) = pg
                .ledger_keys_since(at - OVERLAP_US, Some(at), OVERLAP_LIMIT)
                .await
        {
            fetch_missing(&rt, pg, keys).await;
        }
        if tick.is_multiple_of(4)
            && let Ok(epochs) = pg.epochs().await
        {
            let now_ms = rt.now().0 / 1000;
            for (k, e) in epochs {
                rt.state
                    .cache
                    .raise_from_store(&k, u64::try_from(e).unwrap_or(0), now_ms)
                    .await;
            }
        }
    }
}
