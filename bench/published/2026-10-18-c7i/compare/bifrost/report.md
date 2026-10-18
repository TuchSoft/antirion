# Benchmark report

**Not publishable:** self-test selftest-a: mock-latency failed: L0 at 50000 RPS: p99 1232 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A2/selftest-a2: mock-latency failed: L0 at 50000 RPS: p99 1258 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A2/selftest-a2.log: no result (it did not finish); self-test shards/A3/selftest-a3: mock-latency failed: L0 at 50000 RPS: p99 1098 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A3/selftest-a3.log: no result (it did not finish); self-test shards/A4/selftest-a4: mock-latency failed: L0 at 50000 RPS: p99 1147 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A4/selftest-a4.log: no result (it did not finish); self-test shards/A5/selftest-a5: mock-latency failed: L0 at 50000 RPS: p99 1280 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A5/selftest-a5.log: no result (it did not finish); self-test shards/A6/selftest-a6: mock-latency failed: L0 at 50000 RPS: p99 1102 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A6/selftest-a6.log: no result (it did not finish); self-test shards/A7/selftest-a7: mock-latency failed: L0 at 50000 RPS: p99 949 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A7/selftest-a7.log: no result (it did not finish).

| Run |  |
|---|---|
| Suite | cell |
| Topology | ref-4 |
| Target | bifrost transports/v2.2.4 |
| Target commit | n/a |
| Harness commit | d461c20d900198add16216d329f9fdf582a523c8-dirty |
| Corpus seed | 20260930 |
| Started | 2026-10-18T01:11:24Z |
| Ended | 2026-10-18T01:37:28Z |
| Manifest | [run.json](run.json) |

## Cells

Each number is the median over the repetitions with [min–max]. An overhead smaller than the spread of the direct run at the same percentile is shown as within noise.

### H1 — open loop, S1, P-min, 5000 RPS

**Invalid** (5 repetitions): rep-1: direct p99 1.082ms above the scripted delay + 1 ms (1ms); rep-2: direct p99 1.156ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.134ms above the scripted delay + 1 ms (1ms); rep-4: direct p99 1.12ms above the scripted delay + 1 ms (1ms); rep-5: direct p99 1.129ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/H1/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [19727 [16486–20594]](cells/H1/summary.json) | [70478 [67479–72217]](cells/H1/summary.json) | [95163 [90385–96854]](cells/H1/summary.json) | [119429 [110657–127343]](cells/H1/summary.json) | [203436 [180823–221870]](cells/H1/summary.json) | [27814 [26782–28746]](cells/H1/summary.json) |
| Gateway latency | [20399 [17151–21167]](cells/H1/summary.json) | [71423 [68415–73151]](cells/H1/summary.json) | [96319 [91519–97983]](cells/H1/summary.json) | [120703 [111935–128639]](cells/H1/summary.json) | [204927 [182399–223615]](cells/H1/summary.json) | [28484 [27450–29419]](cells/H1/summary.json) |
| Direct latency | [665 [573–672]](cells/H1/summary.json) | [934 [856–945]](cells/H1/summary.json) | [1129 [1082–1156]](cells/H1/summary.json) | [1278 [1242–1296]](cells/H1/summary.json) | [1524 [1491–1745]](cells/H1/summary.json) | [668 [582–673]](cells/H1/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [5000 [5000–5000]](cells/H1/summary.json) |
| cpu_us_per_request | [717 [707–721]](cells/H1/summary.json) |
| error_rate | [0 [0–0]](cells/H1/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/H1/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/H1/summary.json) |
| errors.timeouts | [0 [0–0]](cells/H1/summary.json) |
| errors.total | [0 [0–0]](cells/H1/summary.json) |
| generator_cpu_pct | [8.13 [8.12–8.20]](cells/H1/summary.json) |
| lag_us.max | [886 [499–1117]](cells/H1/summary.json) |
| lag_us.mean | [86.74 [85.25–88.44]](cells/H1/summary.json) |
| lag_us.p50 | [59 [56–60]](cells/H1/summary.json) |
| lag_us.p90 | [215 [210–221]](cells/H1/summary.json) |
| lag_us.p99 | [317 [309–323]](cells/H1/summary.json) |
| lag_us.p99_9 | [386 [375–395]](cells/H1/summary.json) |
| late_fraction | [0 [0–0.00]](cells/H1/summary.json) |
| mock_cpu_pct | [1.13 [1.11–1.14]](cells/H1/summary.json) |
| rss_bytes.idle | [522317824 [516616192–538578944]](cells/H1/summary.json) |
| rss_bytes.peak | [1456803840 [1420951552–1717800960]](cells/H1/summary.json) |
| rss_bytes.steady | [1320003734 [1289307910–1417211504]](cells/H1/summary.json) |
| rss_over_idle_bytes | [940187648 [889352192–1195483136]](cells/H1/summary.json) |
| success_fraction | [1 [1–1]](cells/H1/summary.json) |
| upstream_requests | [150106 [149952–150176]](cells/H1/summary.json) |

### cmp-C1 — open loop, S1, P-min, 10000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.116ms above the scripted delay + 1 ms (1ms); rep-2: direct p99 1.106ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.122ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C1/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [12123541 [11222442–12279205]](cells/cmp-C1/summary.json) | [17792123 [16972937–18414722]](cells/cmp-C1/summary.json) | [19102627 [18152365–19577757]](cells/cmp-C1/summary.json) | [19544832 [18561808–20019968]](cells/cmp-C1/summary.json) | [20134376 [19315220–21002566]](cells/cmp-C1/summary.json) | [12140604 [11409327–12338609]](cells/cmp-C1/summary.json) |
| Gateway latency | [12124159 [11223039–12279807]](cells/cmp-C1/summary.json) | [17793023 [16973823–18415615]](cells/cmp-C1/summary.json) | [19103743 [18153471–19578879]](cells/cmp-C1/summary.json) | [19546111 [18563071–20021247]](cells/cmp-C1/summary.json) | [20135935 [19316735–21004287]](cells/cmp-C1/summary.json) | [12141230 [11409935–12339222]](cells/cmp-C1/summary.json) |
| Direct latency | [602 [597–618]](cells/cmp-C1/summary.json) | [893 [886–900]](cells/cmp-C1/summary.json) | [1116 [1106–1122]](cells/cmp-C1/summary.json) | [1279 [1263–1279]](cells/cmp-C1/summary.json) | [1559 [1515–1721]](cells/cmp-C1/summary.json) | [613 [607–626]](cells/cmp-C1/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [10000 [10000–10000]](cells/cmp-C1/summary.json) |
| cpu_us_per_request | [399 [399–399]](cells/cmp-C1/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C1/summary.json) |
| generator_cpu_pct | [8.64 [8.62–8.66]](cells/cmp-C1/summary.json) |
| lag_us.max | [471 [452–481]](cells/cmp-C1/summary.json) |
| lag_us.mean | [74.93 [73.27–75.08]](cells/cmp-C1/summary.json) |
| lag_us.p50 | [48 [44–50]](cells/cmp-C1/summary.json) |
| lag_us.p90 | [184 [183–185]](cells/cmp-C1/summary.json) |
| lag_us.p99 | [295 [293–296]](cells/cmp-C1/summary.json) |
| lag_us.p99_9 | [362 [357–365]](cells/cmp-C1/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C1/summary.json) |
| mock_cpu_pct | [1.47 [1.43–1.48]](cells/cmp-C1/summary.json) |
| rss_bytes.idle | [538624000 [519684096–542887936]](cells/cmp-C1/summary.json) |
| rss_bytes.peak | [3442122752 [3330543616–3911458816]](cells/cmp-C1/summary.json) |
| rss_bytes.steady | [3187936870 [3010545086–3475094187]](cells/cmp-C1/summary.json) |
| rss_over_idle_bytes | [2903498752 [2787655680–3391774720]](cells/cmp-C1/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C1/summary.json) |
| upstream_requests | [80040 [78883–83661]](cells/cmp-C1/summary.json) |

### cmp-C11 — saturation, S1, P-min, saturation ladder

**Invalid** (1 repetitions): rep-1: direct p99 1.055ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C11/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [557 [557–557]](cells/cmp-C11/summary.json) | [710 [710–710]](cells/cmp-C11/summary.json) | [2536 [2536–2536]](cells/cmp-C11/summary.json) | [5050 [5050–5050]](cells/cmp-C11/summary.json) | [7749 [7749–7749]](cells/cmp-C11/summary.json) | [633 [633–633]](cells/cmp-C11/summary.json) |
| Gateway latency | [1128 [1128–1128]](cells/cmp-C11/summary.json) | [1542 [1542–1542]](cells/cmp-C11/summary.json) | [3591 [3591–3591]](cells/cmp-C11/summary.json) | [6243 [6243–6243]](cells/cmp-C11/summary.json) | [9111 [9111–9111]](cells/cmp-C11/summary.json) | [1211 [1211–1211]](cells/cmp-C11/summary.json) |
| Direct latency | [571 [571–571]](cells/cmp-C11/summary.json) | [832 [832–832]](cells/cmp-C11/summary.json) | [1055 [1055–1055]](cells/cmp-C11/summary.json) | [1193 [1193–1193]](cells/cmp-C11/summary.json) | [1362 [1362–1362]](cells/cmp-C11/summary.json) | [577 [577–577]](cells/cmp-C11/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [3500 [3500–3500]](cells/cmp-C11/summary.json) |
| cpu_us_per_request | [717 [717–717]](cells/cmp-C11/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C11/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C11/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C11/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C11/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C11/summary.json) |
| generator_cpu_pct | [7.70 [7.70–7.70]](cells/cmp-C11/summary.json) |
| lag_us.max | [467 [467–467]](cells/cmp-C11/summary.json) |
| lag_us.mean | [110 [110–110]](cells/cmp-C11/summary.json) |
| lag_us.p50 | [94 [94–94]](cells/cmp-C11/summary.json) |
| lag_us.p90 | [240 [240–240]](cells/cmp-C11/summary.json) |
| lag_us.p99 | [331 [331–331]](cells/cmp-C11/summary.json) |
| lag_us.p99_9 | [400 [400–400]](cells/cmp-C11/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C11/summary.json) |
| mock_cpu_pct | [0.98 [0.98–0.98]](cells/cmp-C11/summary.json) |
| rss_bytes.idle | [524087296 [524087296–524087296]](cells/cmp-C11/summary.json) |
| rss_bytes.peak | [1172668416 [1172668416–1172668416]](cells/cmp-C11/summary.json) |
| rss_bytes.steady | [1071547313 [1071547313–1071547313]](cells/cmp-C11/summary.json) |
| saturation_rps | [3500 [3500–3500]](cells/cmp-C11/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C11/summary.json) |
| upstream_requests | [24484 [24484–24484]](cells/cmp-C11/summary.json) |

### cmp-C13 — saturation, M1, P-min, saturation ladder

Valid, 1 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C13/summary.json)

| Metric | median [min–max] |
|---|---|
| rss_bytes.idle | [533553152 [533553152–533553152]](cells/cmp-C13/summary.json) |
| saturation_rps | [0 [0–0]](cells/cmp-C13/summary.json) |

### cmp-C16 — idle, P-min, no load

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C16/summary.json)

| Metric | median [min–max] |
|---|---|
| rss_bytes.idle | [528478208 [526360576–540925952]](cells/cmp-C16/summary.json) |
| rss_bytes.peak | [528478208 [526360576–540925952]](cells/cmp-C16/summary.json) |
| rss_bytes.steady | [528478208 [526360576–540925952]](cells/cmp-C16/summary.json) |

### cmp-C17 — open loop, S1, P-min, L1500, 5000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.502207s above the scripted delay + 1 ms (1.501s); rep-2: direct p99 1.502207s above the scripted delay + 1 ms (1.501s); rep-3: direct p99 1.502207s above the scripted delay + 1 ms (1.501s) Informational: no pass condition. [summary.json](cells/cmp-C17/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [6686720 [6420480–6719488]](cells/cmp-C17/summary.json) | [9795584 [9721856–9828352]](cells/cmp-C17/summary.json) | [10482688 [10368000–10671104]](cells/cmp-C17/summary.json) | [10687488 [10400768–10720256]](cells/cmp-C17/summary.json) | [10694656 [10490880–10719232]](cells/cmp-C17/summary.json) | [6619428 [6447644–6636738]](cells/cmp-C17/summary.json) |
| Gateway latency | [8187903 [7921663–8220671]](cells/cmp-C17/summary.json) | [11296767 [11223039–11329535]](cells/cmp-C17/summary.json) | [11984895 [11870207–12173311]](cells/cmp-C17/summary.json) | [12189695 [11902975–12222463]](cells/cmp-C17/summary.json) | [12197887 [11993087–12222463]](cells/cmp-C17/summary.json) | [8120127 [7948326–8137430]](cells/cmp-C17/summary.json) |
| Direct latency | [1501183 [1501183–1501183]](cells/cmp-C17/summary.json) | [1501183 [1501183–1501183]](cells/cmp-C17/summary.json) | [1502207 [1502207–1502207]](cells/cmp-C17/summary.json) | [1502207 [1502207–1502207]](cells/cmp-C17/summary.json) | [1503231 [1502207–1503231]](cells/cmp-C17/summary.json) | [1500692 [1500682–1500698]](cells/cmp-C17/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [5000 [5000–5000]](cells/cmp-C17/summary.json) |
| cpu_us_per_request | [495 [491–495]](cells/cmp-C17/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C17/summary.json) |
| generator_cpu_pct | [7.98 [7.97–8.08]](cells/cmp-C17/summary.json) |
| lag_us.max | [545 [508–588]](cells/cmp-C17/summary.json) |
| lag_us.mean | [91.46 [90.96–94.81]](cells/cmp-C17/summary.json) |
| lag_us.p50 | [61 [58–63]](cells/cmp-C17/summary.json) |
| lag_us.p90 | [232 [225–232]](cells/cmp-C17/summary.json) |
| lag_us.p99 | [337 [328–347]](cells/cmp-C17/summary.json) |
| lag_us.p99_9 | [412 [400–414]](cells/cmp-C17/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C17/summary.json) |
| mock_cpu_pct | [4.06 [4.03–4.12]](cells/cmp-C17/summary.json) |
| rss_bytes.idle | [533794816 [526217216–543985664]](cells/cmp-C17/summary.json) |
| rss_bytes.peak | [3401388032 [3391074304–3447169024]](cells/cmp-C17/summary.json) |
| rss_bytes.steady | [3023668565 [3017968768–3094919478]](cells/cmp-C17/summary.json) |
| rss_over_idle_bytes | [2867593216 [2864857088–2903183360]](cells/cmp-C17/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C17/summary.json) |
| upstream_requests | [49321 [48853–49962]](cells/cmp-C17/summary.json) |

### cmp-C2 — open loop, S1, P-min, 1000 RPS

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C2/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [718 [572–979]](cells/cmp-C2/summary.json) | [937 [701–1210]](cells/cmp-C2/summary.json) | [1160 [876–1356]](cells/cmp-C2/summary.json) | [2645 [2489–2675]](cells/cmp-C2/summary.json) | [4116 [4046–4260]](cells/cmp-C2/summary.json) | [711 [616–1008]](cells/cmp-C2/summary.json) |
| Gateway latency | [1331 [1325–1495]](cells/cmp-C2/summary.json) | [1613 [1610–1782]](cells/cmp-C2/summary.json) | [1855 [1812–2071]](cells/cmp-C2/summary.json) | [3561 [3177–3635]](cells/cmp-C2/summary.json) | [5059 [5031–5543]](cells/cmp-C2/summary.json) | [1365 [1363–1488]](cells/cmp-C2/summary.json) |
| Direct latency | [759 [346–777]](cells/cmp-C2/summary.json) | [845 [400–912]](cells/cmp-C2/summary.json) | [911 [456–979]](cells/cmp-C2/summary.json) | [960 [532–1072]](cells/cmp-C2/summary.json) | [1013 [915–1283]](cells/cmp-C2/summary.json) | [747 [357–777]](cells/cmp-C2/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [1000 [1000–1000]](cells/cmp-C2/summary.json) |
| cpu_us_per_request | [897 [885–907]](cells/cmp-C2/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C2/summary.json) |
| generator_cpu_pct | [6.87 [6.86–6.87]](cells/cmp-C2/summary.json) |
| lag_us.max | [370 [278–464]](cells/cmp-C2/summary.json) |
| lag_us.mean | [137 [126–138]](cells/cmp-C2/summary.json) |
| lag_us.p50 | [134 [132–136]](cells/cmp-C2/summary.json) |
| lag_us.p90 | [177 [173–196]](cells/cmp-C2/summary.json) |
| lag_us.p99 | [221 [219–276]](cells/cmp-C2/summary.json) |
| lag_us.p99_9 | [245 [240–341]](cells/cmp-C2/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C2/summary.json) |
| mock_cpu_pct | [0.40 [0.40–0.44]](cells/cmp-C2/summary.json) |
| rss_bytes.idle | [542081024 [531353600–553807872]](cells/cmp-C2/summary.json) |
| rss_bytes.peak | [854302720 [849645568–855547904]](cells/cmp-C2/summary.json) |
| rss_bytes.steady | [790265466 [790122106–791502458]](cells/cmp-C2/summary.json) |
| rss_over_idle_bytes | [312221696 [301740032–318291968]](cells/cmp-C2/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C2/summary.json) |
| upstream_requests | [14995 [14995–14996]](cells/cmp-C2/summary.json) |

### cmp-C21.401 — open loop, S8, P-min, 20000 RPS

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C21.401/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Gateway latency | [1858559 [1805311–2529279]](cells/cmp-C21.401/summary.json) | [2375679 [2347007–3106815]](cells/cmp-C21.401/summary.json) | [2492415 [2473983–3219455]](cells/cmp-C21.401/summary.json) | [2539519 [2504703–3246079]](cells/cmp-C21.401/summary.json) | [2646015 [2615295–3311615]](cells/cmp-C21.401/summary.json) | [1855698 [1815985–2510456]](cells/cmp-C21.401/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [20000 [20000–20000]](cells/cmp-C21.401/summary.json) |
| cpu_us_per_request | [199 [199–199]](cells/cmp-C21.401/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| generator_cpu_pct | [10.69 [10.62–10.78]](cells/cmp-C21.401/summary.json) |
| lag_us.max | [617 [616–953]](cells/cmp-C21.401/summary.json) |
| lag_us.mean | [30.80 [24.64–33.69]](cells/cmp-C21.401/summary.json) |
| lag_us.p50 | [12 [11–12]](cells/cmp-C21.401/summary.json) |
| lag_us.p90 | [82 [59–93]](cells/cmp-C21.401/summary.json) |
| lag_us.p99 | [258 [226–271]](cells/cmp-C21.401/summary.json) |
| lag_us.p99_9 | [372 [343–379]](cells/cmp-C21.401/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| mock_cpu_pct | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| rss_bytes.idle | [533790720 [511729664–536350720]](cells/cmp-C21.401/summary.json) |
| rss_bytes.peak | [2209931264 [2206683136–3291766784]](cells/cmp-C21.401/summary.json) |
| rss_bytes.steady | [2089080031 [2060031377–2379993429]](cells/cmp-C21.401/summary.json) |
| rss_over_idle_bytes | [1694953472 [1676140544–2755416064]](cells/cmp-C21.401/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C21.401/summary.json) |
| upstream_requests | [0 [0–0]](cells/cmp-C21.401/summary.json) |

### cmp-C21.404 — open loop, S8-404, P-min, 20000 RPS

**Invalid** (3 repetitions): rep-3: 0.18 % of gateway requests left over 1 ms late Informational: no pass condition. [summary.json](cells/cmp-C21.404/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Gateway latency | [17661951 [16670719–18038783]](cells/cmp-C21.404/summary.json) | [25608191 [24330239–25788415]](cells/cmp-C21.404/summary.json) | [27361279 [25952255–27459583]](cells/cmp-C21.404/summary.json) | [27475967 [26148863–27688959]](cells/cmp-C21.404/summary.json) | [27885567 [26558463–28065791]](cells/cmp-C21.404/summary.json) | [17676344 [16678710–17947531]](cells/cmp-C21.404/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [20000 [20000–20000]](cells/cmp-C21.404/summary.json) |
| cpu_us_per_request | [200 [200–200]](cells/cmp-C21.404/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| generator_cpu_pct | [9.09 [9.05–9.11]](cells/cmp-C21.404/summary.json) |
| lag_us.max | [947 [543–27743]](cells/cmp-C21.404/summary.json) |
| lag_us.mean | [61.52 [57.83–75.91]](cells/cmp-C21.404/summary.json) |
| lag_us.p50 | [18 [13–21]](cells/cmp-C21.404/summary.json) |
| lag_us.p90 | [166 [153–172]](cells/cmp-C21.404/summary.json) |
| lag_us.p99 | [308 [307–312]](cells/cmp-C21.404/summary.json) |
| lag_us.p99_9 | [399 [395–13103]](cells/cmp-C21.404/summary.json) |
| late_fraction | [0 [0–0.00]](cells/cmp-C21.404/summary.json) |
| mock_cpu_pct | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| rss_bytes.idle | [536535040 [535072768–538128384]](cells/cmp-C21.404/summary.json) |
| rss_bytes.peak | [3090612224 [2333814784–4391657472]](cells/cmp-C21.404/summary.json) |
| rss_bytes.steady | [2413742507 [2238834383–2671309141]](cells/cmp-C21.404/summary.json) |
| rss_over_idle_bytes | [2554077184 [1798742016–3853529088]](cells/cmp-C21.404/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C21.404/summary.json) |
| upstream_requests | [0 [0–0]](cells/cmp-C21.404/summary.json) |

### cmp-C32.c64 — closed loop, S1, P-min, closed loop, 64 connections

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C32.c64/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [7878 [7855–7880]](cells/cmp-C32.c64/summary.json) | [18417 [18407–18456]](cells/cmp-C32.c64/summary.json) | [32147 [31938–32194]](cells/cmp-C32.c64/summary.json) | [41991 [41401–43960]](cells/cmp-C32.c64/summary.json) | [121400 [94373–124366]](cells/cmp-C32.c64/summary.json) | [9531 [9528–9548]](cells/cmp-C32.c64/summary.json) |
| Gateway latency | [8059 [8039–8063]](cells/cmp-C32.c64/summary.json) | [18719 [18719–18767]](cells/cmp-C32.c64/summary.json) | [32623 [32447–32703]](cells/cmp-C32.c64/summary.json) | [42655 [42143–44703]](cells/cmp-C32.c64/summary.json) | [123775 [95743–127615]](cells/cmp-C32.c64/summary.json) | [9735 [9727–9751]](cells/cmp-C32.c64/summary.json) |
| Direct latency | [183 [181–184]](cells/cmp-C32.c64/summary.json) | [311 [302–312]](cells/cmp-C32.c64/summary.json) | [509 [476–509]](cells/cmp-C32.c64/summary.json) | [742 [664–743]](cells/cmp-C32.c64/summary.json) | [2375 [1370–3249]](cells/cmp-C32.c64/summary.json) | [204 [199–204]](cells/cmp-C32.c64/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [6568 [6561–6575]](cells/cmp-C32.c64/summary.json) |
| cpu_us_per_request | [606 [605–607]](cells/cmp-C32.c64/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| generator_cpu_pct | [40.43 [39.94–40.77]](cells/cmp-C32.c64/summary.json) |
| lag_us.max | [22 [21–27]](cells/cmp-C32.c64/summary.json) |
| lag_us.mean | [1.00 [1.00–1.00]](cells/cmp-C32.c64/summary.json) |
| lag_us.p50 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p90 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p99 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p99_9 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| mock_cpu_pct | [28.84 [28.83–29.90]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.idle | [543002624 [533581824–547028992]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.peak | [1120378880 [1118121984–1133993984]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.steady | [1057001667 [1048548693–1063256454]](cells/cmp-C32.c64/summary.json) |
| rss_over_idle_bytes | [584540160 [577376256–586964992]](cells/cmp-C32.c64/summary.json) |
| rss_per_inflight_over_body | [8825 [8716–8861]](cells/cmp-C32.c64/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| upstream_requests | [98533 [98400–98629]](cells/cmp-C32.c64/summary.json) |

### cmp-C4 — open loop, S4, P-min, 5000 RPS

**Invalid** (3 repetitions): rep-2: direct p99 1.067ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.043ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C4/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [2499 [1680–4149]](cells/cmp-C4/summary.json) | [49062 [42345–49360]](cells/cmp-C4/summary.json) | [70047 [62644–70892]](cells/cmp-C4/summary.json) | [85407 [82261–87336]](cells/cmp-C4/summary.json) | [145326 [123390–161756]](cells/cmp-C4/summary.json) | [15252 [13296–16248]](cells/cmp-C4/summary.json) |
| Gateway latency | [3131 [2281–4663]](cells/cmp-C4/summary.json) | [49919 [43231–50143]](cells/cmp-C4/summary.json) | [71039 [63711–71935]](cells/cmp-C4/summary.json) | [86527 [83455–88511]](cells/cmp-C4/summary.json) | [146687 [124799–163455]](cells/cmp-C4/summary.json) | [15856 [13928–16771]](cells/cmp-C4/summary.json) |
| Direct latency | [601 [514–632]](cells/cmp-C4/summary.json) | [857 [783–886]](cells/cmp-C4/summary.json) | [1043 [992–1067]](cells/cmp-C4/summary.json) | [1175 [1120–1194]](cells/cmp-C4/summary.json) | [1409 [1361–1699]](cells/cmp-C4/summary.json) | [605 [523–631]](cells/cmp-C4/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [5000 [5000–5000]](cells/cmp-C4/summary.json) |
| cpu_us_per_request | [661 [661–668]](cells/cmp-C4/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C4/summary.json) |
| generator_cpu_pct | [8.00 [7.99–8.05]](cells/cmp-C4/summary.json) |
| lag_us.max | [421 [374–457]](cells/cmp-C4/summary.json) |
| lag_us.mean | [70.27 [69.99–79.26]](cells/cmp-C4/summary.json) |
| lag_us.p50 | [40 [39–49]](cells/cmp-C4/summary.json) |
| lag_us.p90 | [180 [177–195]](cells/cmp-C4/summary.json) |
| lag_us.p99 | [254 [253–257]](cells/cmp-C4/summary.json) |
| lag_us.p99_9 | [288 [287–291]](cells/cmp-C4/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C4/summary.json) |
| mock_cpu_pct | [1.07 [1.07–1.07]](cells/cmp-C4/summary.json) |
| rss_bytes.idle | [524058624 [519884800–543961088]](cells/cmp-C4/summary.json) |
| rss_bytes.peak | [1468571648 [1381195776–1751506944]](cells/cmp-C4/summary.json) |
| rss_bytes.steady | [1299837903 [1262006857–1415373775]](cells/cmp-C4/summary.json) |
| rss_over_idle_bytes | [948686848 [857137152–1207545856]](cells/cmp-C4/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C4/summary.json) |
| upstream_requests | [74985 [74984–74988]](cells/cmp-C4/summary.json) |

### cmp-C5 — open loop, S2, P-min, 1000 RPS

**Invalid** (3 repetitions): rep-3: direct p99 1.024ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C5/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [1476 [1458–1487]](cells/cmp-C5/summary.json) | [15169 [14962–16047]](cells/cmp-C5/summary.json) | [27246 [26321–28527]](cells/cmp-C5/summary.json) | [38288 [37189–41107]](cells/cmp-C5/summary.json) | [58234 [53926–65444]](cells/cmp-C5/summary.json) | [5293 [5223–5631]](cells/cmp-C5/summary.json) |
| Gateway latency | [2113 [2113–2119]](cells/cmp-C5/summary.json) | [15895 [15711–16863]](cells/cmp-C5/summary.json) | [28223 [27135–29551]](cells/cmp-C5/summary.json) | [39487 [38047–42207]](cells/cmp-C5/summary.json) | [59519 [54847–66623]](cells/cmp-C5/summary.json) | [5934 [5848–6282]](cells/cmp-C5/summary.json) |
| Direct latency | [637 [626–661]](cells/cmp-C5/summary.json) | [749 [726–816]](cells/cmp-C5/summary.json) | [977 [814–1024]](cells/cmp-C5/summary.json) | [1100 [858–1199]](cells/cmp-C5/summary.json) | [1179 [921–1285]](cells/cmp-C5/summary.json) | [641 [626–651]](cells/cmp-C5/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [1000 [1000–1000]](cells/cmp-C5/summary.json) |
| cpu_us_per_request | [2335 [2318–2352]](cells/cmp-C5/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C5/summary.json) |
| generator_cpu_pct | [6.96 [6.95–6.98]](cells/cmp-C5/summary.json) |
| lag_us.max | [301 [300–309]](cells/cmp-C5/summary.json) |
| lag_us.mean | [113 [112–115]](cells/cmp-C5/summary.json) |
| lag_us.p50 | [121 [121–121]](cells/cmp-C5/summary.json) |
| lag_us.p90 | [149 [145–157]](cells/cmp-C5/summary.json) |
| lag_us.p99 | [203 [198–215]](cells/cmp-C5/summary.json) |
| lag_us.p99_9 | [256 [253–269]](cells/cmp-C5/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C5/summary.json) |
| mock_cpu_pct | [1.05 [0.87–1.11]](cells/cmp-C5/summary.json) |
| rss_bytes.idle | [531668992 [530927616–533553152]](cells/cmp-C5/summary.json) |
| rss_bytes.peak | [1187733504 [1161998336–1472163840]](cells/cmp-C5/summary.json) |
| rss_bytes.steady | [1115786874 [1112825661–1147219188]](cells/cmp-C5/summary.json) |
| rss_over_idle_bytes | [656805888 [630329344–938610688]](cells/cmp-C5/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C5/summary.json) |
| upstream_requests | [15001 [14993–15007]](cells/cmp-C5/summary.json) |

### cmp-C8 — open loop, S3, P-min, LS, 1000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.001471s above the scripted delay + 1 ms (1.001s); rep-2: direct p99 1.001471s above the scripted delay + 1 ms (1.001s); rep-3: direct p99 1.001471s above the scripted delay + 1 ms (1.001s) Informational: no pass condition. [summary.json](cells/cmp-C8/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [12288 [11776–20480]](cells/cmp-C8/summary.json) | [104448 [103424–115712]](cells/cmp-C8/summary.json) | [150528 [149504–173056]](cells/cmp-C8/summary.json) | [184320 [177152–217088]](cells/cmp-C8/summary.json) | [243200 [239104–287232]](cells/cmp-C8/summary.json) | [36356 [36307–42625]](cells/cmp-C8/summary.json) |
| TTFT overhead | [38784 [38528–44672]](cells/cmp-C8/summary.json) | [83840 [83328–95360]](cells/cmp-C8/summary.json) | [107904 [104832–126080]](cells/cmp-C8/summary.json) | [144640 [125440–171008]](cells/cmp-C8/summary.json) | [236544 [236288–272384]](cells/cmp-C8/summary.json) | [39165 [39072–45458]](cells/cmp-C8/summary.json) |
| Inter-event overhead | [-160 [-256–-160]](cells/cmp-C8/summary.json) | [17552 [17376–18784]](cells/cmp-C8/summary.json) | [66320 [66000–74800]](cells/cmp-C8/summary.json) | [98880 [95360–119584]](cells/cmp-C8/summary.json) | [215328 [209296–220432]](cells/cmp-C8/summary.json) | [-550 [-552–-548]](cells/cmp-C8/summary.json) |
| Gateway latency | [1013247 [1012735–1021439]](cells/cmp-C8/summary.json) | [1105919 [1104895–1117183]](cells/cmp-C8/summary.json) | [1151999 [1150975–1174527]](cells/cmp-C8/summary.json) | [1185791 [1178623–1218559]](cells/cmp-C8/summary.json) | [1245183 [1241087–1289215]](cells/cmp-C8/summary.json) | [1037082 [1037050–1043370]](cells/cmp-C8/summary.json) |
| Direct latency | [1000959 [1000959–1000959]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001983 [1001983–1001983]](cells/cmp-C8/summary.json) | [1000743 [1000727–1000745]](cells/cmp-C8/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [1000 [1000–1000]](cells/cmp-C8/summary.json) |
| cpu_us_per_request | [3585 [3581–3713]](cells/cmp-C8/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C8/summary.json) |
| generator_cpu_pct | [9.36 [9.23–9.50]](cells/cmp-C8/summary.json) |
| lag_us.max | [541 [513–559]](cells/cmp-C8/summary.json) |
| lag_us.mean | [59.71 [58.06–61.57]](cells/cmp-C8/summary.json) |
| lag_us.p50 | [26 [18–31]](cells/cmp-C8/summary.json) |
| lag_us.p90 | [161 [156–201]](cells/cmp-C8/summary.json) |
| lag_us.p99 | [283 [281–348]](cells/cmp-C8/summary.json) |
| lag_us.p99_9 | [380 [365–437]](cells/cmp-C8/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C8/summary.json) |
| mock_cpu_pct | [9.03 [8.88–9.06]](cells/cmp-C8/summary.json) |
| rss_bytes.idle | [531451904 [530018304–535478272]](cells/cmp-C8/summary.json) |
| rss_bytes.peak | [1380585472 [1367330816–1389260800]](cells/cmp-C8/summary.json) |
| rss_bytes.steady | [1252271942 [1243881844–1252816151]](cells/cmp-C8/summary.json) |
| rss_over_idle_bytes | [845107200 [837312512–857808896]](cells/cmp-C8/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C8/summary.json) |
| upstream_requests | [14947 [14944–14996]](cells/cmp-C8/summary.json) |

