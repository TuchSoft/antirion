//! The node's spend ledger: lines enter the accounts' window and the writer together, budget
//! rebuilds and spend reports read the window and the store as one ledger.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use antirion_limits::book::{Book, LedgerFilter, LedgerHistory};
use antirion_limits::counter::CounterId;
use antirion_limits::ledger::LedgerLine;
use antirion_limits::period::PeriodBounds;
use antirion_store::{LedgerPage, LedgerPos, LedgerRow, Store, StoreError};

use crate::Runtime;
use crate::spool::LedgerWriter;

/// Budget rebuilds over the store: its charges plus the window's lines it does not hold yet,
/// read while no flush can move a line from one to the other.
#[derive(Debug)]
pub(crate) struct StoreHistory {
    pub(crate) store: Arc<Store>,
    pub(crate) writer: Arc<LedgerWriter>,
}

impl LedgerHistory for StoreHistory {
    fn charged<'a>(
        &'a self,
        book: &'a Book,
        identity: CounterId,
        period: PeriodBounds,
    ) -> Pin<Box<dyn Future<Output = Result<i128, antirion_limits::store::StoreError>> + Send + 'a>> {
        Box::pin(async move {
            let _held = self.writer.hold().await;
            let unstored = book.unstored_charged(identity, period);
            let stored = self
                .store
                .ledger_charged(&identity.hex(), period.start.0, period.end.map(|e| e.0))
                .await
                .map_err(|e| antirion_limits::store::StoreError::Unavailable(e.to_string()))?;
            Ok(stored.saturating_add(unstored))
        })
    }

    fn charged_slots<'a>(
        &'a self,
        book: &'a Book,
        identity: CounterId,
        rolling: antirion_limits::math::Window,
        from: antirion_core::UtcMicros,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<antirion_limits::book::Slots, antirion_limits::store::StoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let _held = self.writer.hold().await;
            let mut slots = book.unstored_charged_slots(identity, rolling, from);
            let stored = self
                .store
                .ledger_charged_slots(
                    &identity.hex(),
                    from.0,
                    i64::try_from(rolling.per_us / u64::from(rolling.precision.max(1))).unwrap_or(i64::MAX),
                )
                .await
                .map_err(|e| antirion_limits::store::StoreError::Unavailable(e.to_string()))?;
            for (k, v) in stored {
                let slot = slots.entry(k).or_insert(0);
                *slot = slot.saturating_add(v);
            }
            Ok(slots)
        })
    }
}

/// Stored lines read per store call when the ledger is walked.
const PAGE: usize = 2_000;

/// A line's place in the ledger: admission, request, line.
fn order_key(l: &LedgerLine) -> (antirion_core::UtcMicros, &str, u32) {
    (l.admitted_at, l.request_id.as_str(), l.line)
}

pub(crate) fn decode(rows: &[LedgerRow]) -> Vec<Arc<LedgerLine>> {
    rows.iter()
        .filter_map(|r| serde_json::from_str::<serde_json::Value>(&r.body).ok())
        .filter_map(|v| LedgerLine::from_json(&v))
        .map(Arc::new)
        .collect()
}

/// `window` and `stored` as one list: each `(request_id, line)` once, oldest admission first.
pub(crate) fn merged(window: Vec<Arc<LedgerLine>>, stored: Vec<Arc<LedgerLine>>) -> Vec<Arc<LedgerLine>> {
    let mut seen = HashSet::with_capacity(window.len() + stored.len());
    let mut out: Vec<Arc<LedgerLine>> = window
        .into_iter()
        .chain(stored)
        .filter(|l| seen.insert((l.request_id.clone(), l.line)))
        .collect();
    out.sort_by(|a, b| (a.admitted_at, &a.request_id, a.line).cmp(&(b.admitted_at, &b.request_id, b.line)));
    out
}

impl Runtime {
    /// Appends settled ledger lines; the new ones are written with the next ledger flush.
    pub fn record_spend(&self, lines: Vec<LedgerLine>) {
        let lines: Vec<(LedgerLine, LedgerRow)> = lines
            .into_iter()
            .map(|l| {
                let row = LedgerRow {
                    request_id: l.request_id.clone(),
                    line: l.line,
                    admitted_us: l.admitted_at.0,
                    body: l.to_json_string(),
                    charges: Some(!l.budgets.is_empty()),
                };
                (l, row)
            })
            .collect();
        let writer = &self.ledger;
        self.accounts.book().append(lines, false, |rows, below| {
            writer.push(rows, below);
        });
    }

    /// Appends lines read from the store (other nodes' lines in cluster mode).
    pub(crate) fn record_stored(&self, lines: Vec<LedgerLine>) {
        self.accounts
            .book()
            .append(lines.into_iter().map(|l| (l, ())).collect(), true, |_, _| {});
    }

    /// Writes the pending ledger lines (see [`LedgerWriter::flush`]).
    pub async fn flush_ledger(&self) {
        let book = self.accounts.book();
        self.ledger
            .flush(&self.store, |below| book.mark_durable(below))
            .await;
    }

    /// Checks that the stored ledger can be read; budget rebuilds and reports read it on demand.
    pub async fn check_ledger(&self) -> Result<(), StoreError> {
        self.store.ledger_charged("", 0, Some(0)).await.map(|_| ())
    }

    /// Every line of one request, by line number.
    pub async fn spend_of_request(&self, request_id: &str) -> Vec<Arc<LedgerLine>> {
        let window = self.accounts.book().request(request_id);
        let stored = match self.store.ledger_request(request_id).await {
            Ok(rows) => decode(&rows),
            Err(e) => {
                tracing::warn!(error = %e, "the stored ledger could not be read; reporting the node's recent lines");
                Vec::new()
            }
        };
        let mut lines = merged(window, stored);
        lines.retain(|l| !self.erased(l));
        lines.sort_by_key(|l| l.line);
        lines
    }

    /// Whether a line belongs to a tenant erased since it was admitted.
    pub(crate) fn erased(&self, l: &LedgerLine) -> bool {
        self.state.operations.erased.hides(&l.organization, l.admitted_at)
    }

    /// Calls `visit` with each line a filter accepts, the window's and the store's as one ledger
    /// (each `(request_id, line)` once, oldest admission first, or the reverse when `descending`),
    /// until it returns false. The store is read a page at a time, so only the window and one page
    /// are held at once. A standalone node's window answers alone when it covers the filter's
    /// range.
    pub async fn for_each_spend_line(
        &self,
        f: &LedgerFilter,
        descending: bool,
        visit: impl FnMut(&Arc<LedgerLine>) -> bool + Send,
    ) {
        let book = self.accounts.book();
        let covered = self.cluster.is_none() && f.from.is_some_and(|from| book.covers(from));
        let store = (!covered).then_some(self.store.as_ref());
        walk(
            store,
            book.select(f),
            f,
            descending,
            PAGE,
            |l| !self.erased(l),
            visit,
        )
        .await;
    }
}

/// Walks `window` (the window's lines `f` accepts) and, unless `store` is `None`, the stored lines
/// `f` accepts, as one ledger in the order of [`merged`] (reversed when `descending`), keeping the
/// lines `keep` accepts; reads the store `page` lines at a time and stops when `visit` returns
/// false.
async fn walk(
    store: Option<&Store>,
    mut window: Vec<Arc<LedgerLine>>,
    f: &LedgerFilter,
    descending: bool,
    page: usize,
    keep: impl Fn(&LedgerLine) -> bool + Send,
    mut visit: impl FnMut(&Arc<LedgerLine>) -> bool + Send,
) {
    window.retain(|l| keep(l));
    window.sort_by(|a, b| order_key(a).cmp(&order_key(b)));
    if descending {
        window.reverse();
    }
    // Whether `a` comes before `b` in the walk.
    let before = |a: &LedgerLine, b: &LedgerLine| {
        let ord = order_key(a).cmp(&order_key(b));
        if descending { ord.is_gt() } else { ord.is_lt() }
    };
    let held: HashSet<(String, u32)> = match store {
        Some(_) => window.iter().map(|l| (l.request_id.clone(), l.line)).collect(),
        None => HashSet::new(),
    };
    let mut window = window.into_iter().peekable();
    if let Some(store) = store {
        let mut after = None;
        loop {
            let rows = match store
                .ledger_page(LedgerPage {
                    from: f.from.map(|t| t.0),
                    to: f.to.map(|t| t.0),
                    descending,
                    after: after.take(),
                    limit: page,
                })
                .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    tracing::warn!(error = %e, "the stored ledger could not be read; reporting the node's recent lines");
                    break;
                }
            };
            let full = rows.len() == page;
            after = rows.last().map(LedgerPos::of);
            for l in decode(&rows) {
                if !f.accepts(&l) || held.contains(&(l.request_id.clone(), l.line)) || !keep(&l) {
                    continue;
                }
                while let Some(w) = window.next_if(|w| before(w, &l)) {
                    if !visit(&w) {
                        return;
                    }
                }
                if !visit(&l) {
                    return;
                }
            }
            if !full {
                break;
            }
        }
    }
    for w in window {
        if !visit(&w) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use antirion_core::UtcMicros;
    use antirion_limits::cost::CostStatus;
    use antirion_limits::estimate::OutSource;
    use antirion_limits::ledger::{BudgetCharge, LineStatus, Purpose, Reserved, UsageSource};
    use antirion_limits::money::Nusd;
    use antirion_limits::usage::Usage;

    use super::*;
    use crate::spool::SpoolSettings;

    const ID: CounterId = CounterId([9; 16]);

    fn line(request: &str, at: i64, amount: i128) -> LedgerLine {
        LedgerLine {
            request_id: request.into(),
            line: 0,
            purpose: Purpose::Primary,
            shadow: false,
            status: LineStatus::Success,
            admitted_at: UtcMicros(at),
            settled_at: UtcMicros(at),
            organization: "o".into(),
            team: "t".into(),
            project: "p".into(),
            key: "k".into(),
            user: None,
            end_user: None,
            tags: vec![],
            model: None,
            route: None,
            provider: None,
            deployment: None,
            upstream_model: None,
            call: antirion_limits::ledger::CallTarget::default(),
            endpoint: "chat_completions".into(),
            stream: false,
            usage: Usage::default(),
            usage_raw: None,
            usage_source: UsageSource::Upstream,
            cost: Some(Nusd(amount)),
            billed: Some(Nusd(amount)),
            cost_status: CostStatus::Exact,
            price_version: None,
            service_tier: None,
            avoided: None,
            reserved: Reserved {
                input: 0,
                output: 0,
                usd: None,
                out_source: OutSource::Request,
            },
            budgets: vec![BudgetCharge {
                level: "key".into(),
                owner: "k".into(),
                name: "b".into(),
                identity: ID,
                amount,
            }],
        }
    }

    fn record(book: &Book, writer: &LedgerWriter, l: LedgerLine) {
        let row = LedgerRow {
            request_id: l.request_id.clone(),
            line: l.line,
            admitted_us: l.admitted_at.0,
            body: l.to_json_string(),
            charges: Some(!l.budgets.is_empty()),
        };
        book.append(vec![(l, row)], false, |rows, below| writer.push(rows, below));
    }

    #[tokio::test]
    async fn rebuilds_count_each_line_once_whether_stored_or_waiting() {
        let dir = std::env::temp_dir().join(format!(
            "antirion-ledger-{:x}",
            antirion_core::ResourceUid::random().0
        ));
        let store = Arc::new(Store::open_embedded(&dir.join("store")).unwrap());
        let writer = Arc::new(LedgerWriter::new(SpoolSettings {
            dir: dir.join("spool"),
            max_bytes: 1 << 20,
            reject_budgeted: false,
            flush_interval: Duration::from_secs(1),
        }));
        let history = StoreHistory {
            store: Arc::clone(&store),
            writer: Arc::clone(&writer),
        };
        let book = Book::new(UtcMicros(1_000), 2);
        let all = PeriodBounds {
            start: UtcMicros::EPOCH,
            end: None,
        };
        let flush = || writer.flush(&store, |below| book.mark_durable(below));
        for (i, amount) in [100, 20, 3].into_iter().enumerate() {
            record(&book, &writer, line(&format!("r{i}"), 2_000 + i as i64, amount));
        }
        assert_eq!(history.charged(&book, ID, all).await.unwrap(), 123);
        flush().await;
        assert_eq!(book.len(), 2, "stored lines leave the window");
        record(&book, &writer, line("r3", 2_100, 4000));
        assert_eq!(history.charged(&book, ID, all).await.unwrap(), 4123);
        flush().await;
        assert_eq!(history.charged(&book, ID, all).await.unwrap(), 4123);
        let late = PeriodBounds {
            start: UtcMicros(2_001),
            end: Some(UtcMicros(2_100)),
        };
        assert_eq!(history.charged(&book, ID, late).await.unwrap(), 23);
        let stored = decode(&store.ledger().await.unwrap());
        let window = book.select(&LedgerFilter::default());
        let all_lines = merged(window, stored);
        let ids: Vec<&str> = all_lines.iter().map(|l| l.request_id.as_str()).collect();
        assert_eq!(ids, ["r0", "r1", "r2", "r3"]);
        store.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What the walk must give: the old materialised read, every line at once.
    fn expected(
        book: &Book,
        stored: &[LedgerRow],
        f: &LedgerFilter,
        keep: impl Fn(&LedgerLine) -> bool,
    ) -> Vec<(String, u32)> {
        let stored = decode(stored).into_iter().filter(|l| f.accepts(l)).collect();
        let mut lines = merged(book.select(f), stored);
        lines.retain(|l| keep(l));
        lines.iter().map(|l| (l.request_id.clone(), l.line)).collect()
    }

    #[tokio::test]
    async fn the_ledger_is_walked_a_page_at_a_time_as_one_list() {
        let dir = std::env::temp_dir().join(format!(
            "antirion-ledger-walk-{:x}",
            antirion_core::ResourceUid::random().0
        ));
        let store = Store::open_embedded(&dir.join("store")).unwrap();
        let book = Book::new(UtcMicros(1_000), 100);
        let mut rows = Vec::new();
        for i in 0..40i64 {
            for n in 0..(1 + i % 3) {
                let mut l = line(&format!("r{:02}", (i * 7) % 40), 2_000 + i / 4, 1);
                l.line = u32::try_from(n).unwrap();
                l.team = if i % 5 == 0 { "other".into() } else { "t".into() };
                let row = LedgerRow {
                    request_id: l.request_id.clone(),
                    line: l.line,
                    admitted_us: l.admitted_at.0,
                    body: l.to_json_string(),
                    charges: Some(true),
                };
                // Some lines only in the store, some only in the window, some in both.
                if i % 4 != 3 {
                    rows.push(row.clone());
                }
                if i % 4 != 0 {
                    book.append(vec![(l, ())], i % 4 == 2, |_, _| {});
                }
            }
        }
        store.append_ledger(rows).await.unwrap();
        let filters = [
            LedgerFilter::default(),
            LedgerFilter {
                team: Some("t".into()),
                from: Some(UtcMicros(2_002)),
                to: Some(UtcMicros(2_008)),
                ..LedgerFilter::default()
            },
        ];
        let keep = |l: &LedgerLine| l.request_id != "r14";
        for f in &filters {
            let stored = store
                .ledger_between(f.from.map(|t| t.0), f.to.map(|t| t.0))
                .await
                .unwrap();
            let want = expected(&book, &stored, f, keep);
            for page in [1, 2, 3, 1_000] {
                for descending in [false, true] {
                    let mut got = Vec::new();
                    walk(Some(&store), book.select(f), f, descending, page, keep, |l| {
                        got.push((l.request_id.clone(), l.line));
                        true
                    })
                    .await;
                    if descending {
                        got.reverse();
                    }
                    assert_eq!(got, want, "page {page}, descending {descending}");
                }
                let mut first = Vec::new();
                walk(Some(&store), book.select(f), f, false, page, keep, |l| {
                    first.push((l.request_id.clone(), l.line));
                    first.len() < 5
                })
                .await;
                assert_eq!(first, want[..5], "a walk stops when told");
            }
            let mut window_only = Vec::new();
            walk(None, book.select(f), f, false, 2, keep, |l| {
                window_only.push((l.request_id.clone(), l.line));
                true
            })
            .await;
            assert_eq!(window_only, expected(&book, &[], f, keep));
        }
        store.close();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
