# Benchmark report

**Not publishable:** self-test selftest-a: mock-latency failed: L0 at 50000 RPS: p99 1232 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A2/selftest-a2: mock-latency failed: L0 at 50000 RPS: p99 1258 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A2/selftest-a2.log: no result (it did not finish); self-test shards/A3/selftest-a3: mock-latency failed: L0 at 50000 RPS: p99 1098 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A3/selftest-a3.log: no result (it did not finish); self-test shards/A4/selftest-a4: mock-latency failed: L0 at 50000 RPS: p99 1147 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A4/selftest-a4.log: no result (it did not finish); self-test shards/A5/selftest-a5: mock-latency failed: L0 at 50000 RPS: p99 1280 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A5/selftest-a5.log: no result (it did not finish); self-test shards/A6/selftest-a6: mock-latency failed: L0 at 50000 RPS: p99 1102 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A6/selftest-a6.log: no result (it did not finish); self-test shards/A7/selftest-a7: mock-latency failed: L0 at 50000 RPS: p99 949 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A7/selftest-a7.log: no result (it did not finish).

| Run |  |
|---|---|
| Suite | cell |
| Topology | ref-4 |
| Target | litellm 1.103.1 |
| Target commit | n/a |
| Harness commit | d461c20d900198add16216d329f9fdf582a523c8-dirty |
| Corpus seed | 20260930 |
| Started | 2026-10-18T01:11:24Z |
| Ended | 2026-10-18T01:37:28Z |
| Manifest | [run.json](run.json) |

## Cells

Each number is the median over the repetitions with [min–max]. An overhead smaller than the spread of the direct run at the same percentile is shown as within noise.

### H1 — open loop, S1, P-min, 5000 RPS

**Invalid** (5 repetitions): rep-1: direct p99 1.101ms above the scripted delay + 1 ms (1ms); rep-2: direct p99 1.08ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.072ms above the scripted delay + 1 ms (1ms); rep-4: direct p99 1.091ms above the scripted delay + 1 ms (1ms); rep-5: direct p99 1.08ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/H1/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Direct latency | [561 [551–596]](cells/H1/summary.json) | [851 [836–871]](cells/H1/summary.json) | [1080 [1072–1101]](cells/H1/summary.json) | [1244 [1242–1262]](cells/H1/summary.json) | [1605 [1483–1673]](cells/H1/summary.json) | [573 [563–604]](cells/H1/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [0 [0–0]](cells/H1/summary.json) |
| error_rate | [1 [1–1]](cells/H1/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/H1/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/H1/summary.json) |
| errors.timeouts | [150000 [150000–150000]](cells/H1/summary.json) |
| errors.total | [150000 [150000–150000]](cells/H1/summary.json) |
| generator_cpu_pct | [7.98 [7.85–7.99]](cells/H1/summary.json) |
| lag_us.max | [2231 [1372–4207]](cells/H1/summary.json) |
| lag_us.mean | [82.09 [79.93–83.34]](cells/H1/summary.json) |
| lag_us.p50 | [44 [37–50]](cells/H1/summary.json) |
| lag_us.p90 | [215 [202–224]](cells/H1/summary.json) |
| lag_us.p99 | [307 [294–319]](cells/H1/summary.json) |
| lag_us.p99_9 | [395 [357–411]](cells/H1/summary.json) |
| late_fraction | [0.00 [0.00–0.00]](cells/H1/summary.json) |
| mock_cpu_pct | [1.06 [1.06–1.08]](cells/H1/summary.json) |
| rss_bytes.idle | [1733566464 [1733017600–1755996160]](cells/H1/summary.json) |
| rss_bytes.peak | [1940783104 [1939750912–1959690240]](cells/H1/summary.json) |
| rss_bytes.steady | [1924663959 [1923771781–1943350067]](cells/H1/summary.json) |
| rss_over_idle_bytes | [206733312 [203694080–207323136]](cells/H1/summary.json) |
| success_fraction | [0 [0–0]](cells/H1/summary.json) |
| upstream_requests | [9923 [9847–9963]](cells/H1/summary.json) |

### cmp-C1 — open loop, S1, P-min, 10000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.112ms above the scripted delay + 1 ms (1ms); rep-2: direct p99 1.105ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.124ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C1/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Direct latency | [587 [573–598]](cells/cmp-C1/summary.json) | [875 [872–889]](cells/cmp-C1/summary.json) | [1112 [1105–1124]](cells/cmp-C1/summary.json) | [1284 [1274–1307]](cells/cmp-C1/summary.json) | [1875 [1743–1888]](cells/cmp-C1/summary.json) | [598 [587–610]](cells/cmp-C1/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [0 [0–0]](cells/cmp-C1/summary.json) |
| error_rate | [1 [1–1]](cells/cmp-C1/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.timeouts | [150000 [150000–150000]](cells/cmp-C1/summary.json) |
| errors.total | [150000 [150000–150000]](cells/cmp-C1/summary.json) |
| generator_cpu_pct | [8.64 [8.61–8.68]](cells/cmp-C1/summary.json) |
| lag_us.max | [1535 [1299–9103]](cells/cmp-C1/summary.json) |
| lag_us.mean | [77.52 [75.76–83.63]](cells/cmp-C1/summary.json) |
| lag_us.p50 | [63 [59–66]](cells/cmp-C1/summary.json) |
| lag_us.p90 | [173 [171–175]](cells/cmp-C1/summary.json) |
| lag_us.p99 | [288 [286–288]](cells/cmp-C1/summary.json) |
| lag_us.p99_9 | [384 [375–423]](cells/cmp-C1/summary.json) |
| late_fraction | [0.00 [0.00–0.00]](cells/cmp-C1/summary.json) |
| mock_cpu_pct | [1.45 [1.44–1.45]](cells/cmp-C1/summary.json) |
| rss_bytes.idle | [1732927488 [1731792896–1760387072]](cells/cmp-C1/summary.json) |
| rss_bytes.peak | [1940160512 [1939046400–1960288256]](cells/cmp-C1/summary.json) |
| rss_bytes.steady | [1917705437 [1916644252–1937703813]](cells/cmp-C1/summary.json) |
| rss_over_idle_bytes | [207233024 [199901184–207253504]](cells/cmp-C1/summary.json) |
| success_fraction | [0 [0–0]](cells/cmp-C1/summary.json) |
| upstream_requests | [4994 [4970–5115]](cells/cmp-C1/summary.json) |

### cmp-C11 — saturation, S1, P-min, saturation ladder

Valid, 1 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C11/summary.json)

| Metric | median [min–max] |
|---|---|
| rss_bytes.idle | [1761263616 [1761263616–1761263616]](cells/cmp-C11/summary.json) |
| saturation_rps | [0 [0–0]](cells/cmp-C11/summary.json) |

### cmp-C13 — saturation, M1, P-min, saturation ladder

Valid, 1 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C13/summary.json)

| Metric | median [min–max] |
|---|---|
| rss_bytes.idle | [1776877568 [1776877568–1776877568]](cells/cmp-C13/summary.json) |
| saturation_rps | [0 [0–0]](cells/cmp-C13/summary.json) |

### cmp-C16 — idle, P-min, no load

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C16/summary.json)

| Metric | median [min–max] |
|---|---|
| rss_bytes.idle | [1742700544 [1742368768–1743179776]](cells/cmp-C16/summary.json) |
| rss_bytes.peak | [1742700544 [1742368768–1743179776]](cells/cmp-C16/summary.json) |
| rss_bytes.steady | [1742700544 [1742368768–1743179776]](cells/cmp-C16/summary.json) |

### cmp-C17 — open loop, S1, P-min, L1500, 5000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.502207s above the scripted delay + 1 ms (1.501s); rep-2: direct p99 1.502207s above the scripted delay + 1 ms (1.501s); rep-3: direct p99 1.502207s above the scripted delay + 1 ms (1.501s) Informational: no pass condition. [summary.json](cells/cmp-C17/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Direct latency | [1501183 [1501183–1501183]](cells/cmp-C17/summary.json) | [1501183 [1501183–1501183]](cells/cmp-C17/summary.json) | [1502207 [1502207–1502207]](cells/cmp-C17/summary.json) | [1502207 [1502207–1502207]](cells/cmp-C17/summary.json) | [1502207 [1502207–1503231]](cells/cmp-C17/summary.json) | [1500694 [1500691–1500702]](cells/cmp-C17/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [0 [0–0]](cells/cmp-C17/summary.json) |
| error_rate | [1 [1–1]](cells/cmp-C17/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.timeouts | [75000 [75000–75000]](cells/cmp-C17/summary.json) |
| errors.total | [75000 [75000–75000]](cells/cmp-C17/summary.json) |
| generator_cpu_pct | [8.06 [8.02–8.09]](cells/cmp-C17/summary.json) |
| lag_us.max | [2477 [2159–3067]](cells/cmp-C17/summary.json) |
| lag_us.mean | [102 [99.67–105]](cells/cmp-C17/summary.json) |
| lag_us.p50 | [81 [78–87]](cells/cmp-C17/summary.json) |
| lag_us.p90 | [219 [212–231]](cells/cmp-C17/summary.json) |
| lag_us.p99 | [303 [299–345]](cells/cmp-C17/summary.json) |
| lag_us.p99_9 | [409 [376–410]](cells/cmp-C17/summary.json) |
| late_fraction | [0.00 [0.00–0.00]](cells/cmp-C17/summary.json) |
| mock_cpu_pct | [4.17 [4.17–4.26]](cells/cmp-C17/summary.json) |
| rss_bytes.idle | [1736048640 [1735221248–1736265728]](cells/cmp-C17/summary.json) |
| rss_bytes.peak | [2046046208 [2043969536–2050375680]](cells/cmp-C17/summary.json) |
| rss_bytes.steady | [2013731682 [2012500283–2016173607]](cells/cmp-C17/summary.json) |
| rss_over_idle_bytes | [310824960 [307920896–314109952]](cells/cmp-C17/summary.json) |
| success_fraction | [0 [0–0]](cells/cmp-C17/summary.json) |
| upstream_requests | [5049 [5018–5097]](cells/cmp-C17/summary.json) |

### cmp-C2 — open loop, S1, P-min, 1000 RPS

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C2/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [9108771 [9067940–9395499]](cells/cmp-C2/summary.json) | [24247606 [23526569–25230560]](cells/cmp-C2/summary.json) | [29064251 [28851353–29080783]](cells/cmp-C2/summary.json) | [29834312 [29785203–29883384]](cells/cmp-C2/summary.json) | [29997959 [29965355–29998105]](cells/cmp-C2/summary.json) | [10270854 [10180102–10435211]](cells/cmp-C2/summary.json) |
| Gateway latency | [9109503 [9068543–9396223]](cells/cmp-C2/summary.json) | [24248319 [23527423–25231359]](cells/cmp-C2/summary.json) | [29065215 [28852223–29081599]](cells/cmp-C2/summary.json) | [29835263 [29786111–29884415]](cells/cmp-C2/summary.json) | [29999103 [29966335–29999103]](cells/cmp-C2/summary.json) | [10271450 [10180786–10435916]](cells/cmp-C2/summary.json) |
| Direct latency | [724 [603–732]](cells/cmp-C2/summary.json) | [799 [713–854]](cells/cmp-C2/summary.json) | [870 [816–964]](cells/cmp-C2/summary.json) | [951 [908–1031]](cells/cmp-C2/summary.json) | [998 [980–1144]](cells/cmp-C2/summary.json) | [684 [596–705]](cells/cmp-C2/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [719 [712–727]](cells/cmp-C2/summary.json) |
| cpu_us_per_request | [5399 [5394–5462]](cells/cmp-C2/summary.json) |
| error_rate | [0.28 [0.27–0.29]](cells/cmp-C2/summary.json) |
| errors.connect_errors | [1429 [1322–1500]](cells/cmp-C2/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.timeouts | [2819 [2667–2892]](cells/cmp-C2/summary.json) |
| errors.total | [4214 [4096–4319]](cells/cmp-C2/summary.json) |
| generator_cpu_pct | [6.84 [6.79–6.85]](cells/cmp-C2/summary.json) |
| lag_us.max | [448 [401–453]](cells/cmp-C2/summary.json) |
| lag_us.mean | [131 [117–141]](cells/cmp-C2/summary.json) |
| lag_us.p50 | [136 [134–137]](cells/cmp-C2/summary.json) |
| lag_us.p90 | [169 [169–177]](cells/cmp-C2/summary.json) |
| lag_us.p99 | [287 [239–304]](cells/cmp-C2/summary.json) |
| lag_us.p99_9 | [375 [302–376]](cells/cmp-C2/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C2/summary.json) |
| mock_cpu_pct | [0.39 [0.38–0.40]](cells/cmp-C2/summary.json) |
| rss_bytes.idle | [1735794688 [1735421952–1758650368]](cells/cmp-C2/summary.json) |
| rss_bytes.peak | [1945366528 [1945313280–1964265472]](cells/cmp-C2/summary.json) |
| rss_bytes.steady | [1919063472 [1918651688–1938864128]](cells/cmp-C2/summary.json) |
| rss_over_idle_bytes | [209571840 [205615104–209891328]](cells/cmp-C2/summary.json) |
| success_fraction | [0.72 [0.71–0.73]](cells/cmp-C2/summary.json) |
| upstream_requests | [5110 [5104–5123]](cells/cmp-C2/summary.json) |

### cmp-C21.401 — open loop, S8, P-min, 20000 RPS

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C21.401/summary.json)

| Metric | median [min–max] |
|---|---|
| achieved_rps | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| error_rate | [1 [1–1]](cells/cmp-C21.401/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.timeouts | [300000 [300000–300000]](cells/cmp-C21.401/summary.json) |
| errors.total | [300000 [300000–300000]](cells/cmp-C21.401/summary.json) |
| generator_cpu_pct | [7.78 [7.77–7.82]](cells/cmp-C21.401/summary.json) |
| lag_us.max | [843 [546–901]](cells/cmp-C21.401/summary.json) |
| lag_us.mean | [69.47 [67.58–71.61]](cells/cmp-C21.401/summary.json) |
| lag_us.p50 | [41 [37–47]](cells/cmp-C21.401/summary.json) |
| lag_us.p90 | [170 [169–172]](cells/cmp-C21.401/summary.json) |
| lag_us.p99 | [292 [291–292]](cells/cmp-C21.401/summary.json) |
| lag_us.p99_9 | [371 [369–374]](cells/cmp-C21.401/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| mock_cpu_pct | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| rss_bytes.idle | [1734717440 [1734586368–1761468416]](cells/cmp-C21.401/summary.json) |
| rss_bytes.peak | [2378702848 [2378444800–2396639232]](cells/cmp-C21.401/summary.json) |
| rss_bytes.steady | [2262665256 [2262109471–2281166968]](cells/cmp-C21.401/summary.json) |
| rss_over_idle_bytes | [643858432 [635170816–643985408]](cells/cmp-C21.401/summary.json) |
| success_fraction | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| upstream_requests | [0 [0–0]](cells/cmp-C21.401/summary.json) |

### cmp-C21.404 — open loop, S8-404, P-min, 20000 RPS

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C21.404/summary.json)

| Metric | median [min–max] |
|---|---|
| achieved_rps | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| error_rate | [1 [1–1]](cells/cmp-C21.404/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.timeouts | [300000 [300000–300000]](cells/cmp-C21.404/summary.json) |
| errors.total | [300000 [300000–300000]](cells/cmp-C21.404/summary.json) |
| generator_cpu_pct | [7.61 [7.60–7.63]](cells/cmp-C21.404/summary.json) |
| lag_us.max | [3137 [1704–3841]](cells/cmp-C21.404/summary.json) |
| lag_us.mean | [69.40 [67.00–70.00]](cells/cmp-C21.404/summary.json) |
| lag_us.p50 | [44 [37–47]](cells/cmp-C21.404/summary.json) |
| lag_us.p90 | [165 [164–165]](cells/cmp-C21.404/summary.json) |
| lag_us.p99 | [286 [283–286]](cells/cmp-C21.404/summary.json) |
| lag_us.p99_9 | [398 [381–399]](cells/cmp-C21.404/summary.json) |
| late_fraction | [0.00 [0.00–0.00]](cells/cmp-C21.404/summary.json) |
| mock_cpu_pct | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| rss_bytes.idle | [1737064448 [1736654848–1737658368]](cells/cmp-C21.404/summary.json) |
| rss_bytes.peak | [2552119296 [2529329152–2561646592]](cells/cmp-C21.404/summary.json) |
| rss_bytes.steady | [2296247091 [2289640950–2298279012]](cells/cmp-C21.404/summary.json) |
| rss_over_idle_bytes | [815464448 [791670784–824582144]](cells/cmp-C21.404/summary.json) |
| success_fraction | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| upstream_requests | [0 [0–0]](cells/cmp-C21.404/summary.json) |

### cmp-C32.c64 — closed loop, S1, P-min, closed loop, 64 connections

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C32.c64/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [132167 [107398–149573]](cells/cmp-C32.c64/summary.json) | [213575 [174787–277191]](cells/cmp-C32.c64/summary.json) | [488445 [362755–510973]](cells/cmp-C32.c64/summary.json) | [568561 [554782–643839]](cells/cmp-C32.c64/summary.json) | [586110 [561344–644833]](cells/cmp-C32.c64/summary.json) | [150264 [149305–151525]](cells/cmp-C32.c64/summary.json) |
| Gateway latency | [132351 [107583–149759]](cells/cmp-C32.c64/summary.json) | [213887 [175103–277503]](cells/cmp-C32.c64/summary.json) | [488959 [363263–511487]](cells/cmp-C32.c64/summary.json) | [569343 [555519–644607]](cells/cmp-C32.c64/summary.json) | [587775 [562687–646143]](cells/cmp-C32.c64/summary.json) | [150468 [149509–151731]](cells/cmp-C32.c64/summary.json) |
| Direct latency | [185 [184–186]](cells/cmp-C32.c64/summary.json) | [312 [312–316]](cells/cmp-C32.c64/summary.json) | [514 [508–514]](cells/cmp-C32.c64/summary.json) | [768 [737–782]](cells/cmp-C32.c64/summary.json) | [1343 [1310–1665]](cells/cmp-C32.c64/summary.json) | [205 [204–206]](cells/cmp-C32.c64/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [424 [422–427]](cells/cmp-C32.c64/summary.json) |
| cpu_us_per_request | [9018 [9015–9124]](cells/cmp-C32.c64/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| generator_cpu_pct | [39.82 [39.73–39.98]](cells/cmp-C32.c64/summary.json) |
| lag_us.max | [16 [10–17]](cells/cmp-C32.c64/summary.json) |
| lag_us.mean | [1.00 [1.00–1.01]](cells/cmp-C32.c64/summary.json) |
| lag_us.p50 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p90 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p99 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p99_9 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| mock_cpu_pct | [28.72 [28.46–28.88]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.idle | [1735708672 [1735622656–1736060928]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.peak | [1873117184 [1872801792–1876287488]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.steady | [1863014693 [1862804041–1865562014]](cells/cmp-C32.c64/summary.json) |
| rss_over_idle_bytes | [137494528 [136740864–140578816]](cells/cmp-C32.c64/summary.json) |
| rss_per_inflight_over_body | [2098 [2086–2145]](cells/cmp-C32.c64/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| upstream_requests | [6388 [6318–6411]](cells/cmp-C32.c64/summary.json) |

### cmp-C4 — open loop, S4, P-min, 5000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.024ms above the scripted delay + 1 ms (1ms); rep-2: direct p99 1.026ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.045ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C4/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [29326807 [28720613–29425081]](cells/cmp-C4/summary.json) | [29768919 [29686990–29932721]](cells/cmp-C4/summary.json) | [29981674 [29883389–29981695]](cells/cmp-C4/summary.json) | [29981532 [29883250–29981556]](cells/cmp-C4/summary.json) | [29981159 [29882992–29981382]](cells/cmp-C4/summary.json) | [29349171 [29060939–29477536]](cells/cmp-C4/summary.json) |
| Gateway latency | [29327359 [28721151–29425663]](cells/cmp-C4/summary.json) | [29769727 [29687807–29933567]](cells/cmp-C4/summary.json) | [29982719 [29884415–29982719]](cells/cmp-C4/summary.json) | [29982719 [29884415–29982719]](cells/cmp-C4/summary.json) | [29982719 [29884415–29982719]](cells/cmp-C4/summary.json) | [29349730 [29061482–29478124]](cells/cmp-C4/summary.json) |
| Direct latency | [552 [538–582]](cells/cmp-C4/summary.json) | [817 [808–846]](cells/cmp-C4/summary.json) | [1026 [1024–1045]](cells/cmp-C4/summary.json) | [1165 [1163–1187]](cells/cmp-C4/summary.json) | [1423 [1337–1560]](cells/cmp-C4/summary.json) | [559 [543–588]](cells/cmp-C4/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [35.33 [34.67–86.73]](cells/cmp-C4/summary.json) |
| cpu_us_per_request | [109415 [44281–112654]](cells/cmp-C4/summary.json) |
| error_rate | [0.99 [0.98–0.99]](cells/cmp-C4/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.timeouts | [74470 [73699–74480]](cells/cmp-C4/summary.json) |
| errors.total | [74470 [73699–74480]](cells/cmp-C4/summary.json) |
| generator_cpu_pct | [8.00 [7.98–8.01]](cells/cmp-C4/summary.json) |
| lag_us.max | [1294 [867–1838]](cells/cmp-C4/summary.json) |
| lag_us.mean | [92.06 [91.24–95.39]](cells/cmp-C4/summary.json) |
| lag_us.p50 | [94 [91–95]](cells/cmp-C4/summary.json) |
| lag_us.p90 | [175 [171–175]](cells/cmp-C4/summary.json) |
| lag_us.p99 | [233 [231–243]](cells/cmp-C4/summary.json) |
| lag_us.p99_9 | [291 [276–304]](cells/cmp-C4/summary.json) |
| late_fraction | [0.00 [0–0.00]](cells/cmp-C4/summary.json) |
| mock_cpu_pct | [1.03 [1.02–1.03]](cells/cmp-C4/summary.json) |
| rss_bytes.idle | [1735770112 [1735393280–1766977536]](cells/cmp-C4/summary.json) |
| rss_bytes.peak | [1894223872 [1893879808–1912410112]](cells/cmp-C4/summary.json) |
| rss_bytes.steady | [1872854638 [1872770630–1891509589]](cells/cmp-C4/summary.json) |
| rss_over_idle_bytes | [158109696 [145432576–158830592]](cells/cmp-C4/summary.json) |
| success_fraction | [0.01 [0.01–0.02]](cells/cmp-C4/summary.json) |
| upstream_requests | [5691 [5636–5724]](cells/cmp-C4/summary.json) |

### cmp-C5 — open loop, S2, P-min, 1000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.068ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.001ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C5/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [10206540 [9469269–10280333]](cells/cmp-C5/summary.json) | [24722591 [24296625–24984743]](cells/cmp-C5/summary.json) | [29178911 [28752918–29522899]](cells/cmp-C5/summary.json) | [29948883 [29916107–29965163]](cells/cmp-C5/summary.json) | [29997547 [29996800–29997835]](cells/cmp-C5/summary.json) | [11011998 [10449893–11049956]](cells/cmp-C5/summary.json) |
| Gateway latency | [10207231 [9469951–10280959]](cells/cmp-C5/summary.json) | [24723455 [24297471–24985599]](cells/cmp-C5/summary.json) | [29179903 [28753919–29523967]](cells/cmp-C5/summary.json) | [29949951 [29917183–29966335]](cells/cmp-C5/summary.json) | [29999103 [29999103–29999103]](cells/cmp-C5/summary.json) | [11012653 [10450589–11050649]](cells/cmp-C5/summary.json) |
| Direct latency | [682 [626–691]](cells/cmp-C5/summary.json) | [856 [846–864]](cells/cmp-C5/summary.json) | [1001 [992–1068]](cells/cmp-C5/summary.json) | [1076 [1068–1172]](cells/cmp-C5/summary.json) | [1556 [1268–2303]](cells/cmp-C5/summary.json) | [693 [655–696]](cells/cmp-C5/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [653 [639–654]](cells/cmp-C5/summary.json) |
| cpu_us_per_request | [6050 [5987–6188]](cells/cmp-C5/summary.json) |
| error_rate | [0.35 [0.35–0.36]](cells/cmp-C5/summary.json) |
| errors.connect_errors | [1863 [1651–2351]](cells/cmp-C5/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.timeouts | [3328 [3068–3549]](cells/cmp-C5/summary.json) |
| errors.total | [5200 [5191–5419]](cells/cmp-C5/summary.json) |
| generator_cpu_pct | [6.92 [6.88–6.94]](cells/cmp-C5/summary.json) |
| lag_us.max | [372 [318–493]](cells/cmp-C5/summary.json) |
| lag_us.mean | [120 [119–121]](cells/cmp-C5/summary.json) |
| lag_us.p50 | [123 [122–136]](cells/cmp-C5/summary.json) |
| lag_us.p90 | [141 [141–146]](cells/cmp-C5/summary.json) |
| lag_us.p99 | [190 [190–196]](cells/cmp-C5/summary.json) |
| lag_us.p99_9 | [258 [247–260]](cells/cmp-C5/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C5/summary.json) |
| mock_cpu_pct | [0.96 [0.88–1.02]](cells/cmp-C5/summary.json) |
| rss_bytes.idle | [1733357568 [1732714496–1768869888]](cells/cmp-C5/summary.json) |
| rss_bytes.peak | [2163933184 [2161582080–2180567040]](cells/cmp-C5/summary.json) |
| rss_bytes.steady | [2077279713 [2072688128–2088714837]](cells/cmp-C5/summary.json) |
| rss_over_idle_bytes | [428867584 [411697152–430575616]](cells/cmp-C5/summary.json) |
| success_fraction | [0.65 [0.64–0.65]](cells/cmp-C5/summary.json) |
| upstream_requests | [4515 [4471–4592]](cells/cmp-C5/summary.json) |

### cmp-C8 — open loop, S3, P-min, LS, 1000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.001471s above the scripted delay + 1 ms (1.001s); rep-2: direct p99 1.001471s above the scripted delay + 1 ms (1.001s); rep-3: direct p99 1.001471s above the scripted delay + 1 ms (1.001s) Informational: no pass condition. [summary.json](cells/cmp-C8/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [21330432 [20511232–21395968]](cells/cmp-C8/summary.json) | [28047360 [27965440–28178944]](cells/cmp-C8/summary.json) | [28882944 [28604416–28899328]](cells/cmp-C8/summary.json) | [29014016 [28669952–29030400]](cells/cmp-C8/summary.json) | [29029888 [28685824–29079040]](cells/cmp-C8/summary.json) | [19486490 [19177613–19507491]](cells/cmp-C8/summary.json) |
| TTFT overhead | [19095936 [18489856–19227008]](cells/cmp-C8/summary.json) | [26943744 [26747008–27042048]](cells/cmp-C8/summary.json) | [28188672 [27910016–28221440]](cells/cmp-C8/summary.json) | [28466944 [28188416–28614400]](cells/cmp-C8/summary.json) | [28515584 [28204672–28663040]](cells/cmp-C8/summary.json) | [17907169 [17626621–17915995]](cells/cmp-C8/summary.json) |
| Inter-event overhead | [-19476 [-19479–-19466]](cells/cmp-C8/summary.json) | [-19450 [-19469–-19446]](cells/cmp-C8/summary.json) | [1515520 [1467344–1527792]](cells/cmp-C8/summary.json) | [1921888 [1921856–1974112]](cells/cmp-C8/summary.json) | [2225712 [2203200–2293248]](cells/cmp-C8/summary.json) | [35841 [35182–36122]](cells/cmp-C8/summary.json) |
| Gateway latency | [22331391 [21512191–22396927]](cells/cmp-C8/summary.json) | [29048831 [28966911–29179903]](cells/cmp-C8/summary.json) | [29884415 [29605887–29900799]](cells/cmp-C8/summary.json) | [30015487 [29671423–30031871]](cells/cmp-C8/summary.json) | [30031871 [29687807–30081023]](cells/cmp-C8/summary.json) | [20487225 [20178331–20508202]](cells/cmp-C8/summary.json) |
| Direct latency | [1000959 [1000959–1000959]](cells/cmp-C8/summary.json) | [1001471 [1000959–1001471]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001983 [1001983–1001983]](cells/cmp-C8/summary.json) | [1000718 [1000711–1000735]](cells/cmp-C8/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [182 [179–182]](cells/cmp-C8/summary.json) |
| cpu_us_per_request | [21120 [21036–21371]](cells/cmp-C8/summary.json) |
| error_rate | [0.82 [0.82–0.82]](cells/cmp-C8/summary.json) |
| errors.connect_errors | [1262 [1115–1323]](cells/cmp-C8/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.timeouts | [11006 [10992–11162]](cells/cmp-C8/summary.json) |
| errors.total | [12277 [12268–12315]](cells/cmp-C8/summary.json) |
| generator_cpu_pct | [9.25 [9.17–9.43]](cells/cmp-C8/summary.json) |
| lag_us.max | [703 [618–707]](cells/cmp-C8/summary.json) |
| lag_us.mean | [112 [109–142]](cells/cmp-C8/summary.json) |
| lag_us.p50 | [129 [128–144]](cells/cmp-C8/summary.json) |
| lag_us.p90 | [200 [198–229]](cells/cmp-C8/summary.json) |
| lag_us.p99 | [301 [292–333]](cells/cmp-C8/summary.json) |
| lag_us.p99_9 | [417 [366–521]](cells/cmp-C8/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C8/summary.json) |
| mock_cpu_pct | [6.62 [6.57–6.65]](cells/cmp-C8/summary.json) |
| rss_bytes.idle | [1732653056 [1732636672–1732804608]](cells/cmp-C8/summary.json) |
| rss_bytes.peak | [2082873344 [2081587200–2085363712]](cells/cmp-C8/summary.json) |
| rss_bytes.steady | [2037274151 [2036148224–2038827530]](cells/cmp-C8/summary.json) |
| rss_over_idle_bytes | [350068736 [348934144–352727040]](cells/cmp-C8/summary.json) |
| success_fraction | [0.18 [0.18–0.18]](cells/cmp-C8/summary.json) |
| upstream_requests | [1179 [1167–1193]](cells/cmp-C8/summary.json) |

