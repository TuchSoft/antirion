//! Degraded mode: the counter store's health, the node's local share of each counter while the
//! store is unavailable, and the replay queue of settlements that could not reach it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::counter::{Hold, Rule};
use crate::store::ReservationId;

/// Consecutive good probes before the store is used again.
pub const PROBES_TO_RECOVER: u8 = 3;

/// Why a request was evaluated in a degraded mode (the log record's `degraded` cause).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DegradedCause {
    LocalShare,
    FailOpen,
    /// A settlement was queued because the store did not answer.
    StoreTimeout,
}

impl DegradedCause {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DegradedCause::LocalShare => "local_share",
            DegradedCause::FailOpen => "fail_open",
            DegradedCause::StoreTimeout => "store_timeout",
        }
    }
}

/// The degraded components of one request: `limits` and `budgets` in `x-antirion-degraded`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Degraded {
    pub limits: Option<DegradedCause>,
    pub budgets: Option<DegradedCause>,
}

impl Degraded {
    pub(crate) fn mark(&mut self, budget: bool, cause: DegradedCause) {
        let slot = if budget {
            &mut self.budgets
        } else {
            &mut self.limits
        };
        if slot.is_none() {
            *slot = Some(cause);
        }
    }

    /// Adds the components `other` names that this one does not.
    pub fn merge(&mut self, other: Degraded) {
        self.limits = self.limits.or(other.limits);
        self.budgets = self.budgets.or(other.budgets);
    }

    /// Each degraded component with its cause.
    #[must_use]
    pub fn causes(&self) -> Vec<(&'static str, &'static str)> {
        let mut v = Vec::new();
        if let Some(c) = self.limits {
            v.push(("limits", c.as_str()));
        }
        if let Some(c) = self.budgets {
            v.push(("budgets", c.as_str()));
        }
        v
    }

    /// The component names for `x-antirion-degraded`.
    #[must_use]
    pub fn components(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.limits.is_some() {
            v.push("limits");
        }
        if self.budgets.is_some() {
            v.push("budgets");
        }
        v
    }

    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.limits.is_some() || self.budgets.is_some()
    }
}

/// When the counter store counts as unavailable
/// (`cluster.counters.redis.failure_detection`): within the trailing `window`, at least
/// `min_failures` calls failed and failures are at least `failure_rate` of the calls. A short stall
/// fails the calls it holds but leaves the rate low, so it is never taken for an outage.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FailureDetection {
    pub window: Duration,
    pub failure_rate: f64,
    pub min_failures: u32,
}

impl Default for FailureDetection {
    fn default() -> FailureDetection {
        FailureDetection {
            window: Duration::from_secs(1),
            failure_rate: 0.5,
            min_failures: 5,
        }
    }
}

/// Buckets of the trailing window.
const BUCKETS: u32 = 10;

/// Calls and failures in one bucket of the window.
#[derive(Copy, Clone, Debug, Default)]
struct Bucket {
    /// The bucket's index since the tracker started; stale buckets count nothing.
    at: u64,
    calls: u32,
    failed: u32,
}

#[derive(Debug)]
struct Tracker {
    started: Instant,
    buckets: [Bucket; BUCKETS as usize],
    /// Consecutive good probes while down.
    probes: u8,
}

/// Whether request-path commands go to the store: down once the failures of the trailing window
/// meet [`FailureDetection`], until [`PROBES_TO_RECOVER`] probes succeed in a row. While down,
/// calls are not sent and every counter follows its `on_store_unavailable`.
#[derive(Debug)]
pub(crate) struct Health {
    down: AtomicBool,
    detection: FailureDetection,
    tracker: Mutex<Tracker>,
}

impl Health {
    pub(crate) fn new(detection: FailureDetection) -> Health {
        Health {
            down: AtomicBool::new(false),
            detection,
            tracker: Mutex::new(Tracker {
                started: Instant::now(),
                buckets: [Bucket::default(); BUCKETS as usize],
                probes: 0,
            }),
        }
    }

    pub(crate) fn is_down(&self) -> bool {
        self.down.load(Ordering::Acquire)
    }

    fn bucket_width(&self) -> Duration {
        (self.detection.window / BUCKETS).max(Duration::from_micros(1))
    }

    /// Records the outcome of a call; true when it marked the store down.
    pub(crate) fn record(&self, ok: bool) -> bool {
        if self.is_down() {
            return false;
        }
        let width = self.bucket_width().as_micros();
        let mut t = self.tracker.lock();
        let now = u64::try_from(t.started.elapsed().as_micros() / width).unwrap_or(u64::MAX);
        let i = usize::try_from(now % u64::from(BUCKETS)).unwrap_or(0);
        if let Some(b) = t.buckets.get_mut(i) {
            if b.at != now {
                *b = Bucket {
                    at: now,
                    ..Bucket::default()
                };
            }
            b.calls = b.calls.saturating_add(1);
            b.failed = b.failed.saturating_add(u32::from(!ok));
        }
        if ok {
            return false;
        }
        let live = |b: &&Bucket| now.saturating_sub(b.at) < u64::from(BUCKETS);
        let (calls, failed) = t.buckets.iter().filter(live).fold((0u64, 0u64), |(c, f), b| {
            (c + u64::from(b.calls), f + u64::from(b.failed))
        });
        #[allow(clippy::cast_precision_loss)]
        let rate = failed as f64 / calls.max(1) as f64;
        if failed >= u64::from(self.detection.min_failures) && rate >= self.detection.failure_rate {
            t.buckets = [Bucket::default(); BUCKETS as usize];
            t.probes = 0;
            self.down.store(true, Ordering::Release);
            return true;
        }
        false
    }

    /// Records a probe; true when the store is up afterwards.
    pub(crate) fn probed(&self, ok: bool) -> bool {
        if !self.is_down() {
            return true;
        }
        let mut t = self.tracker.lock();
        if !ok {
            t.probes = 0;
            return false;
        }
        t.probes = t.probes.saturating_add(1);
        if t.probes < PROBES_TO_RECOVER {
            return false;
        }
        t.probes = 0;
        t.buckets = [Bucket::default(); BUCKETS as usize];
        self.down.store(false, Ordering::Release);
        true
    }
}

/// The local form of a hold for a node's share `1/m` of the quota, and the factor its amounts
/// are multiplied by. Quotas are kept and amounts multiplied by `m`, so a fractional share stays
/// exact; concurrency takes `floor(L/m)` slots instead.
#[must_use]
pub fn local_share(h: &Hold, m: u32, budget_remaining: Option<u128>) -> Option<(Hold, u128)> {
    let m = u128::from(m.max(1));
    let mut local = h.clone();
    let scale = match &h.rule {
        Rule::Gcra(_) | Rule::Window(_) => m,
        Rule::Concurrency { limit } => {
            local.rule = Rule::Concurrency {
                limit: u64::try_from(u128::from(*limit) / m).unwrap_or(0),
            };
            1
        }
        Rule::Budget { period, .. } => {
            local.rule = Rule::Budget {
                amount: budget_remaining?,
                period: *period,
            };
            m
        }
        Rule::Rolling(w) => {
            local.rule = Rule::Rolling(crate::math::Window {
                limit: u64::try_from(budget_remaining?).unwrap_or(u64::MAX),
                ..*w
            });
            m
        }
    };
    local.amount = h.amount.saturating_mul(scale);
    Some((local, scale))
}

/// A settlement waiting for the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Queued {
    Reconcile {
        uid: ReservationId,
        node: String,
        group: u128,
        holds: Vec<Hold>,
        actual: Vec<u128>,
    },
    /// Consumption the shared store did not see: a soft budget's spend, or what a local share
    /// admitted while the store was down. Replayed as an admission of `hold` and its settlement
    /// under a derived id, so a retry never charges twice.
    Track {
        uid: ReservationId,
        node: String,
        group: u128,
        hold: Hold,
        actual: u128,
    },
}

/// A bounded queue; when full, the oldest entry is dropped and counted.
#[derive(Debug)]
pub(crate) struct ReplayQueue {
    items: Mutex<VecDeque<Queued>>,
    capacity: usize,
    dropped: Mutex<u64>,
}

impl ReplayQueue {
    pub(crate) fn new(capacity: usize) -> ReplayQueue {
        ReplayQueue {
            items: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
            dropped: Mutex::new(0),
        }
    }

    pub(crate) fn push(&self, q: Queued) {
        let mut items = self.items.lock();
        if items.len() >= self.capacity {
            items.pop_front();
            *self.dropped.lock() += 1;
        }
        items.push_back(q);
    }

    pub(crate) fn pop(&self) -> Option<Queued> {
        self.items.lock().pop_front()
    }

    pub(crate) fn push_front(&self, q: Queued) {
        self.items.lock().push_front(q);
    }

    pub(crate) fn len(&self) -> usize {
        self.items.lock().len()
    }

    pub(crate) fn dropped(&self) -> u64 {
        *self.dropped.lock()
    }
}

#[cfg(test)]
mod tests {
    use antirion_core::{ResourceUid, UtcMicros};

    use super::*;
    use crate::counter::{CounterId, HoldMode};
    use crate::level::Level;
    use crate::math::Gcra;
    use crate::period::PeriodBounds;

    fn hold(rule: Rule, amount: u128) -> Hold {
        Hold {
            counter: CounterId::derive("t", Level::Key, ResourceUid(1), "", "x", "", &[]),
            group: 0,
            rule,
            amount,
            mode: HoldMode::Enforce,
        }
    }

    /// The store is out of use only when enough calls failed and they are a large enough share
    /// of the window's calls; failures older than the window are forgotten.
    #[test]
    #[allow(clippy::disallowed_methods)] // a plain test thread, not a runtime worker
    fn failures_take_the_store_down_by_count_and_rate_within_the_window() {
        let h = Health::new(FailureDetection {
            window: Duration::from_millis(100),
            ..FailureDetection::default()
        });
        for _ in 0..4 {
            assert!(!h.record(false));
        }
        std::thread::sleep(Duration::from_millis(150));
        for _ in 0..4 {
            assert!(!h.record(false), "the first four are out of the window");
        }
        assert!(h.record(false) && h.is_down());
        assert!(!h.probed(true) && !h.probed(true) && h.probed(true) && !h.is_down());

        let h = Health::new(FailureDetection::default());
        for _ in 0..100 {
            h.record(true);
        }
        for _ in 0..99 {
            assert!(!h.record(false), "fewer failures than successes");
        }
        assert!(h.record(false), "100 of 200 calls failed");
    }

    #[test]
    fn shares() {
        let g = hold(
            Rule::Gcra(Gcra {
                limit: 10,
                per_us: 1,
                burst: 10,
            }),
            1,
        );
        let (l, s) = local_share(&g, 2, None).unwrap();
        assert_eq!((l.amount, s), (2, 2));
        let c = hold(Rule::Concurrency { limit: 2 }, 1);
        let (l, _) = local_share(&c, 3, None).unwrap();
        assert_eq!(l.rule, Rule::Concurrency { limit: 0 });
        let b = hold(
            Rule::Budget {
                amount: 10,
                period: PeriodBounds {
                    start: UtcMicros(0),
                    end: None,
                },
            },
            2,
        );
        assert!(local_share(&b, 2, None).is_none());
        let (l, _) = local_share(&b, 2, Some(7)).unwrap();
        assert_eq!(
            (l.amount, l.rule.clone()),
            (
                4,
                Rule::Budget {
                    amount: 7,
                    period: PeriodBounds {
                        start: UtcMicros(0),
                        end: None
                    }
                }
            )
        );
    }

    #[test]
    fn health_needs_three_good_probes() {
        let h = Health::new(FailureDetection::default());
        assert!(!h.is_down());
        while !h.record(false) {}
        assert!(h.is_down());
        assert!(!h.probed(true));
        assert!(!h.probed(false));
        assert!(!h.probed(true));
        assert!(!h.probed(true));
        assert!(h.probed(true));
        assert!(!h.is_down());
    }

    #[test]
    fn queue_is_bounded() {
        let q = ReplayQueue::new(2);
        for i in 0..3 {
            q.push(Queued::Reconcile {
                uid: ReservationId(i),
                node: "n".into(),
                group: 0,
                holds: vec![],
                actual: vec![],
            });
        }
        assert_eq!((q.len(), q.dropped()), (2, 1));
        assert!(matches!(
            q.pop(),
            Some(Queued::Reconcile {
                uid: ReservationId(1),
                ..
            })
        ));
        let mut d = Degraded::default();
        d.mark(false, DegradedCause::LocalShare);
        d.mark(false, DegradedCause::FailOpen);
        assert_eq!(d.limits, Some(DegradedCause::LocalShare));
        assert_eq!(d.components(), vec!["limits"]);
    }
}
