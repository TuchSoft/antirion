#!/usr/bin/env bash
# Runs one lane's phases on its generator, detached from any ssh session (started with setsid
# nohup by the orchestrator), so it survives the orchestrating machine going away.
# t3 topology:
#   lane.sh A   selftest, (a) compare on the xlarge gateway, (b) C20 on xlarge, then on medium
#   lane.sh B   selftest, (d) C7 on the three-node cluster, (c) the release cells on one xlarge node
# Reference topology (TOPOLOGY=ref in /opt/ab/run/hosts.env):
#   lane.sh A   ref-4: selftest, its share of the comparison (every target interleaved per cell and
#               repetition), then, with WITH_GATES=1, the release cells at 5 reps (gates) until the
#               lane's fill time
#   lane.sh A<k> ref-4, shard k of the comparison: selftest, its share of the comparison
#   lane.sh B   ref-4 with P-cluster: selftest, C7 on three gateway nodes, PostgreSQL and Valkey
#   lane.sh C   ref-16: selftest, C15
# Time limits and cells (/opt/ab/run/lane-<lane>.env, written by run.sh): HARD_END (epoch seconds):
# no phase runs past it (a phase's budget is cut to what is left; with under 2 minutes left it is
# skipped); FILL_END: a "fill" phase runs until then; WITH_GATES: 1 runs the gates; SHARD_CELLS: the
# comparison cells of this shard (ids of /opt/ab/cells/cmp/<id>.yaml; default: every cell of
# cells/comparison.tsv); COMPARE_BUDGET: the comparison phase's timeout in minutes.
# Progress: /var/tmp/ab-out/status ("<phase> start|done rc=<n>|skipped <time>", then "lane done").
# Each phase's results: /var/tmp/ab-out/<phase>/ and <phase>.log; budgets bound every phase.
set -uo pipefail
LANE="$1"
AB=/opt/ab
RUN=$AB/run
OUT=/var/tmp/ab-out
BENCH=("$AB/bin/antirion-bench" --bench-dir "$AB/bench")
mkdir -p "$OUT"
ANTIRION_BENCH_AGENT_TOKEN=$(cat "$RUN/agent.token")
export ANTIRION_BENCH_AGENT_TOKEN
ulimit -n 1048576 2>/dev/null || ulimit -n 524288 2>/dev/null || true
TOPOLOGY=$(sed -n 's/^TOPOLOGY=//p' "$RUN/hosts.env" | tail -1)
TOPOLOGY="${TOPOLOGY:-t3}"
HARD_END=""
FILL_END=""
WITH_GATES=0
SHARD_CELLS=""
COMPARE_BUDGET=120
# shellcheck disable=SC1090
[ -f "$RUN/lane-$LANE.env" ] && source "$RUN/lane-$LANE.env"

status() { echo "$* $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$OUT/status"; }

phase() { # name budget-minutes|fill command...
  local name="$1" budget="$2" left now; shift 2
  now=$(date +%s)
  if [ "$budget" = fill ]; then
    budget=$(( (${FILL_END:-$now} - now) / 60 ))
  fi
  if [ -n "$HARD_END" ]; then
    left=$(( (HARD_END - now) / 60 ))
    [ "$left" -lt "$budget" ] && budget=$left
  fi
  if [ "$budget" -lt 2 ]; then
    status "$name skipped (no time left)"; return 0
  fi
  status "$name start budget=${budget}m"
  echo "== phase $name (budget ${budget} min) at $(date -u +%FT%TZ): $*" > "$OUT/$name.log"
  timeout --kill-after=60 "${budget}m" "$@" >> "$OUT/$name.log" 2>&1
  local rc=$?
  echo "== phase $name ended rc=$rc at $(date -u +%FT%TZ)" >> "$OUT/$name.log"
  status "$name done rc=$rc"
}

selftest() { # name topology-file
  phase "$1" 10 "${BENCH[@]}" selftest --topology "$2" --no-gateway --out "$OUT/$1" --work-dir /var/tmp/abw-selftest
}

"$AB/vm/render.sh" "$LANE" > /dev/null
status "lane $LANE start ($TOPOLOGY)"
case "$TOPOLOGY:$LANE" in
  t3:A)
    selftest selftest-a "$RUN/aws-a.yaml"
    phase compare 110 "${BENCH[@]}" cell --topology "$RUN/aws-a.yaml" --cell-file "$AB/cells/compare.yaml" \
      --interleave "$RUN/targets-xlarge.yaml" --out "$OUT/compare" --work-dir /var/tmp/abw-compare --skip-selftest
    phase c20-xlarge 30 "${BENCH[@]}" cell --topology "$RUN/aws-a.yaml" --cell-file "$AB/cells/c20-xlarge.yaml" \
      --interleave "$RUN/targets-xlarge.yaml" --out "$OUT/c20-xlarge" --work-dir /var/tmp/abw-c20x --skip-selftest
    phase c20-medium 30 "${BENCH[@]}" cell --topology "$RUN/aws-a.yaml" --cell-file "$AB/cells/c20-medium.yaml" \
      --interleave "$RUN/targets-medium.yaml" --out "$OUT/c20-medium" --work-dir /var/tmp/abw-c20m --skip-selftest
    ;;
  t3:B)
    selftest selftest-b "$RUN/aws-b.yaml"
    phase c7-cluster 25 "${BENCH[@]}" cell --topology "$RUN/cluster.yaml" --cell-file "$AB/cells/c7.yaml" \
      --out "$OUT/c7-cluster" --work-dir /var/tmp/abw-c7 --skip-selftest
    phase gates 100 "${BENCH[@]}" cell --topology "$RUN/aws-b.yaml" --cell-file "$AB/cells/gates.yaml" \
      --out "$OUT/gates" --work-dir /var/tmp/abw-gates --skip-selftest
    ;;
  ref:A|ref:A[0-9]*)
    selftest "selftest-$(echo "$LANE" | tr '[:upper:]' '[:lower:]')" "$RUN/ref-4.yaml"
    [ -n "$SHARD_CELLS" ] || SHARD_CELLS=$(awk '{printf "%s ", $1}' "$AB/cells/comparison.tsv")
    : > "$RUN/cells-$LANE.yaml"
    for c in $SHARD_CELLS; do cat "$AB/cells/cmp/$c.yaml" >> "$RUN/cells-$LANE.yaml"; done
    phase compare "$COMPARE_BUDGET" "${BENCH[@]}" cell --topology "$RUN/ref-4.yaml" --cell-file "$RUN/cells-$LANE.yaml" \
      --interleave "$RUN/targets-ref-4.yaml" --out "$OUT/compare" --work-dir /var/tmp/abw-compare --skip-selftest
    if [ "$LANE" = A ] && [ "$WITH_GATES" = 1 ]; then
      # Cut at FILL_END: the cells done by then are complete (run.json is written at the start).
      phase gates fill "${BENCH[@]}" cell --topology "$RUN/ref-4.yaml" --cell-file "$AB/cells/gates5.yaml" \
        --out "$OUT/gates" --work-dir /var/tmp/abw-gates --skip-selftest
    fi
    ;;
  ref:B)
    selftest selftest-b "$RUN/ref-4-cluster.yaml"
    phase c7-cluster 25 "${BENCH[@]}" cell --topology "$RUN/ref-4-cluster.yaml" --cell-file "$AB/cells/c7.yaml" \
      --out "$OUT/c7-cluster" --work-dir /var/tmp/abw-c7 --skip-selftest
    ;;
  ref:C)
    selftest selftest-c "$RUN/ref-16.yaml"
    phase c15 35 "${BENCH[@]}" cell --topology "$RUN/ref-16.yaml" --cell-file "$AB/cells/c15.yaml" \
      --out "$OUT/c15" --work-dir /var/tmp/abw-c15 --skip-selftest
    ;;
  *) echo "usage: lane.sh A|B (t3) or A|A<k>|B|C (ref); this is $TOPOLOGY:$LANE" >&2; exit 2 ;;
esac
cp -R "$AB/inventory" "$OUT/inventory-generator-$LANE" 2>/dev/null || true
status "lane done"
