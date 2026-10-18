# Benchmark report

**Not publishable:** self-test selftest-a: mock-latency failed: L0 at 50000 RPS: p99 1232 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A2/selftest-a2: mock-latency failed: L0 at 50000 RPS: p99 1258 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A2/selftest-a2.log: no result (it did not finish); self-test shards/A3/selftest-a3: mock-latency failed: L0 at 50000 RPS: p99 1098 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A3/selftest-a3.log: no result (it did not finish); self-test shards/A4/selftest-a4: mock-latency failed: L0 at 50000 RPS: p99 1147 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A4/selftest-a4.log: no result (it did not finish); self-test shards/A5/selftest-a5: mock-latency failed: L0 at 50000 RPS: p99 1280 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A5/selftest-a5.log: no result (it did not finish); self-test shards/A6/selftest-a6: mock-latency failed: L0 at 50000 RPS: p99 1102 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A6/selftest-a6.log: no result (it did not finish); self-test shards/A7/selftest-a7: mock-latency failed: L0 at 50000 RPS: p99 949 µs (bound 50 µs), 1500000 of 1500000 ok; self-test shards/A7/selftest-a7.log: no result (it did not finish).

| Run |  |
|---|---|
| Suite | cell |
| Topology | ref-4 |
| Target | antirion 0.1.0 |
| Target commit | unknown (prebuilt binary) |
| Harness commit | d461c20d900198add16216d329f9fdf582a523c8-dirty |
| Corpus seed | 20260930 |
| Started | 2026-10-18T01:11:25Z |
| Ended | 2026-10-18T01:37:28Z |
| Manifest | [run.json](run.json) |

## Cells

Each number is the median over the repetitions with [min–max]. An overhead smaller than the spread of the direct run at the same percentile is shown as within noise.

### H1 — open loop, S1, P-min, 5000 RPS

**Invalid** (5 repetitions): rep-1: direct p99 1.118ms above the scripted delay + 1 ms (1ms); rep-2: direct p99 1.112ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.131ms above the scripted delay + 1 ms (1ms); rep-4: direct p99 1.102ms above the scripted delay + 1 ms (1ms); rep-5: direct p99 1.142ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/H1/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [528 [441–619]](cells/H1/summary.json) | [747 [650–927]](cells/H1/summary.json) | [955 [858–1239]](cells/H1/summary.json) | [1177 [1042–1551]](cells/H1/summary.json) | [16685 [15573–22041]](cells/H1/summary.json) | [555 [466–658]](cells/H1/summary.json) |
| Gateway latency | [1165 [1114–1261]](cells/H1/summary.json) | [1645 [1588–1838]](cells/H1/summary.json) | [2057 [2000–2351]](cells/H1/summary.json) | [2433 [2333–2807]](cells/H1/summary.json) | [18383 [17167–23679]](cells/H1/summary.json) | [1194 [1140–1302]](cells/H1/summary.json) |
| Direct latency | [642 [619–673]](cells/H1/summary.json) | [911 [892–938]](cells/H1/summary.json) | [1118 [1102–1142]](cells/H1/summary.json) | [1270 [1256–1291]](cells/H1/summary.json) | [1594 [1481–1698]](cells/H1/summary.json) | [644 [627–674]](cells/H1/summary.json) |
| x-antirion-overhead-ms | [117 [80–124]](cells/H1/summary.json) | [172 [141–181]](cells/H1/summary.json) | [205 [186–212]](cells/H1/summary.json) | [233 [211–244]](cells/H1/summary.json) | [9543 [9359–10023]](cells/H1/summary.json) | [122 [90.25–129]](cells/H1/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [5000 [5000–5000]](cells/H1/summary.json) |
| cpu_us_per_request | [247 [181–263]](cells/H1/summary.json) |
| error_rate | [0 [0–0]](cells/H1/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/H1/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/H1/summary.json) |
| errors.timeouts | [0 [0–0]](cells/H1/summary.json) |
| errors.total | [0 [0–0]](cells/H1/summary.json) |
| generator_cpu_pct | [8.07 [7.94–8.10]](cells/H1/summary.json) |
| lag_us.max | [626 [486–910]](cells/H1/summary.json) |
| lag_us.mean | [91.52 [85.40–94.32]](cells/H1/summary.json) |
| lag_us.p50 | [65 [62–71]](cells/H1/summary.json) |
| lag_us.p90 | [219 [207–222]](cells/H1/summary.json) |
| lag_us.p99 | [312 [310–323]](cells/H1/summary.json) |
| lag_us.p99_9 | [379 [376–390]](cells/H1/summary.json) |
| late_fraction | [0 [0–0]](cells/H1/summary.json) |
| mock_cpu_pct | [1.11 [1.10–1.12]](cells/H1/summary.json) |
| rss_bytes.idle | [42669397 [42419541–42769067]](cells/H1/summary.json) |
| rss_bytes.peak | [225153024 [221773824–234340352]](cells/H1/summary.json) |
| rss_bytes.steady | [191670322 [188552567–196952164]](cells/H1/summary.json) |
| rss_over_idle_bytes | [182478165 [179004757–191920811]](cells/H1/summary.json) |
| success_fraction | [1 [1–1]](cells/H1/summary.json) |
| upstream_requests | [149978 [149975–149982]](cells/H1/summary.json) |

### cmp-C1 — open loop, S1, P-min, 10000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.118ms above the scripted delay + 1 ms (1ms); rep-2: direct p99 1.121ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.158ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C1/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [610 [530–903]](cells/cmp-C1/summary.json) | [912 [747–2094]](cells/cmp-C1/summary.json) | [1501 [1141–4554]](cells/cmp-C1/summary.json) | [3046 [3031–7589]](cells/cmp-C1/summary.json) | [18227 [16382–23658]](cells/cmp-C1/summary.json) | [665 [572–1163]](cells/cmp-C1/summary.json) |
| Gateway latency | [1219 [1168–1519]](cells/cmp-C1/summary.json) | [1804 [1679–2997]](cells/cmp-C1/summary.json) | [2619 [2299–5675]](cells/cmp-C1/summary.json) | [4355 [4303–8863]](cells/cmp-C1/summary.json) | [19775 [17983–25199]](cells/cmp-C1/summary.json) | [1282 [1216–1786]](cells/cmp-C1/summary.json) |
| Direct latency | [616 [609–638]](cells/cmp-C1/summary.json) | [903 [892–932]](cells/cmp-C1/summary.json) | [1121 [1118–1158]](cells/cmp-C1/summary.json) | [1274 [1272–1309]](cells/cmp-C1/summary.json) | [1548 [1541–1601]](cells/cmp-C1/summary.json) | [623 [618–644]](cells/cmp-C1/summary.json) |
| x-antirion-overhead-ms | [80 [80–99]](cells/cmp-C1/summary.json) | [131 [126–144]](cells/cmp-C1/summary.json) | [191 [179–201]](cells/cmp-C1/summary.json) | [231 [217–378]](cells/cmp-C1/summary.json) | [9903 [9183–10447]](cells/cmp-C1/summary.json) | [88.09 [86.81–104]](cells/cmp-C1/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [10000 [10000–10000]](cells/cmp-C1/summary.json) |
| cpu_us_per_request | [176 [172–205]](cells/cmp-C1/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C1/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C1/summary.json) |
| generator_cpu_pct | [8.90 [8.89–8.92]](cells/cmp-C1/summary.json) |
| lag_us.max | [889 [709–898]](cells/cmp-C1/summary.json) |
| lag_us.mean | [90.47 [88.97–95.84]](cells/cmp-C1/summary.json) |
| lag_us.p50 | [72 [68–73]](cells/cmp-C1/summary.json) |
| lag_us.p90 | [205 [204–219]](cells/cmp-C1/summary.json) |
| lag_us.p99 | [310 [308–334]](cells/cmp-C1/summary.json) |
| lag_us.p99_9 | [388 [379–407]](cells/cmp-C1/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C1/summary.json) |
| mock_cpu_pct | [1.48 [1.46–1.51]](cells/cmp-C1/summary.json) |
| rss_bytes.idle | [42739029 [42445483–42868736]](cells/cmp-C1/summary.json) |
| rss_bytes.peak | [308838400 [304930816–314826752]](cells/cmp-C1/summary.json) |
| rss_bytes.steady | [258263918 [257254546–262359528]](cells/cmp-C1/summary.json) |
| rss_over_idle_bytes | [266392917 [262062080–272087723]](cells/cmp-C1/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C1/summary.json) |
| upstream_requests | [149961 [149959–149969]](cells/cmp-C1/summary.json) |

### cmp-C11 — saturation, S1, P-min, saturation ladder

**Invalid** (1 repetitions): rep-1: direct p99 1.231ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C11/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [606 [606–606]](cells/cmp-C11/summary.json) | [902 [902–902]](cells/cmp-C11/summary.json) | [2726 [2726–2726]](cells/cmp-C11/summary.json) | [8277 [8277–8277]](cells/cmp-C11/summary.json) | [14268 [14268–14268]](cells/cmp-C11/summary.json) | [709 [709–709]](cells/cmp-C11/summary.json) |
| Gateway latency | [1276 [1276–1276]](cells/cmp-C11/summary.json) | [1886 [1886–1886]](cells/cmp-C11/summary.json) | [3957 [3957–3957]](cells/cmp-C11/summary.json) | [9687 [9687–9687]](cells/cmp-C11/summary.json) | [16015 [16015–16015]](cells/cmp-C11/summary.json) | [1386 [1386–1386]](cells/cmp-C11/summary.json) |
| Direct latency | [670 [670–670]](cells/cmp-C11/summary.json) | [984 [984–984]](cells/cmp-C11/summary.json) | [1231 [1231–1231]](cells/cmp-C11/summary.json) | [1410 [1410–1410]](cells/cmp-C11/summary.json) | [1747 [1747–1747]](cells/cmp-C11/summary.json) | [678 [678–678]](cells/cmp-C11/summary.json) |
| x-antirion-overhead-ms | [101 [101–101]](cells/cmp-C11/summary.json) | [134 [134–134]](cells/cmp-C11/summary.json) | [194 [194–194]](cells/cmp-C11/summary.json) | [1068 [1068–1068]](cells/cmp-C11/summary.json) | [3123 [3123–3123]](cells/cmp-C11/summary.json) | [106 [106–106]](cells/cmp-C11/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [14000 [14000–14000]](cells/cmp-C11/summary.json) |
| cpu_us_per_request | [199 [199–199]](cells/cmp-C11/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C11/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C11/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C11/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C11/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C11/summary.json) |
| generator_cpu_pct | [9.20 [9.20–9.20]](cells/cmp-C11/summary.json) |
| lag_us.max | [510 [510–510]](cells/cmp-C11/summary.json) |
| lag_us.mean | [98.02 [98.02–98.02]](cells/cmp-C11/summary.json) |
| lag_us.p50 | [74 [74–74]](cells/cmp-C11/summary.json) |
| lag_us.p90 | [228 [228–228]](cells/cmp-C11/summary.json) |
| lag_us.p99 | [341 [341–341]](cells/cmp-C11/summary.json) |
| lag_us.p99_9 | [410 [410–410]](cells/cmp-C11/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C11/summary.json) |
| mock_cpu_pct | [1.83 [1.83–1.83]](cells/cmp-C11/summary.json) |
| rss_bytes.idle | [42467328 [42467328–42467328]](cells/cmp-C11/summary.json) |
| rss_bytes.peak | [391360512 [391360512–391360512]](cells/cmp-C11/summary.json) |
| rss_bytes.steady | [357228229 [357228229–357228229]](cells/cmp-C11/summary.json) |
| saturation_rps | [14000 [14000–14000]](cells/cmp-C11/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C11/summary.json) |
| upstream_requests | [97936 [97936–97936]](cells/cmp-C11/summary.json) |

### cmp-C13 — saturation, M1, P-min, saturation ladder

**Invalid** (1 repetitions): rep-1: direct p99 1.084ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C13/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [799 [799–799]](cells/cmp-C13/summary.json) | [1661 [1661–1661]](cells/cmp-C13/summary.json) | [4339 [4339–4339]](cells/cmp-C13/summary.json) | [9225 [9225–9225]](cells/cmp-C13/summary.json) | [24536 [24536–24536]](cells/cmp-C13/summary.json) | [1013 [1013–1013]](cells/cmp-C13/summary.json) |
| TTFT overhead | [698 [698–698]](cells/cmp-C13/summary.json) | [1194 [1194–1194]](cells/cmp-C13/summary.json) | [2402 [2402–2402]](cells/cmp-C13/summary.json) | [7007 [7007–7007]](cells/cmp-C13/summary.json) | [11322 [11322–11322]](cells/cmp-C13/summary.json) | [806 [806–806]](cells/cmp-C13/summary.json) |
| Inter-event overhead | [0 [0–0]](cells/cmp-C13/summary.json) | [43 [43–43]](cells/cmp-C13/summary.json) | [417 [417–417]](cells/cmp-C13/summary.json) | [812 [812–812]](cells/cmp-C13/summary.json) | [5191 [5191–5191]](cells/cmp-C13/summary.json) | [25.89 [25.89–25.89]](cells/cmp-C13/summary.json) |
| Gateway latency | [1380 [1380–1380]](cells/cmp-C13/summary.json) | [2523 [2523–2523]](cells/cmp-C13/summary.json) | [5423 [5423–5423]](cells/cmp-C13/summary.json) | [10519 [10519–10519]](cells/cmp-C13/summary.json) | [26031 [26031–26031]](cells/cmp-C13/summary.json) | [1607 [1607–1607]](cells/cmp-C13/summary.json) |
| Direct latency | [581 [581–581]](cells/cmp-C13/summary.json) | [862 [862–862]](cells/cmp-C13/summary.json) | [1084 [1084–1084]](cells/cmp-C13/summary.json) | [1294 [1294–1294]](cells/cmp-C13/summary.json) | [1495 [1495–1495]](cells/cmp-C13/summary.json) | [594 [594–594]](cells/cmp-C13/summary.json) |
| x-antirion-overhead-ms | [150 [150–150]](cells/cmp-C13/summary.json) | [330 [330–330]](cells/cmp-C13/summary.json) | [705 [705–705]](cells/cmp-C13/summary.json) | [1379 [1379–1379]](cells/cmp-C13/summary.json) | [3771 [3771–3771]](cells/cmp-C13/summary.json) | [191 [191–191]](cells/cmp-C13/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [6000 [6000–6000]](cells/cmp-C13/summary.json) |
| cpu_us_per_request | [454 [454–454]](cells/cmp-C13/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C13/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C13/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C13/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C13/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C13/summary.json) |
| generator_cpu_pct | [9.04 [9.04–9.04]](cells/cmp-C13/summary.json) |
| lag_us.max | [405 [405–405]](cells/cmp-C13/summary.json) |
| lag_us.mean | [64.03 [64.03–64.03]](cells/cmp-C13/summary.json) |
| lag_us.p50 | [38 [38–38]](cells/cmp-C13/summary.json) |
| lag_us.p90 | [166 [166–166]](cells/cmp-C13/summary.json) |
| lag_us.p99 | [253 [253–253]](cells/cmp-C13/summary.json) |
| lag_us.p99_9 | [299 [299–299]](cells/cmp-C13/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C13/summary.json) |
| mock_cpu_pct | [1.56 [1.56–1.56]](cells/cmp-C13/summary.json) |
| rss_bytes.idle | [42531499 [42531499–42531499]](cells/cmp-C13/summary.json) |
| rss_bytes.peak | [943828992 [943828992–943828992]](cells/cmp-C13/summary.json) |
| rss_bytes.steady | [929003678 [929003678–929003678]](cells/cmp-C13/summary.json) |
| saturation_rps | [6000 [6000–6000]](cells/cmp-C13/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C13/summary.json) |
| upstream_requests | [41993 [41993–41993]](cells/cmp-C13/summary.json) |

### cmp-C16 — idle, P-min, no load

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C16/summary.json)

| Metric | median [min–max] |
|---|---|
| rss_bytes.idle | [42507916 [41797446–42803945]](cells/cmp-C16/summary.json) |
| rss_bytes.peak | [42573824 [42487808–42868736]](cells/cmp-C16/summary.json) |
| rss_bytes.steady | [42507916 [41797446–42803945]](cells/cmp-C16/summary.json) |

### cmp-C17 — open loop, S1, P-min, L1500, 5000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.502207s above the scripted delay + 1 ms (1.501s); rep-2: direct p99 1.502207s above the scripted delay + 1 ms (1.501s); rep-3: direct p99 1.502207s above the scripted delay + 1 ms (1.501s) Informational: no pass condition. [summary.json](cells/cmp-C17/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [0 [0–0]](cells/cmp-C17/summary.json) | [1024 [1024–1024]](cells/cmp-C17/summary.json) | [0 [0–0]](cells/cmp-C17/summary.json) | [6144 [6144–6144]](cells/cmp-C17/summary.json) | [13312 [12288–13312]](cells/cmp-C17/summary.json) | [258 [254–298]](cells/cmp-C17/summary.json) |
| Gateway latency | [1501183 [1501183–1501183]](cells/cmp-C17/summary.json) | [1502207 [1502207–1502207]](cells/cmp-C17/summary.json) | [1502207 [1502207–1502207]](cells/cmp-C17/summary.json) | [1508351 [1508351–1508351]](cells/cmp-C17/summary.json) | [1515519 [1515519–1516543]](cells/cmp-C17/summary.json) | [1500944 [1500940–1501006]](cells/cmp-C17/summary.json) |
| Direct latency | [1501183 [1501183–1501183]](cells/cmp-C17/summary.json) | [1501183 [1501183–1501183]](cells/cmp-C17/summary.json) | [1502207 [1502207–1502207]](cells/cmp-C17/summary.json) | [1502207 [1502207–1502207]](cells/cmp-C17/summary.json) | [1503231 [1502207–1503231]](cells/cmp-C17/summary.json) | [1500687 [1500685–1500708]](cells/cmp-C17/summary.json) |
| x-antirion-overhead-ms | [107 [107–108]](cells/cmp-C17/summary.json) | [133 [133–135]](cells/cmp-C17/summary.json) | [160 [160–161]](cells/cmp-C17/summary.json) | [196 [195–199]](cells/cmp-C17/summary.json) | [10711 [10623–10951]](cells/cmp-C17/summary.json) | [109 [109–110]](cells/cmp-C17/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [5000 [5000–5000]](cells/cmp-C17/summary.json) |
| cpu_us_per_request | [230 [230–231]](cells/cmp-C17/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C17/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C17/summary.json) |
| generator_cpu_pct | [8.16 [8.15–8.22]](cells/cmp-C17/summary.json) |
| lag_us.max | [503 [500–503]](cells/cmp-C17/summary.json) |
| lag_us.mean | [83.43 [78.07–90.95]](cells/cmp-C17/summary.json) |
| lag_us.p50 | [49 [38–61]](cells/cmp-C17/summary.json) |
| lag_us.p90 | [224 [211–232]](cells/cmp-C17/summary.json) |
| lag_us.p99 | [354 [331–361]](cells/cmp-C17/summary.json) |
| lag_us.p99_9 | [422 [407–430]](cells/cmp-C17/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C17/summary.json) |
| mock_cpu_pct | [4.11 [4.09–4.16]](cells/cmp-C17/summary.json) |
| rss_bytes.idle | [42745856 [42668032–42758144]](cells/cmp-C17/summary.json) |
| rss_bytes.peak | [1254289408 [1253822464–1256976384]](cells/cmp-C17/summary.json) |
| rss_bytes.steady | [1098105018 [1097601769–1098504192]](cells/cmp-C17/summary.json) |
| rss_over_idle_bytes | [1211543552 [1211064320–1214308352]](cells/cmp-C17/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C17/summary.json) |
| upstream_requests | [74982 [74978–74983]](cells/cmp-C17/summary.json) |

### cmp-C2 — open loop, S1, P-min, 1000 RPS

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C2/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [761 [750–828]](cells/cmp-C2/summary.json) | [881 [847–932]](cells/cmp-C2/summary.json) | [951 [866–1003]](cells/cmp-C2/summary.json) | [1020 [914–1103]](cells/cmp-C2/summary.json) | [2465 [2270–2491]](cells/cmp-C2/summary.json) | [721 [715–763]](cells/cmp-C2/summary.json) |
| Gateway latency | [1530 [1459–1595]](cells/cmp-C2/summary.json) | [1719 [1709–1770]](cells/cmp-C2/summary.json) | [1859 [1850–1910]](cells/cmp-C2/summary.json) | [1994 [1989–2075]](cells/cmp-C2/summary.json) | [3503 [3397–3619]](cells/cmp-C2/summary.json) | [1492 [1420–1531]](cells/cmp-C2/summary.json) |
| Direct latency | [767 [709–769]](cells/cmp-C2/summary.json) | [838 [838–862]](cells/cmp-C2/summary.json) | [908 [907–984]](cells/cmp-C2/summary.json) | [972 [969–1080]](cells/cmp-C2/summary.json) | [1127 [1012–1154]](cells/cmp-C2/summary.json) | [768 [705–771]](cells/cmp-C2/summary.json) |
| x-antirion-overhead-ms | [154 [153–155]](cells/cmp-C2/summary.json) | [175 [171–175]](cells/cmp-C2/summary.json) | [190 [187–191]](cells/cmp-C2/summary.json) | [239 [234–248]](cells/cmp-C2/summary.json) | [2241 [2235–2393]](cells/cmp-C2/summary.json) | [153 [153–155]](cells/cmp-C2/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [1000 [1000–1000]](cells/cmp-C2/summary.json) |
| cpu_us_per_request | [351 [345–352]](cells/cmp-C2/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C2/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C2/summary.json) |
| generator_cpu_pct | [6.87 [6.87–6.88]](cells/cmp-C2/summary.json) |
| lag_us.max | [316 [315–317]](cells/cmp-C2/summary.json) |
| lag_us.mean | [138 [136–140]](cells/cmp-C2/summary.json) |
| lag_us.p50 | [135 [134–135]](cells/cmp-C2/summary.json) |
| lag_us.p90 | [173 [169–173]](cells/cmp-C2/summary.json) |
| lag_us.p99 | [221 [218–226]](cells/cmp-C2/summary.json) |
| lag_us.p99_9 | [262 [254–264]](cells/cmp-C2/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C2/summary.json) |
| mock_cpu_pct | [0.41 [0.40–0.44]](cells/cmp-C2/summary.json) |
| rss_bytes.idle | [42603861 [42385408–42672128]](cells/cmp-C2/summary.json) |
| rss_bytes.peak | [91230208 [90120192–91279360]](cells/cmp-C2/summary.json) |
| rss_bytes.steady | [72742619 [72608427–73394274]](cells/cmp-C2/summary.json) |
| rss_over_idle_bytes | [48558080 [47516331–48893952]](cells/cmp-C2/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C2/summary.json) |
| upstream_requests | [14996 [14996–14996]](cells/cmp-C2/summary.json) |

### cmp-C21.401 — open loop, S8, P-min, 20000 RPS

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C21.401/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Gateway latency | [299 [298–299]](cells/cmp-C21.401/summary.json) | [490 [486–496]](cells/cmp-C21.401/summary.json) | [747 [745–748]](cells/cmp-C21.401/summary.json) | [877 [874–877]](cells/cmp-C21.401/summary.json) | [1122 [1102–1246]](cells/cmp-C21.401/summary.json) | [331 [330–333]](cells/cmp-C21.401/summary.json) |
| x-antirion-overhead-ms | [10 [10–10]](cells/cmp-C21.401/summary.json) | [19 [19–19]](cells/cmp-C21.401/summary.json) | [33 [33–33]](cells/cmp-C21.401/summary.json) | [50 [49–51]](cells/cmp-C21.401/summary.json) | [162 [140–251]](cells/cmp-C21.401/summary.json) | [11.81 [11.76–11.94]](cells/cmp-C21.401/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [20000 [20000–20000]](cells/cmp-C21.401/summary.json) |
| cpu_us_per_request | [61 [60.97–61.07]](cells/cmp-C21.401/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| generator_cpu_pct | [13.12 [13.08–13.22]](cells/cmp-C21.401/summary.json) |
| lag_us.max | [362 [360–364]](cells/cmp-C21.401/summary.json) |
| lag_us.mean | [10.63 [10.28–10.64]](cells/cmp-C21.401/summary.json) |
| lag_us.p50 | [11 [10–11]](cells/cmp-C21.401/summary.json) |
| lag_us.p90 | [16 [14–16]](cells/cmp-C21.401/summary.json) |
| lag_us.p99 | [26 [24–26]](cells/cmp-C21.401/summary.json) |
| lag_us.p99_9 | [58 [56–61]](cells/cmp-C21.401/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| mock_cpu_pct | [0 [0–0]](cells/cmp-C21.401/summary.json) |
| rss_bytes.idle | [42699435 [42397696–42719915]](cells/cmp-C21.401/summary.json) |
| rss_bytes.peak | [49217536 [47636480–73416704]](cells/cmp-C21.401/summary.json) |
| rss_bytes.steady | [47268815 [45584774–55500410]](cells/cmp-C21.401/summary.json) |
| rss_over_idle_bytes | [6497621 [5238784–30717269]](cells/cmp-C21.401/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C21.401/summary.json) |
| upstream_requests | [0 [0–0]](cells/cmp-C21.401/summary.json) |

### cmp-C21.404 — open loop, S8-404, P-min, 20000 RPS

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C21.404/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Gateway latency | [571 [554–580]](cells/cmp-C21.404/summary.json) | [940 [891–951]](cells/cmp-C21.404/summary.json) | [1272 [1215–1288]](cells/cmp-C21.404/summary.json) | [1508 [1438–1543]](cells/cmp-C21.404/summary.json) | [2311 [1919–6667]](cells/cmp-C21.404/summary.json) | [594 [575–605]](cells/cmp-C21.404/summary.json) |
| x-antirion-overhead-ms | [17 [17–25]](cells/cmp-C21.404/summary.json) | [35 [32–61]](cells/cmp-C21.404/summary.json) | [80 [78–89]](cells/cmp-C21.404/summary.json) | [98 [96–105]](cells/cmp-C21.404/summary.json) | [281 [209–360]](cells/cmp-C21.404/summary.json) | [22.02 [21.34–30.65]](cells/cmp-C21.404/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [20000 [20000–20000]](cells/cmp-C21.404/summary.json) |
| cpu_us_per_request | [39.27 [37.77–55.33]](cells/cmp-C21.404/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| generator_cpu_pct | [9.52 [9.46–9.55]](cells/cmp-C21.404/summary.json) |
| lag_us.max | [520 [502–522]](cells/cmp-C21.404/summary.json) |
| lag_us.mean | [95.04 [93.36–96.04]](cells/cmp-C21.404/summary.json) |
| lag_us.p50 | [70 [68–71]](cells/cmp-C21.404/summary.json) |
| lag_us.p90 | [226 [223–227]](cells/cmp-C21.404/summary.json) |
| lag_us.p99 | [341 [337–343]](cells/cmp-C21.404/summary.json) |
| lag_us.p99_9 | [409 [406–412]](cells/cmp-C21.404/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| mock_cpu_pct | [0 [0–0]](cells/cmp-C21.404/summary.json) |
| rss_bytes.idle | [39744853 [38607531–39854080]](cells/cmp-C21.404/summary.json) |
| rss_bytes.peak | [62537728 [44318720–76734464]](cells/cmp-C21.404/summary.json) |
| rss_bytes.steady | [61028645 [43762249–73472683]](cells/cmp-C21.404/summary.json) |
| rss_over_idle_bytes | [22792875 [4464640–38126933]](cells/cmp-C21.404/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C21.404/summary.json) |
| upstream_requests | [0 [0–0]](cells/cmp-C21.404/summary.json) |

### cmp-C32.c64 — closed loop, S1, P-min, closed loop, 64 connections

Valid, 3 repetitions. Informational: no pass condition. [summary.json](cells/cmp-C32.c64/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [2802 [2685–2874]](cells/cmp-C32.c64/summary.json) | [3634 [3392–3671]](cells/cmp-C32.c64/summary.json) | [6152 [5663–6175]](cells/cmp-C32.c64/summary.json) | [10613 [10245–13649]](cells/cmp-C32.c64/summary.json) | [26735 [20113–26795]](cells/cmp-C32.c64/summary.json) | [2772 [2762–2788]](cells/cmp-C32.c64/summary.json) |
| Gateway latency | [2983 [2867–3055]](cells/cmp-C32.c64/summary.json) | [3933 [3691–3975]](cells/cmp-C32.c64/summary.json) | [6619 [6127–6659]](cells/cmp-C32.c64/summary.json) | [11303 [10887–14295]](cells/cmp-C32.c64/summary.json) | [27983 [21695–28703]](cells/cmp-C32.c64/summary.json) | [2973 [2960–2986]](cells/cmp-C32.c64/summary.json) |
| Direct latency | [181 [181–182]](cells/cmp-C32.c64/summary.json) | [299 [299–304]](cells/cmp-C32.c64/summary.json) | [467 [464–484]](cells/cmp-C32.c64/summary.json) | [646 [642–690]](cells/cmp-C32.c64/summary.json) | [1582 [1248–1908]](cells/cmp-C32.c64/summary.json) | [198 [198–201]](cells/cmp-C32.c64/summary.json) |
| x-antirion-overhead-ms | [94 [93–94]](cells/cmp-C32.c64/summary.json) | [127 [127–128]](cells/cmp-C32.c64/summary.json) | [178 [177–180]](cells/cmp-C32.c64/summary.json) | [1831 [1799–1912]](cells/cmp-C32.c64/summary.json) | [8311 [7787–8863]](cells/cmp-C32.c64/summary.json) | [104 [103–104]](cells/cmp-C32.c64/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [21512 [21415–21604]](cells/cmp-C32.c64/summary.json) |
| cpu_us_per_request | [185 [184–186]](cells/cmp-C32.c64/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| generator_cpu_pct | [41.36 [41.01–41.54]](cells/cmp-C32.c64/summary.json) |
| lag_us.max | [27 [21–27]](cells/cmp-C32.c64/summary.json) |
| lag_us.mean | [1.00 [1.00–1.00]](cells/cmp-C32.c64/summary.json) |
| lag_us.p50 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p90 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p99 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| lag_us.p99_9 | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C32.c64/summary.json) |
| mock_cpu_pct | [29.90 [29.56–29.99]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.idle | [42561536 [42206549–42764971]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.peak | [403607552 [400465920–437096448]](cells/cmp-C32.c64/summary.json) |
| rss_bytes.steady | [334924654 [328448098–346160762]](cells/cmp-C32.c64/summary.json) |
| rss_over_idle_bytes | [361046016 [358259371–394331477]](cells/cmp-C32.c64/summary.json) |
| rss_per_inflight_over_body | [5509 [5467–6017]](cells/cmp-C32.c64/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C32.c64/summary.json) |
| upstream_requests | [322609 [321157–324017]](cells/cmp-C32.c64/summary.json) |

### cmp-C4 — open loop, S4, P-min, 5000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.034ms above the scripted delay + 1 ms (1ms); rep-2: direct p99 1.07ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.069ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C4/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [805 [747–836]](cells/cmp-C4/summary.json) | [1214 [1044–1238]](cells/cmp-C4/summary.json) | [1933 [1482–2073]](cells/cmp-C4/summary.json) | [6655 [5561–14216]](cells/cmp-C4/summary.json) | [21048 [16367–24423]](cells/cmp-C4/summary.json) | [901 [821–909]](cells/cmp-C4/summary.json) |
| Gateway latency | [1432 [1373–1439]](cells/cmp-C4/summary.json) | [2069 [1930–2127]](cells/cmp-C4/summary.json) | [2967 [2551–3143]](cells/cmp-C4/summary.json) | [7855 [6735–15407]](cells/cmp-C4/summary.json) | [22383 [17743–25775]](cells/cmp-C4/summary.json) | [1512 [1447–1536]](cells/cmp-C4/summary.json) |
| Direct latency | [626 [596–634]](cells/cmp-C4/summary.json) | [886 [855–889]](cells/cmp-C4/summary.json) | [1069 [1034–1070]](cells/cmp-C4/summary.json) | [1191 [1174–1200]](cells/cmp-C4/summary.json) | [1352 [1335–1376]](cells/cmp-C4/summary.json) | [627 [603–635]](cells/cmp-C4/summary.json) |
| x-antirion-overhead-ms | [215 [207–215]](cells/cmp-C4/summary.json) | [269 [261–273]](cells/cmp-C4/summary.json) | [318 [309–322]](cells/cmp-C4/summary.json) | [394 [356–430]](cells/cmp-C4/summary.json) | [8167 [7995–8231]](cells/cmp-C4/summary.json) | [216 [209–217]](cells/cmp-C4/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [5000 [5000–5000]](cells/cmp-C4/summary.json) |
| cpu_us_per_request | [417 [401–420]](cells/cmp-C4/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C4/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C4/summary.json) |
| generator_cpu_pct | [8.09 [8.09–8.12]](cells/cmp-C4/summary.json) |
| lag_us.max | [337 [333–360]](cells/cmp-C4/summary.json) |
| lag_us.mean | [85.87 [79.69–86.13]](cells/cmp-C4/summary.json) |
| lag_us.p50 | [59 [58–59]](cells/cmp-C4/summary.json) |
| lag_us.p90 | [201 [183–203]](cells/cmp-C4/summary.json) |
| lag_us.p99 | [262 [257–263]](cells/cmp-C4/summary.json) |
| lag_us.p99_9 | [294 [292–295]](cells/cmp-C4/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C4/summary.json) |
| mock_cpu_pct | [1.07 [1.07–1.07]](cells/cmp-C4/summary.json) |
| rss_bytes.idle | [42654379 [42448213–42725376]](cells/cmp-C4/summary.json) |
| rss_bytes.peak | [220114944 [218382336–223047680]](cells/cmp-C4/summary.json) |
| rss_bytes.steady | [169515691 [163511345–173309172]](cells/cmp-C4/summary.json) |
| rss_over_idle_bytes | [177389568 [175934123–180393301]](cells/cmp-C4/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C4/summary.json) |
| upstream_requests | [74980 [74980–74982]](cells/cmp-C4/summary.json) |

### cmp-C5 — open loop, S2, P-min, 1000 RPS

**Invalid** (3 repetitions): rep-2: direct p99 1.012ms above the scripted delay + 1 ms (1ms); rep-3: direct p99 1.06ms above the scripted delay + 1 ms (1ms) Informational: no pass condition. [summary.json](cells/cmp-C5/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [928 [755–980]](cells/cmp-C5/summary.json) | [1029 [852–1127]](cells/cmp-C5/summary.json) | [1169 [1085–1235]](cells/cmp-C5/summary.json) | [1340 [1194–1452]](cells/cmp-C5/summary.json) | [2809 [2570–5557]](cells/cmp-C5/summary.json) | [908 [768–986]](cells/cmp-C5/summary.json) |
| Gateway latency | [1653 [1454–1656]](cells/cmp-C5/summary.json) | [1970 [1711–1970]](cells/cmp-C5/summary.json) | [2229 [2035–2247]](cells/cmp-C5/summary.json) | [2337 [2329–2523]](cells/cmp-C5/summary.json) | [3897 [3763–6683]](cells/cmp-C5/summary.json) | [1666 [1461–1687]](cells/cmp-C5/summary.json) |
| Direct latency | [699 [676–725]](cells/cmp-C5/summary.json) | [859 [843–941]](cells/cmp-C5/summary.json) | [1012 [950–1060]](cells/cmp-C5/summary.json) | [1071 [989–1143]](cells/cmp-C5/summary.json) | [1126 [1088–1193]](cells/cmp-C5/summary.json) | [701 [692–758]](cells/cmp-C5/summary.json) |
| x-antirion-overhead-ms | [323 [319–332]](cells/cmp-C5/summary.json) | [456 [384–557]](cells/cmp-C5/summary.json) | [714 [568–731]](cells/cmp-C5/summary.json) | [801 [726–857]](cells/cmp-C5/summary.json) | [2561 [2435–5183]](cells/cmp-C5/summary.json) | [351 [333–372]](cells/cmp-C5/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [1000 [1000–1000]](cells/cmp-C5/summary.json) |
| cpu_us_per_request | [623 [613–627]](cells/cmp-C5/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C5/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C5/summary.json) |
| generator_cpu_pct | [6.95 [6.88–6.95]](cells/cmp-C5/summary.json) |
| lag_us.max | [285 [213–332]](cells/cmp-C5/summary.json) |
| lag_us.mean | [117 [44.52–118]](cells/cmp-C5/summary.json) |
| lag_us.p50 | [121 [12–121]](cells/cmp-C5/summary.json) |
| lag_us.p90 | [128 [123–131]](cells/cmp-C5/summary.json) |
| lag_us.p99 | [172 [149–173]](cells/cmp-C5/summary.json) |
| lag_us.p99_9 | [201 [191–207]](cells/cmp-C5/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C5/summary.json) |
| mock_cpu_pct | [1.10 [1.05–1.14]](cells/cmp-C5/summary.json) |
| rss_bytes.idle | [42702165 [42695339–42749952]](cells/cmp-C5/summary.json) |
| rss_bytes.peak | [92454912 [92200960–94793728]](cells/cmp-C5/summary.json) |
| rss_bytes.steady | [74915840 [74505655–76494165]](cells/cmp-C5/summary.json) |
| rss_over_idle_bytes | [49752747 [49451008–52098389]](cells/cmp-C5/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C5/summary.json) |
| upstream_requests | [14997 [14996–14998]](cells/cmp-C5/summary.json) |

### cmp-C8 — open loop, S3, P-min, LS, 1000 RPS

**Invalid** (3 repetitions): rep-1: direct p99 1.001471s above the scripted delay + 1 ms (1.001s); rep-2: direct p99 1.001471s above the scripted delay + 1 ms (1.001s); rep-3: direct p99 1.001471s above the scripted delay + 1 ms (1.001s) Informational: no pass condition. [summary.json](cells/cmp-C8/summary.json)

| µs | p50 | p90 | p99 | p99.9 | max | mean |
|---|---|---|---|---|---|---|
| Overhead | [512 [512–512]](cells/cmp-C8/summary.json) | [0 [0–0]](cells/cmp-C8/summary.json) | [512 [512–512]](cells/cmp-C8/summary.json) | [512 [512–512]](cells/cmp-C8/summary.json) | [4608 [1536–5632]](cells/cmp-C8/summary.json) | [301 [252–353]](cells/cmp-C8/summary.json) |
| TTFT overhead | [384 [384–512]](cells/cmp-C8/summary.json) | [256 [256–384]](cells/cmp-C8/summary.json) | [384 [256–384]](cells/cmp-C8/summary.json) | [384 [384–384]](cells/cmp-C8/summary.json) | [4864 [1408–5376]](cells/cmp-C8/summary.json) | [360 [319–384]](cells/cmp-C8/summary.json) |
| Inter-event overhead | [-16 [-16–-16]](cells/cmp-C8/summary.json) | [-16 [-48–0]](cells/cmp-C8/summary.json) | [-16 [-48–-16]](cells/cmp-C8/summary.json) | [16 [-16–32]](cells/cmp-C8/summary.json) | [4032 [2368–5120]](cells/cmp-C8/summary.json) | [-476 [-476–-476]](cells/cmp-C8/summary.json) |
| Gateway latency | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001983 [1001983–1001983]](cells/cmp-C8/summary.json) | [1001983 [1001983–1001983]](cells/cmp-C8/summary.json) | [1006591 [1003519–1007615]](cells/cmp-C8/summary.json) | [1001036 [1000979–1001086]](cells/cmp-C8/summary.json) |
| Direct latency | [1000959 [1000959–1000959]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001471 [1001471–1001471]](cells/cmp-C8/summary.json) | [1001983 [1001983–1001983]](cells/cmp-C8/summary.json) | [1000733 [1000728–1000735]](cells/cmp-C8/summary.json) |

| Metric | median [min–max] |
|---|---|
| achieved_rps | [1000 [1000–1000]](cells/cmp-C8/summary.json) |
| cpu_us_per_request | [1425 [1406–1457]](cells/cmp-C8/summary.json) |
| error_rate | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.connect_errors | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.non_2xx | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.timeouts | [0 [0–0]](cells/cmp-C8/summary.json) |
| errors.total | [0 [0–0]](cells/cmp-C8/summary.json) |
| generator_cpu_pct | [9.39 [9.27–9.48]](cells/cmp-C8/summary.json) |
| lag_us.max | [396 [387–424]](cells/cmp-C8/summary.json) |
| lag_us.mean | [65.15 [53.04–69.42]](cells/cmp-C8/summary.json) |
| lag_us.p50 | [36 [23–36]](cells/cmp-C8/summary.json) |
| lag_us.p90 | [170 [148–183]](cells/cmp-C8/summary.json) |
| lag_us.p99 | [286 [286–292]](cells/cmp-C8/summary.json) |
| lag_us.p99_9 | [351 [346–352]](cells/cmp-C8/summary.json) |
| late_fraction | [0 [0–0]](cells/cmp-C8/summary.json) |
| mock_cpu_pct | [6.65 [6.62–6.69]](cells/cmp-C8/summary.json) |
| rss_bytes.idle | [42790912 [42610688–42840064]](cells/cmp-C8/summary.json) |
| rss_bytes.peak | [207273984 [204681216–208928768]](cells/cmp-C8/summary.json) |
| rss_bytes.steady | [181352076 [180839570–182278516]](cells/cmp-C8/summary.json) |
| rss_over_idle_bytes | [164663296 [161841152–166137856]](cells/cmp-C8/summary.json) |
| success_fraction | [1 [1–1]](cells/cmp-C8/summary.json) |
| upstream_requests | [14996 [14996–14997]](cells/cmp-C8/summary.json) |

