//! The closed metric set of alert rules, computed from request-log records: one pass over the
//! records of a window and scope, so evaluation reads what the request path already wrote and adds
//! nothing to it.

use std::collections::BTreeMap;

use antirion_core::UtcMicros;
use serde_json::Value;

const MINUTE_US: i64 = 60_000_000;
const HOUR_US: i64 = 60 * MINUTE_US;
const WEEK_US: i64 = 7 * 24 * HOUR_US;

/// The record's start, in microseconds.
#[must_use]
pub fn started_us(r: &Value) -> Option<i64> {
    r.get("started_at")
        .and_then(Value::as_str)
        .and_then(|s| UtcMicros::parse_rfc3339(s).ok())
        .map(|t| t.0)
}

/// The dimensions of a scope that records carry: an instance's scope may also name what it is about
/// (`anomaly`, `budget`, `store`), which no record has.
const RECORD_DIMENSIONS: &[&str] = &[
    "organization",
    "team",
    "project",
    "key",
    "user",
    "end_user",
    "provider",
    "deployment",
    "model",
    "route",
    "environment",
];

/// `scope` without the dimensions records do not carry; `None` when nothing is left to match, so
/// that a scope about something else counts no request.
#[must_use]
pub fn record_scope(scope: &BTreeMap<String, String>) -> Option<BTreeMap<String, String>> {
    let kept: BTreeMap<String, String> = scope
        .iter()
        .filter(|(k, _)| RECORD_DIMENSIONS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (!kept.is_empty() || scope.is_empty()).then_some(kept)
}

/// Whether a record falls in `scope` (every dimension must match the record's own field).
#[must_use]
pub fn in_scope(r: &Value, scope: &BTreeMap<String, String>) -> bool {
    scope
        .iter()
        .all(|(k, v)| r.get(k.as_str()).and_then(Value::as_str) == Some(v.as_str()))
}

fn status(r: &Value) -> u64 {
    r.get("status_code").and_then(Value::as_u64).unwrap_or(0)
}

fn cost(r: &Value) -> f64 {
    r.pointer("/cost/total")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// The `p`-th percentile (nearest rank) of `values`, which it sorts.
#[must_use]
pub fn percentile(values: &mut [f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let rank = ((p / 100.0) * values.len() as f64).ceil() as usize;
    values.get(rank.saturating_sub(1)).copied()
}

#[allow(clippy::cast_precision_loss)]
fn ratio(n: usize, d: usize) -> Option<f64> {
    (d > 0).then(|| n as f64 / d as f64)
}

fn mean(values: &[f64]) -> Option<f64> {
    #[allow(clippy::cast_precision_loss)]
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// The value a float sum starts from, so a running sum equals the iterator's `sum` of the same
/// values in the same order, an empty one included.
fn zero() -> f64 {
    std::iter::empty::<f64>().sum()
}

/// Requests, errors and throttled requests of a set of records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Impact {
    pub requests: u64,
    pub errors: u64,
    pub throttled: u64,
}

impl Impact {
    pub fn add(&mut self, r: &Value) {
        self.requests += 1;
        match status(r) {
            s if s >= 500 => self.errors += 1,
            429 => self.throttled += 1,
            _ => {}
        }
    }
}

#[must_use]
pub fn impact<'a>(records: impl Iterator<Item = &'a Value>) -> Impact {
    let mut i = Impact::default();
    for r in records {
        i.add(r);
    }
    i
}

/// Per-minute request and token limits that apply to a scope, for the utilization metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Limits {
    pub rpm: Option<f64>,
    pub tpm: Option<f64>,
}

/// What the metrics read of a set of records, gathered one record at a time, so an evaluation
/// never holds the records themselves. Latencies are kept only for the metrics that rank or
/// average them.
#[derive(Clone, Debug)]
pub struct Tally {
    pub impact: Impact,
    keep_total: bool,
    keep_ttft: bool,
    total_ms: Vec<f64>,
    ttft_ms: Vec<f64>,
    attempts: usize,
    failed_attempts: usize,
    fell_back: usize,
    cost: f64,
    guardrail_events: usize,
    cache_looked: usize,
    cache_hits: usize,
    tokens: f64,
}

impl Tally {
    /// An empty tally for `metric`.
    #[must_use]
    pub fn new(metric: &str) -> Tally {
        Tally {
            impact: Impact::default(),
            keep_total: matches!(
                metric,
                "p95_latency_ms" | "p99_latency_ms" | "latency_baseline_ratio"
            ),
            keep_ttft: metric == "ttft_p95_ms",
            total_ms: Vec::new(),
            ttft_ms: Vec::new(),
            attempts: 0,
            failed_attempts: 0,
            fell_back: 0,
            cost: zero(),
            guardrail_events: 0,
            cache_looked: 0,
            cache_hits: 0,
            tokens: zero(),
        }
    }

    /// The tally of `records` for `metric`.
    #[must_use]
    pub fn of<'a>(metric: &str, records: impl IntoIterator<Item = &'a Value>) -> Tally {
        let mut t = Tally::new(metric);
        for r in records {
            t.add(r);
        }
        t
    }

    /// Counts one record.
    pub fn add(&mut self, r: &Value) {
        self.impact.add(r);
        if self.keep_total
            && let Some(ms) = r.pointer("/timing/total_ms").and_then(Value::as_f64)
        {
            self.total_ms.push(ms);
        }
        if self.keep_ttft
            && let Some(ms) = r.pointer("/timing/ttft_ms").and_then(Value::as_f64)
        {
            self.ttft_ms.push(ms);
        }
        if let Some(attempts) = r.get("attempts").and_then(Value::as_array) {
            self.attempts += attempts.len();
            self.failed_attempts += attempts
                .iter()
                .filter(|a| {
                    a.get("status_code")
                        .and_then(Value::as_u64)
                        .is_some_and(|s| s >= 500)
                })
                .count();
            if attempts
                .iter()
                .any(|a| a.get("reason").and_then(Value::as_str) == Some("fallback"))
            {
                self.fell_back += 1;
            }
        }
        self.cost += cost(r);
        self.guardrail_events += r
            .get("guardrails")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|g| {
                !matches!(
                    g.get("verdict").and_then(Value::as_str),
                    Some("pass" | "skipped") | None
                )
            })
            .count();
        if let Some(s) = r.pointer("/cache/status").and_then(Value::as_str)
            && matches!(s, "hit" | "miss" | "refresh")
        {
            self.cache_looked += 1;
            if s == "hit" {
                self.cache_hits += 1;
            }
        }
        if let Some(n) = r.pointer("/usage/total").and_then(Value::as_f64) {
            self.tokens += n;
        }
    }

    fn requests(&self) -> usize {
        usize::try_from(self.impact.requests).unwrap_or(usize::MAX)
    }

    fn mean_cost(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        (self.requests() > 0).then(|| self.cost / self.requests() as f64)
    }
}

/// What a metric needs besides the window's records.
#[derive(Debug)]
pub struct Inputs<'a> {
    /// The tally of the window's records in scope.
    pub window: &'a Tally,
    pub window_us: i64,
    /// The tally of the baseline hour's records (the same hour of the week, a week earlier), in
    /// scope.
    pub baseline: &'a Tally,
    pub limits: Limits,
    /// `(spent in the period so far, amount, elapsed of the period, remaining of the period)` of each
    /// budget in scope, in USD and microseconds.
    pub budgets: &'a [(f64, f64, i64, i64)],
}

/// The value of `metric`, or `None` when the window holds nothing to compute it from.
#[must_use]
pub fn compute(metric: &str, x: &Inputs<'_>) -> Option<f64> {
    let w = x.window;
    #[allow(clippy::cast_precision_loss)]
    let minutes = (x.window_us as f64 / MINUTE_US as f64).max(1.0 / 60.0);
    #[allow(clippy::cast_precision_loss)]
    let requests = w.requests() as f64;
    match metric {
        "requests" => Some(requests),
        "p95_latency_ms" => percentile(&mut w.total_ms.clone(), 95.0),
        "p99_latency_ms" => percentile(&mut w.total_ms.clone(), 99.0),
        "ttft_p95_ms" => percentile(&mut w.ttft_ms.clone(), 95.0),
        "error_rate" => ratio(
            usize::try_from(w.impact.errors).unwrap_or(usize::MAX),
            w.requests(),
        ),
        "upstream_5xx_rate" => ratio(w.failed_attempts, w.attempts),
        "fallback_rate" => ratio(w.fell_back, w.requests()),
        "cost_per_request_usd" => w.mean_cost(),
        "spend_usd" => Some(w.cost),
        "guardrail_events" =>
        {
            #[allow(clippy::cast_precision_loss)]
            Some(w.guardrail_events as f64)
        }
        "cache_hit_rate" => ratio(w.cache_hits, w.cache_looked),
        "rpm_utilization" => {
            let rate = requests / minutes;
            x.limits.rpm.filter(|l| *l > 0.0).map(|l| rate / l)
        }
        "tpm_utilization" => x.limits.tpm.filter(|l| *l > 0.0).map(|l| w.tokens / minutes / l),
        "budget_projection_ratio" => x
            .budgets
            .iter()
            .filter(|(_, amount, ..)| *amount > 0.0)
            .map(|(spent, amount, elapsed, remaining)| {
                let trailing = (*elapsed).clamp(1, WEEK_US);
                #[allow(clippy::cast_precision_loss)]
                let rate = spent / trailing as f64;
                #[allow(clippy::cast_precision_loss)]
                let projected = spent + rate * (*remaining).max(0) as f64;
                projected / amount
            })
            .reduce(f64::max),
        "latency_baseline_ratio" => {
            let now = mean(&w.total_ms)?;
            let base = mean(&x.baseline.total_ms)?;
            (base > 0.0).then(|| now / base)
        }
        "cost_baseline_ratio" => {
            let now = w.mean_cost()?;
            let base = x.baseline.mean_cost()?;
            (base > 0.0).then(|| now / base)
        }
        _ => None,
    }
}

/// The baseline window of an evaluation at `now`: the same hour of the week, one week earlier.
#[must_use]
pub fn baseline_hour(now_us: i64) -> (i64, i64) {
    let start = now_us - now_us.rem_euclid(HOUR_US) - WEEK_US;
    (start, start + HOUR_US)
}

/// Per-minute rate of a declared limit (`limit` per `window`), when it counts requests or tokens.
#[must_use]
pub fn per_minute(limit: &Value) -> Option<(String, f64)> {
    let unit = limit.get("unit")?.as_str()?;
    let amount = limit.get("limit")?.as_f64()?;
    let window = limit
        .get("per")
        .and_then(Value::as_str)
        .and_then(antirion_config::scalar::parse_duration)?;
    let minutes = window.as_secs_f64() / 60.0;
    let kind = match unit {
        "requests" => "rpm",
        "tokens" => "tpm",
        _ => return None,
    };
    (minutes > 0.0).then(|| (kind.to_owned(), amount / minutes))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn impact_matches_what_records_carry() {
        let scope: BTreeMap<String, String> = [("key", "k"), ("anomaly", "a1")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into();
        assert_eq!(
            record_scope(&scope),
            Some([("key".to_owned(), "k".to_owned())].into())
        );
        let about: BTreeMap<String, String> = [("store".to_owned(), "vault".to_owned())].into();
        assert_eq!(record_scope(&about), None);
        assert_eq!(record_scope(&BTreeMap::new()), Some(BTreeMap::new()));
    }

    fn rec(ms: f64, status: u64, cost: &str) -> Value {
        json!({"started_at": "2026-10-01T00:00:00Z", "status_code": status, "provider": "p",
               "timing": {"total_ms": ms, "ttft_ms": null}, "cost": {"total": cost},
               "attempts": [{"reason": "primary", "status_code": status}],
               "cache": {"status": "miss"}, "guardrails": [], "usage": {"total": 10}})
    }

    const LIMITS: Limits = Limits {
        rpm: Some(1.0),
        tpm: Some(10.0),
    };

    /// `metric` over `window` and `baseline` as an evaluation computes it: one tally per set.
    fn value(
        metric: &str,
        window: &[Value],
        baseline: &[Value],
        budgets: &[(f64, f64, i64, i64)],
    ) -> Option<f64> {
        compute(
            metric,
            &Inputs {
                window: &Tally::of(metric, window),
                window_us: 5 * MINUTE_US,
                baseline: &Tally::of(metric, baseline),
                limits: LIMITS,
                budgets,
            },
        )
    }

    #[test]
    fn latency_errors_and_spend() {
        let rs: Vec<Value> = (1..=20)
            .map(|i| rec(f64::from(i) * 100.0, if i == 20 { 502 } else { 200 }, "0.01"))
            .collect();
        let v = |m: &str| value(m, &rs, &[], &[]);
        assert_eq!(v("p95_latency_ms"), Some(1900.0));
        assert_eq!(v("p99_latency_ms"), Some(2000.0));
        assert_eq!(v("error_rate"), Some(0.05));
        assert_eq!(v("upstream_5xx_rate"), Some(0.05));
        assert_eq!(v("requests"), Some(20.0));
        assert!((v("spend_usd").unwrap() - 0.2).abs() < 1e-9);
        assert_eq!(v("rpm_utilization"), Some(4.0));
        assert_eq!(v("tpm_utilization"), Some(4.0));
        assert_eq!(v("cache_hit_rate"), Some(0.0));
        assert_eq!(v("ttft_p95_ms"), None, "no stream in the window");
        assert_eq!(v("latency_baseline_ratio"), None, "no baseline");
        assert_eq!(v("unknown"), None);
        let i = impact(rs.iter());
        assert_eq!((i.requests, i.errors, i.throttled), (20, 1, 0));
        assert_eq!(Tally::of("requests", &rs).impact, i);
    }

    #[test]
    fn fallbacks_guardrails_caches_and_streams() {
        let mut rs: Vec<Value> = (0..4).map(|_| rec(10.0, 200, "0")).collect();
        rs[0]["attempts"] =
            json!([{"reason": "primary", "status_code": 503}, {"reason": "fallback", "status_code": 200}]);
        rs[1]["guardrails"] = json!([{"verdict": "flag"}, {"verdict": "pass"}, {"verdict": "block"}, {}]);
        rs[2]["cache"] = json!({"status": "hit"});
        rs[3]["cache"] = json!({"status": "bypass"});
        rs[3]["timing"]["ttft_ms"] = json!(40.0);
        rs[3]["status_code"] = json!(429);
        let v = |m: &str| value(m, &rs, &[], &[]);
        assert_eq!(v("fallback_rate"), Some(0.25));
        assert_eq!(v("upstream_5xx_rate"), Some(0.2));
        assert_eq!(v("guardrail_events"), Some(2.0));
        assert_eq!(
            v("cache_hit_rate"),
            Some(1.0 / 3.0),
            "two misses, a hit and a bypass"
        );
        assert_eq!(v("ttft_p95_ms"), Some(40.0));
        assert_eq!(Tally::of("requests", &rs).impact.throttled, 1);
    }

    #[test]
    fn sums_over_nothing_match_the_iterator_sum() {
        let none: [Value; 0] = [];
        let spend = value("spend_usd", &none, &[], &[]).unwrap();
        let summed: f64 = none.iter().map(cost).sum();
        assert_eq!(spend.to_bits(), summed.to_bits());
        assert_eq!(value("cost_per_request_usd", &none, &[], &[]), None);
    }

    #[test]
    fn projections_and_baselines() {
        let budgets = [(50.0, 100.0, 10 * 86_400_000_000, 20 * 86_400_000_000)];
        let r = value("budget_projection_ratio", &[], &[], &budgets).unwrap();
        assert!((r - (50.0 + 50.0 / 7.0 * 20.0) / 100.0).abs() < 1e-9, "{r}");
        let now = [rec(200.0, 200, "0.02")];
        let base = [rec(100.0, 200, "0.01")];
        assert_eq!(value("latency_baseline_ratio", &now, &base, &[]), Some(2.0));
        assert_eq!(value("cost_baseline_ratio", &now, &base, &[]), Some(2.0));
        let (s, e) = baseline_hour(WEEK_US + 90 * MINUTE_US);
        assert_eq!((s, e), (HOUR_US, 2 * HOUR_US));
    }

    #[test]
    fn scopes_and_limits() {
        let r = rec(1.0, 200, "0");
        let mut scope = BTreeMap::new();
        assert!(in_scope(&r, &scope));
        scope.insert("provider".into(), "p".into());
        assert!(in_scope(&r, &scope));
        scope.insert("model".into(), "m".into());
        assert!(!in_scope(&r, &scope));
        assert_eq!(
            per_minute(&json!({"unit": "requests", "limit": 120, "per": "2m"})),
            Some(("rpm".into(), 60.0))
        );
        assert_eq!(
            per_minute(&json!({"unit": "tokens", "limit": 1000, "per": "1m"})),
            Some(("tpm".into(), 1000.0))
        );
        assert_eq!(per_minute(&json!({"unit": "concurrency", "limit": 5})), None);
    }
}
