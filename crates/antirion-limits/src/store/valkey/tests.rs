//! Run against a real server when `ANTIRION_TEST_VALKEY_URL` is set, for example
//! `docker run -d -p 16379:6379 valkey/valkey:8` and `redis://127.0.0.1:16379`; skipped otherwise.
//! The shared scenarios also run on a Redis Cluster when `ANTIRION_TEST_VALKEY_CLUSTER_URL` names one
//! of its nodes.

use std::sync::Arc;
use std::time::Duration;

use antirion_core::{ResourceUid, UtcMicros};
use antirion_testkit::FakeClock;
use proptest::prelude::*;
use tokio::task::JoinSet;

use super::ValkeyCounters;
use crate::counter::HoldMode;
use crate::math::{Fit, Gcra, GcraState, Window, WindowState};
use crate::store::memory::MemoryCounters;
use crate::store::scenarios::{self, DAY_US, admit, admitted, budget, conc, gcra, id, uid, window};
use crate::store::{AdmitDecision, CounterStore, ResvState, SettleOutcome};

fn url() -> Option<String> {
    std::env::var("ANTIRION_TEST_VALKEY_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

fn store(url: &str) -> ValkeyCounters {
    let prefix = format!("t{}:", ResourceUid::random().to_hex());
    ValkeyCounters::new(url, &prefix, 60_000_000, antirion_redis::Topology::Standalone).unwrap()
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn shared_scenarios() {
    let Some(url) = url() else { return };
    rt().block_on(async {
        let s = store(&url);
        scenarios::all_or_nothing(&s).await;
        scenarios::idempotency(&s).await;
        scenarios::refunds_readmit(&s).await;
        scenarios::budgets(&s).await;
        scenarios::rolling_budgets(&s, wall_us()).await;
        scenarios::headroom(&s).await;
        scenarios::concurrency_and_attempts(&s).await;
        scenarios::concurrent_no_overshoot(Arc::new(s), 60).await;
    });
}

/// The wall clock in microseconds: Valkey counts with its own `TIME`.
#[allow(clippy::disallowed_methods)]
fn wall_us() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
}

#[test]
fn shared_scenarios_on_a_cluster() {
    let Some(url) = std::env::var("ANTIRION_TEST_VALKEY_CLUSTER_URL")
        .ok()
        .filter(|u| !u.is_empty())
    else {
        return;
    };
    rt().block_on(async {
        let prefix = format!("t{}:", ResourceUid::random().to_hex());
        let s = ValkeyCounters::new(&url, &prefix, 60_000_000, antirion_redis::Topology::Cluster).unwrap();
        scenarios::all_or_nothing(&s).await;
        scenarios::idempotency(&s).await;
        scenarios::refunds_readmit(&s).await;
        scenarios::budgets(&s).await;
        scenarios::rolling_budgets(&s, wall_us()).await;
        scenarios::headroom(&s).await;
        scenarios::concurrency_and_attempts(&s).await;
        scenarios::concurrent_no_overshoot(Arc::new(s.clone()), 60).await;
        s.heartbeat("n1", 300_000).await.unwrap();
        assert_eq!(s.announce("n1", 60_000).await.unwrap(), 1);
        s.share_revocations(&[("abc".into(), 4_000_000_000)])
            .await
            .unwrap();
        assert_eq!(
            s.revocations().await.unwrap(),
            [("abc".to_owned(), 4_000_000_000)]
        );
        s.bind("d", "v", 60_000).await.unwrap();
        assert_eq!(s.binding("d").await.unwrap().as_deref(), Some("v"));
    });
}

#[test]
fn library_reloads_after_a_flush() {
    let Some(url) = url() else { return };
    rt().block_on(async {
        let s = store(&url);
        let mut conn = s.conn.clone();
        redis::cmd("FUNCTION")
            .arg("FLUSH")
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        assert!(admitted(&admit(&s, uid(), &[gcra(id("x"), 1, DAY_US, 1)]).await));
        s.ping().await.unwrap();
    });
}

#[test]
fn expired_leases_release_concurrency_and_charge_holds() {
    let Some(url) = url() else { return };
    rt().block_on(async {
        let s = store(&url);
        s.heartbeat("n1", 300_000).await.unwrap();
        let c = conc(id("conc"), 1);
        let b = budget(id("b"), 10, 4);
        let mut soft = budget(id("soft"), 10, 3);
        soft.mode = HoldMode::Track;
        let u = uid();
        assert!(admitted(
            &admit(&s, u, &[c.clone(), b.clone(), soft.clone()]).await
        ));
        assert!(!admitted(&admit(&s, uid(), std::slice::from_ref(&c)).await));
        assert!(s.reap(10).await.unwrap().is_empty());
        tokio::time::sleep(Duration::from_millis(500)).await;
        let expired = s.reap(10).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].uid, u);
        assert_eq!(expired[0].holds, vec![c.clone(), b.clone(), soft.clone()]);
        assert!(s.reap(10).await.unwrap().is_empty());
        assert!(admitted(&admit(&s, uid(), std::slice::from_ref(&c)).await));
        let views = s.read(&[b.clone(), soft.clone()]).await.unwrap();
        assert_eq!((views[0].spent, views[0].reserved), (4, 0));
        assert_eq!(views[1].spent, 3);
        let outcome = scenarios::settle(&s, u, &[c, b, soft], &[0, 0, 0]).await.unwrap();
        assert_eq!(outcome, SettleOutcome::Noop(Some(ResvState::Expired)));
    });
}

/// The same random sequence against both stores gives the same decisions and the same views.
/// Windows of a day keep the refill during the run below one unit, so the server's clock and the
/// frozen test clock decide alike.
#[test]
fn memory_and_valkey_agree() {
    let Some(url) = url() else { return };
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(12));
    let ops = proptest::collection::vec((0usize..4, 1u128..40, 0u128..60, any::<bool>()), 1..40);
    let rt = rt();
    runner
        .run(&ops, |ops| {
            rt.block_on(async {
                let v = store(&url);
                let m = MemoryCounters::new(Arc::new(FakeClock::at(UtcMicros(1_790_000_000_000_000))));
                let kinds = [
                    gcra(id("g"), 100, DAY_US, 1),
                    window(id("w"), 100, DAY_US, 24, 1),
                    conc(id("c"), 3),
                    budget(id("b"), 300, 1),
                ];
                let mut open = Vec::new();
                for (k, rho, actual, settle_now) in ops {
                    let mut h = kinds[k].clone();
                    if k != 2 {
                        h.amount = rho;
                    }
                    let u = uid();
                    let dm = admit(&m, u, std::slice::from_ref(&h)).await;
                    let dv = admit(&v, u, std::slice::from_ref(&h)).await;
                    match (&dm, &dv) {
                        (AdmitDecision::Admitted { views: a }, AdmitDecision::Admitted { views: b }) => {
                            prop_assert_eq!(a[0].remaining, b[0].remaining);
                            prop_assert_eq!((a[0].spent, a[0].reserved), (b[0].spent, b[0].reserved));
                            open.push((u, h, actual));
                        }
                        (AdmitDecision::Refused { refusals: a }, AdmitDecision::Refused { refusals: b }) => {
                            prop_assert_eq!(a[0].kind, b[0].kind);
                        }
                        _ => prop_assert!(false, "{dm:?} vs {dv:?}"),
                    }
                    if settle_now && let Some((u, h, a)) = open.pop() {
                        let sm = scenarios::settle(&m, u, std::slice::from_ref(&h), &[a])
                            .await
                            .unwrap();
                        let sv = scenarios::settle(&v, u, std::slice::from_ref(&h), &[a])
                            .await
                            .unwrap();
                        prop_assert_eq!(sm, sv);
                    }
                }
                let all = kinds.to_vec();
                let a = m.read(&all).await.unwrap();
                let b = v.read(&all).await.unwrap();
                for (x, y) in a.iter().zip(&b) {
                    prop_assert_eq!(x.remaining, y.remaining);
                    prop_assert_eq!((x.spent, x.reserved), (y.spent, y.reserved));
                }
                Ok(())
            })
        })
        .unwrap();
}

fn st_args(s: Option<GcraState>) -> [String; 4] {
    match s {
        Some(s) => [
            s.avail.to_string(),
            s.at_us.to_string(),
            s.burst.to_string(),
            s.per_us.to_string(),
        ],
        None => [String::new(), "0".into(), "0".into(), "0".into()],
    }
}

/// The library's exact arithmetic equals the Rust model on large values, where doubles would not.
#[test]
fn lua_arithmetic_matches_the_rust_model() {
    let Some(url) = url() else { return };
    let rt = rt();
    let s = rt.block_on(async { store(&url) });
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(200));
    let arith = (-(1i128 << 63)..(1i128 << 63), 1i128..(1i128 << 62));
    runner
        .run(&arith, |(a, b)| {
            let out = rt
                .block_on(s.eval(&["arith".into(), a.to_string(), b.to_string()]))
                .unwrap();
            let expect: Vec<String> = vec![
                (a + b).to_string(),
                (a - b).to_string(),
                (a * b).to_string(),
                (a.max(0) / b).to_string(),
                (a.max(0) % b).to_string(),
                (a.cmp(&b) as i8).to_string(),
            ];
            prop_assert_eq!(out, expect);
            Ok(())
        })
        .unwrap();
    let gcra_case = (
        1u64..(1 << 40),
        1_000_000u64..(1 << 45),
        1u64..(1 << 40),
        proptest::option::of((
            -(1i128 << 80)..(1i128 << 84),
            0i64..(1 << 50),
            1u64..(1 << 40),
            prop_oneof![Just(0u64), 1_000_000u64..(1 << 45)],
        )),
        0i64..(1 << 52),
        0u128..(1 << 41),
        0u128..(1 << 41),
    );
    runner
        .run(&gcra_case, |(limit, per_us, burst, st, now, rho, actual)| {
            let g = Gcra { limit, per_us, burst };
            let st = st.map(|(avail, at_us, b, p)| GcraState {
                avail,
                at_us,
                burst: b,
                per_us: p,
            });
            let [a, t, b, p] = st_args(st);
            let base = |op: &str| -> Vec<String> {
                vec![
                    op.into(),
                    limit.to_string(),
                    per_us.to_string(),
                    burst.to_string(),
                    a.clone(),
                    t.clone(),
                    b.clone(),
                    p.clone(),
                    now.to_string(),
                ]
            };
            let mut args = base("gcra_admit");
            args.push(rho.to_string());
            let out = rt.block_on(s.eval(&args)).unwrap();
            match g.admit(st, now, rho) {
                Fit::Admit(n) => prop_assert_eq!(
                    out,
                    vec![
                        "admit".to_string(),
                        n.avail.to_string(),
                        n.at_us.to_string(),
                        n.burst.to_string(),
                        n.per_us.to_string()
                    ]
                ),
                Fit::Wait(w) => prop_assert_eq!(
                    out,
                    vec!["wait".to_string(), w.map(|w| w.to_string()).unwrap_or_default()]
                ),
                Fit::Exceeds => prop_assert_eq!(out, vec!["exceeds".to_string()]),
            }
            let mut args = base("gcra_settle");
            args.push(rho.to_string());
            args.push(actual.to_string());
            let n = g.settle(st, now, rho, actual);
            prop_assert_eq!(
                rt.block_on(s.eval(&args)).unwrap(),
                vec![
                    n.avail.to_string(),
                    n.at_us.to_string(),
                    n.burst.to_string(),
                    n.per_us.to_string()
                ]
            );
            let (remaining, full) = g.view(st, now);
            prop_assert_eq!(
                rt.block_on(s.eval(&base("gcra_view"))).unwrap(),
                vec![remaining.to_string(), full.to_string()]
            );
            Ok(())
        })
        .unwrap();
    let window_case = (
        1u64..1000,
        1_000_000u64..100_000_000,
        1u32..100,
        proptest::collection::btree_map(0i64..2_000_000, 1u128..50, 0..10),
        1_000_000_000i64..1_000_000_000_000,
        1u128..60,
    );
    runner
        .run(&window_case, |(limit, per_us, precision, slots, now, rho)| {
            let w = Window {
                limit,
                per_us,
                precision,
            };
            let base = w.slot(now) - 1_000_000;
            let state: WindowState = slots.into_iter().map(|(k, v)| (base + k / 10_000, v)).collect();
            let text = state
                .iter()
                .map(|(k, v)| format!("{k}:{v}"))
                .collect::<Vec<_>>()
                .join(";");
            let args = vec![
                "window_admit".to_string(),
                limit.to_string(),
                per_us.to_string(),
                precision.to_string(),
                text,
                now.to_string(),
                rho.to_string(),
            ];
            let out = rt.block_on(s.eval(&args)).unwrap();
            match w.admit(state, now, rho) {
                Fit::Admit((n, k)) => {
                    let t = n
                        .iter()
                        .map(|(k, v)| format!("{k}:{v}"))
                        .collect::<Vec<_>>()
                        .join(";");
                    prop_assert_eq!(out, vec!["admit".to_string(), t, k.to_string()]);
                }
                Fit::Wait(wt) => prop_assert_eq!(out, vec!["wait".to_string(), wt.unwrap_or(0).to_string()]),
                Fit::Exceeds => prop_assert_eq!(out, vec!["exceeds".to_string()]),
            }
            Ok(())
        })
        .unwrap();
}

/// A rate hold settled at its reserved amount leaves the counter exactly where admission put it,
/// as the memory store does, and one settled below it gives the difference back.
#[test]
fn settling_a_rate_hold_at_its_reservation_changes_nothing() {
    let Some(url) = url() else { return };
    rt().block_on(async {
        let v = store(&url);
        let m = MemoryCounters::new(Arc::new(FakeClock::at(UtcMicros(1_790_000_000_000_000))));
        // Ten units a day: refill within the test is far below one unit.
        let h = gcra(id("g"), 10, DAY_US, 3);
        for (actual, remaining) in [(3, 7), (3, 4), (1, 3)] {
            let u = uid();
            assert!(admitted(&admit(&m, u, std::slice::from_ref(&h)).await));
            assert!(admitted(&admit(&v, u, std::slice::from_ref(&h)).await));
            let sm = scenarios::settle(&m, u, std::slice::from_ref(&h), &[actual])
                .await
                .unwrap();
            let sv = scenarios::settle(&v, u, std::slice::from_ref(&h), &[actual])
                .await
                .unwrap();
            assert_eq!(sm, sv);
            let views = (
                m.read(std::slice::from_ref(&h)).await.unwrap(),
                v.read(std::slice::from_ref(&h)).await.unwrap(),
            );
            assert_eq!(views.0[0].remaining, remaining);
            assert_eq!(views.1[0].remaining, remaining);
        }
    });
}

/// Long division by limbs is exact for every limb count of dividend and divisor, at the limb
/// boundaries and where a quotient limb's estimate in doubles is off by one; numbers read and
/// written at the 2^48 and 2^53 boundaries keep every digit.
#[test]
fn lua_division_and_number_text_are_exact() {
    let Some(url) = url() else { return };
    let rt = rt();
    let s = rt.block_on(async { store(&url) });
    let check = |a: u128, b: u128| -> Result<(), TestCaseError> {
        let out = rt
            .block_on(s.eval(&["divmod".into(), a.to_string(), b.to_string()]))
            .unwrap();
        let (q, r) = (a / b, a % b);
        let expect = vec![q.to_string(), r.to_string(), (q + u128::from(r > 0)).to_string()];
        prop_assert_eq!(out, expect, "{} / {}", a, b);
        Ok(())
    };
    let base = 1u128 << 24;
    let mut edges = vec![0u128, 1, 2, 3, 999_999, 1_000_000, 1_000_001];
    for k in 1..5u32 {
        let p = base.pow(k);
        edges.extend([p - 1, p, p + 1, 2 * p - 1, (base - 1) * p]);
    }
    for k in [48u32, 53, 63, 64, 96, 120, 126] {
        let p = 1u128 << k;
        edges.extend([p - 1, p, p + 1]);
    }
    edges.extend([
        9_007_199_254_740_991,
        9_007_199_254_740_992,
        9_007_199_254_740_993,
    ]);
    edges.extend([999_999_999_999_999, 1_000_000_000_000_000, 9_999_999_999_999_999]);
    for &a in &edges {
        for &b in edges.iter().filter(|b| **b > 0) {
            check(a, b).unwrap();
        }
    }
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig::with_cases(400));
    let limbs = |max_bits: u32| {
        (1u32..=max_bits).prop_flat_map(|bits| (1u128 << (bits - 1))..=(u128::MAX >> (128 - bits)))
    };
    runner
        .run(&(limbs(127), limbs(100)), |(a, b)| check(a, b))
        .unwrap();
    // A divisor whose top limb is small and whose lower limbs are large makes the estimate of a
    // quotient limb, from the leading limbs in doubles, land one away from the true limb.
    runner
        .run(
            &(limbs(127), 1u128..base, 1u32..4, 0u128..base),
            |(a, top, shift, low)| {
                let b =
                    (top << (24 * shift)) | (low.wrapping_mul(0x00ff_fff1) & ((1u128 << (24 * shift)) - 1));
                check(a, b.max(1))?;
                check(a.wrapping_mul(b.max(1)) >> 1, b.max(1))
            },
        )
        .unwrap();
    // Text in and out: the arithmetic answers with every digit around the fast paths' bounds.
    let signed = edges
        .iter()
        .filter(|v| **v < (1u128 << 100))
        .flat_map(|v| [i128::try_from(*v).unwrap(), -i128::try_from(*v).unwrap()]);
    for a in signed {
        let out = rt
            .block_on(s.eval(&["arith".into(), a.to_string(), "1".into()]))
            .unwrap();
        let expect: Vec<String> = vec![
            (a + 1).to_string(),
            (a - 1).to_string(),
            a.to_string(),
            a.max(0).to_string(),
            "0".into(),
            (a.cmp(&1) as i8).to_string(),
        ];
        assert_eq!(out, expect, "{a}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    #[test]
    fn concurrent_reservations_admit_exactly_floor_of_quota(
        quota in 1u64..2000, rho in 1u128..400, tasks in 1usize..60, use_budget in any::<bool>(),
    ) {
        let Some(url) = url() else { return Ok(()) };
        let admitted_total = rt().block_on(async move {
            let s = Arc::new(store(&url));
            let h = if use_budget { budget(id("p"), u128::from(quota), rho) } else { gcra(id("p"), quota, DAY_US, rho) };
            let mut set = JoinSet::new();
            for _ in 0..tasks {
                let s = Arc::clone(&s);
                let h = h.clone();
                set.spawn(async move { admitted(&admit(&*s, uid(), &[h]).await) });
            }
            let mut n = 0u128;
            while let Some(ok) = set.join_next().await {
                if ok.unwrap() {
                    n += 1;
                }
            }
            n
        });
        prop_assert_eq!(admitted_total, (u128::from(quota) / rho).min(tasks as u128));
    }
}

#[test]
fn membership_and_shared_revocations() {
    let Some(url) = url() else { return };
    rt().block_on(async {
        let s = store(&url);
        assert_eq!(s.announce("a", 10_000).await.unwrap(), 1);
        assert_eq!(s.announce("b", 10_000).await.unwrap(), 2);
        let far = 4_000_000_000i64;
        let digest = "ab".repeat(32);
        assert_eq!(s.share_revocations(&[(digest.clone(), far)]).await.unwrap(), 1);
        assert_eq!(s.share_revocations(&[(digest.clone(), far)]).await.unwrap(), 0);
        assert_eq!(s.revocations().await.unwrap(), vec![(digest, far)]);
    });
}

/// A node reads only the revocations shared after the last one it saw; expired ones are dropped,
/// lookups by digest are exact (the previous version's keys included), and a lost log is read
/// again from the start.
#[test]
fn revocations_are_read_incrementally_from_the_log() {
    let Some(url) = url() else { return };
    rt().block_on(async {
        let s = store(&url);
        let far = 4_000_000_000i64;
        let d = |i: usize| format!("{i:064x}");
        let first: Vec<(String, i64)> = (0..2500).map(|i| (d(i), far)).collect();
        assert_eq!(s.share_revocations(&first).await.unwrap(), 2500);
        let (cursor, all) = s.revocations_since(0).await.unwrap();
        assert_eq!((cursor, all.len()), (2500, 2500), "read page by page");
        assert_eq!(s.revocations_since(cursor).await.unwrap(), (cursor, vec![]));

        let expired = (d(9999), 1_000);
        let fresh = (d(10_000), far + 1);
        assert_eq!(
            s.share_revocations(&[expired.clone(), fresh.clone(), first[0].clone()])
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            s.revocations_since(cursor).await.unwrap(),
            (cursor + 1, vec![fresh.clone()])
        );
        assert_eq!(s.revocation(&fresh.0).await.unwrap(), Some(far + 1));
        assert_eq!(s.revocation(&expired.0).await.unwrap(), None);

        // A node of the previous version writes only its own key.
        let mut conn = s.conn.clone();
        let old = d(20_000);
        redis::cmd("SET")
            .arg(format!("{}mrev:{old}", s.prefix))
            .arg(far.to_string())
            .query_async::<()>(&mut conn)
            .await
            .unwrap();
        assert_eq!(s.revocation(&old).await.unwrap(), Some(far));
        let legacy: Option<String> = redis::cmd("GET")
            .arg(format!("{}mrev:{}", s.prefix, fresh.0))
            .query_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(
            legacy,
            Some((far + 1).to_string()),
            "nodes of the previous version find it"
        );

        // The store lost the log: a cursor past its sequence reads everything again.
        for k in ["mrevs:n", "mrevs:log", "mrevs:exp"] {
            redis::cmd("DEL")
                .arg(format!("{}{k}", s.prefix))
                .query_async::<()>(&mut conn)
                .await
                .unwrap();
        }
        assert_eq!(
            s.share_revocations(std::slice::from_ref(&fresh)).await.unwrap(),
            1
        );
        assert_eq!(s.revocations_since(cursor + 1).await.unwrap(), (1, vec![fresh]));
    });
}

#[test]
fn health_records_keep_the_newest_and_version_changes() {
    let Some(url) = url() else { return };
    rt().block_on(async {
        let s = store(&url);
        assert_eq!(s.health_version().await.unwrap(), 0);
        s.health_put(&[("b\u{1f}a".into(), 10, "10|open|99|1".into())])
            .await
            .unwrap();
        let v1 = s.health_version().await.unwrap();
        s.health_put(&[("b\u{1f}a".into(), 5, "5|closed|0".into())])
            .await
            .unwrap();
        assert_eq!(
            s.health_version().await.unwrap(),
            v1,
            "an older record changes nothing"
        );
        s.health_put(&[
            ("b\u{1f}a".into(), 11, "11|closed|1".into()),
            ("m\u{1f}a".into(), 11, "11|off".into()),
        ])
        .await
        .unwrap();
        assert!(s.health_version().await.unwrap() > v1);
        let mut all = s.health_records().await.unwrap();
        all.sort();
        assert_eq!(
            all,
            vec![
                ("b\u{1f}a".to_owned(), "11|closed|1".to_owned()),
                ("m\u{1f}a".to_owned(), "11|off".to_owned())
            ]
        );
    });
}

#[test]
fn probe_leases_are_counted_and_given_back() {
    let Some(url) = url() else { return };
    rt().block_on(async {
        let s = store(&url);
        let dep = format!("dep-{}", uid().hex());
        assert!(s.probe_lease(&dep, 1, 60_000, 1).await.unwrap());
        assert!(
            !s.probe_lease(&dep, 1, 60_000, 2).await.unwrap(),
            "one lease is held"
        );
        assert!(
            s.probe_lease(&dep, 2, 60_000, 3).await.unwrap(),
            "a larger limit has room"
        );
        s.probe_release(&dep, 1).await.unwrap();
        s.probe_release(&dep, 3).await.unwrap();
        assert!(s.probe_lease(&dep, 1, 50, 4).await.unwrap());
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        assert!(
            s.probe_lease(&dep, 1, 60_000, 5).await.unwrap(),
            "an expired lease is no longer held"
        );
    });
}
