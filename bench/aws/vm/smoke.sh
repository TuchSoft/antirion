#!/usr/bin/env bash
# Single-machine check of the bundle on a fresh VM (run by run.sh --smoke --setup, as root, after
# setup.sh gateway): every target (vm/targets.sh) started by the harness through the local agent,
# against the mock on this machine's private address, a few seconds per cell.
set -euo pipefail
AB=/opt/ab
OUT=/var/tmp/ab-smoke
PRIV=$(hostname -I | awk '{print $1}')
rm -rf "$OUT" && mkdir -p "$OUT"
cat > "$OUT/topology.yaml" <<EOF
name: aws-smoke
reference: false
gateway:
  launch: local
  bin: "/opt/ab/bin/antirion"
  nodes:
    - {data: "$PRIV:8080", admin: "$PRIV:8081", agent: "127.0.0.1:19500"}
mock:
  launch: local
  listen: "$PRIV:19401"
  agent: "127.0.0.1:19500"
  workers: 1
generator:
  workers: 1
  max_connections: 512
selftest: {mock_rate: 500, mock_p99_us: 5000, duration_s: 2}
EOF
"$AB/vm/targets.sh" "$PRIV" 127.0.0.1 "$PRIV:19401" > "$OUT/targets.yaml"
cat > "$OUT/cells.yaml" <<'EOF'
- {cell: smoke-S1, kind: open, scenario: S1, profile: P-min, level: {rps: 50}, compare: true, informational: true, gates: [], warmup_s: 1, measure_s: 3, repetitions: 2}
- {cell: smoke-S4, kind: open, scenario: S4, profile: P-min, level: {rps: 50}, compare: true, informational: true, gates: [], warmup_s: 1, measure_s: 3, repetitions: 1}
- {cell: smoke-S8, kind: open, scenario: S8, profile: P-min, level: {rps: 50}, compare: true, informational: true, gates: [], warmup_s: 1, measure_s: 3, repetitions: 1}
- {cell: smoke-S8-404, kind: open, scenario: S8-404, profile: P-min, level: {rps: 50}, compare: true, informational: true, gates: [], warmup_s: 1, measure_s: 3, repetitions: 1}
EOF
ANTIRION_BENCH_AGENT_TOKEN=$(cat "$AB/run/agent.token")
export ANTIRION_BENCH_AGENT_TOKEN
ulimit -n 1048576
"$AB/bin/antirion-bench" --bench-dir "$AB/bench" cell --topology "$OUT/topology.yaml" --cell-file "$OUT/cells.yaml" \
  --interleave "$OUT/targets.yaml" --out "$OUT/out" --work-dir /var/tmp/abw-smoke --skip-selftest 2>&1 | tee "$OUT/smoke.log"
echo "listeners left: $(ss -Hltn '( sport = :18080 or sport = :18090 or sport = :8080 or sport = :19401 )' | wc -l)"
