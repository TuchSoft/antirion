#!/usr/bin/env bash
# Functions reached through traps and passed by name (cost_lines mm_expected) look unused to shellcheck.
# shellcheck disable=SC2329
# Orchestrates the benchmark run on AWS EC2 (default VPC, one AZ) with the aws CLI. The whole
# procedure, what it measures and how to read the results: docs/benchmarks.md.
#
#   run.sh --topology ref [OPTIONS] --plan    machines, lanes, timeline, cost estimate, dead-man time,
#                                              vCPU quota and credential lifetime; creates nothing
#   run.sh --topology ref [OPTIONS] [--run-id ID]
#                                              create, set up, start the lanes, follow, fetch, tear down
#   run.sh --resume ID [--now]                 reattach to a run (its options come from its state):
#                                              follow it to the end (--now: fetch what is done at
#                                              once), fetch, tear down
#   run.sh [--topology t3|ref] --smoke [--setup]
#                                              one machine through the create, ssh and teardown flow
#   run.sh --topology t3 [--plan]              the t3 reproduction of a published setup
# OPTIONS (reference topology):
#   --cells compare|headline|<id,...>   the comparison cells (default compare: H1 and every cmp-*
#                                       cell; headline: H1; ids, e.g. H1,C11,cmp-C13); --only is the same
#   --shards N                          split them over N lane-A machine sets (A, A2 .. AN)
#   --deadline 45m                      pick the fewest shards whose plan fits, within the quota
#   --lanes A,B,C                       the lanes (default A: the comparison); B: C7 on a 3-node
#                                       cluster; C: C15 on a 16 vCPU gateway
#   --with-gates                        lane A also runs Antirion's release cells after its share
#
# Topology ref (the reference: topology files named ref-4 and ref-16, reference: true):
#   Lane A (and A2 .. AN): gateway c7i.xlarge, generator and mock c7i.4xlarge: the selected
#   comparison cells, every target interleaved per cell and repetition. With --shards the cells are
#   balanced by expected duration (gen-cells.py's comparison.tsv); every cell still runs all targets
#   on one machine set, so cells are compared within a shard, never across. The shards' results are
#   merged into one compare/ directory. With --with-gates, lane A then runs the release cells at
#   5 reps until the planned wall time (WALL_MIN, default: long enough for all of them).
#   Lane B (ref-4, P-cluster): C7 on three c7i.xlarge gateway nodes, PostgreSQL and Valkey each on a
#   c7i.2xlarge, generator and mock c7i.4xlarge. Lane C (ref-16): C15 on a c7i.4xlarge gateway,
#   generator and mock c7i.8xlarge.
#   Every machine is in one AZ and one cluster placement group, tagged and deleted with the run.
# Topology t3 (another gateway's published setup; burstable, so never publishable as reference):
#   lane A: (a) the comparison, every target interleaved, on the t3.xlarge gateway, then
#           (b) C20 for every target on the t3.xlarge gateway and on the t3.medium gateway
#   lane B: (d) C7 on three t3.xlarge nodes with PostgreSQL and Valkey on a t3.medium, then
#           (c) the release cells (gates) on one of those t3.xlarge nodes
# The phases run on the generators under setsid nohup and survive this machine going away (and the
# AWS credentials expiring). Each lane's machines are terminated as soon as the lane is done;
# everything is torn down at the end, on errors and on signals. Every resource is tagged
# Project=antirion-bench and RunId=<id>; teardown.sh <id> deletes them by tag. Each instance also
# terminates itself (dead-man switch): t3, at the longest lane's budget plus a margin; ref, at its
# lane's planned time plus 30 min. So a lost laptop or expired credentials cannot leave machines running.
#
# Credentials: before creating anything, the credentials must be valid and, for an `aws login`
# session, last the planned wall time plus 30 min (AB_LOGIN_SESSION_MIN: the session lifetime assumed,
# default 600; AB_IGNORE_CRED_EXPIRY=1 skips the check).
# Environment:
#   AB_REGION      the region (default us-east-1; the price table is us-east-1's)
#   AB_ACCOUNT     when set, the run refuses any other AWS account
#   AB_STATE_DIR   each run's state, ssh key and logs, under runs/<id>/ (default bench/aws/.state)
#   AB_RESULTS_DIR where results land, under <id>/ (default bench/results)
#   BUNDLE         the bundle (default bench/aws/.build/ab-bundle.tgz, from build-bundle.sh)
#   BENCH_SRC      the checkout whose bench/ renders reports (default: this repository)
#   REPORT_BIN     a native antirion-bench for the reports (default: the one build-bundle.sh builds)
#   WALL_MIN       ref, --with-gates: planned wall time (default: lane A's comparison share plus
#                  every release cell)
#   COST_CAP       ref: refuse above this expected cost in USD, default 15
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
STATE_ROOT="${AB_STATE_DIR:-$HERE/.state}"
RESULTS_ROOT="${AB_RESULTS_DIR:-$REPO/bench/results}"
REGION="${AB_REGION:-us-east-1}"
ACCOUNT="${AB_ACCOUNT:-}"
PROJECT=antirion-bench
BUNDLE="${BUNDLE:-$HERE/.build/ab-bundle.tgz}"
BENCH_SRC="${BENCH_SRC:-$REPO}"
REPORT_BIN="${REPORT_BIN:-${BUILD_TARGET_ROOT:-$REPO/target/bench-aws}/native/debug/antirion-bench}"
SSM_AMI=/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id
VOLUME_GB=16
POLL_S=60
QUOTA_CODE=L-1216C47A
LOGIN_SESSION_MIN="${AB_LOGIN_SESSION_MIN:-600}"
WALL_MIN="${WALL_MIN:-}"
COST_CAP="${COST_CAP:-15}"
CELLS=compare
SHARDS=1
SEL_LANES=A
WITH_GATES=0
DEADLINE_MIN=""
# The comparison table (id seconds set), read once.
TABLE_FILE=""
# The comparison's cells by shard lane (lane cell expected-seconds), from assign_shards.
SHARDS_FILE=""
export AWS_PAGER=""

TOPOLOGY=t3
# Set by config_<topology>:
#   MACHINES: name type role lane hosts.env-variable released-after
#             (released-after: the lane status line after which the machine is no longer needed)
#   PHASES:   lane phase expected-minutes budget-minutes
MACHINES=""; PHASES=""; LANES=""; SETUP_MIN=15; FETCH_MIN=5; DEADMAN_MARGIN_MIN=25

config_t3() {
  MACHINES="gen-a t3.xlarge generator A GEN_A lane-A
mock-a t3.xlarge mock A MOCK_A lane-A
gw-xl t3.xlarge gateway A GWX lane-A
gw-med t3.medium gateway A GWM lane-A
gen-b t3.xlarge generator B GEN_B lane-B
mock-b t3.xlarge mock B MOCK_B lane-B
gw-b t3.xlarge cluster B GWB lane-B
cl-2 t3.xlarge cluster B CL2 c7-cluster
cl-3 t3.xlarge cluster B CL3 c7-cluster
store t3.medium store B STORE c7-cluster"
  # Expected minutes (from the previous multi-VM run's rounds) and budgets (the phase timeouts in
  # vm/lane.sh), per phase.
  PHASES="A selftest-a 2 10
A compare 90 110
A c20-xlarge 12 30
A c20-medium 12 30
B selftest-b 2 10
B c7-cluster 6 25
B gates 70 100"
  LANES="A B"; SETUP_MIN=15; FETCH_MIN=5; DEADMAN_MARGIN_MIN=25
}

# Expected minutes on c7i: the t3 run's gates took 60 min at 3 reps (about 95 min at 5); C7 3 min;
# C15's ladder (10,000 RPS steps, each a gateway and a direct run) ~15 min. The comparison's cells
# come with their own estimates (comparison.tsv, see gen-cells.py).
GATES5_FULL_MIN=95

# The comparison's cells: id, expected seconds, set (headline|compare), in run order. From the
# bundle when there is one (what the machines will run), else from this checkout's matrix.
comparison_table() {
  if [ -n "$TABLE_FILE" ] && [ -s "$TABLE_FILE" ]; then cat "$TABLE_FILE"; return 0; fi
  if [ -f "$BUNDLE" ] && tar -tzf "$BUNDLE" | grep -qx ab/cells/comparison.tsv; then
    tar -xzOf "$BUNDLE" ab/cells/comparison.tsv
  else
    command -v uv >/dev/null || die "uv is needed to read the comparison's cells from the matrix (or build the bundle first)"
    uv run --quiet --with pyyaml python "$HERE/gen-cells.py" --table "$BENCH_SRC/bench/matrix/cells.yaml"
  fi
}

# Whether lane $1 (A, B or C) is among the selected lanes.
lane_selected() { case ",$SEL_LANES," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }

# The comparison cells --cells selects, in run order: compare or all (every one), headline (H1), or
# a comma-separated list of ids (C1 stands for cmp-C1).
selected_table() {
  local t
  t=$(comparison_table)
  case "$CELLS" in
    all|compare) echo "$t" | awk 'NF >= 2' ;;
    headline) echo "$t" | awk 'NF >= 2 && $3 == "headline"' ;;
    *)
      local id missing=""
      for id in $(echo "$CELLS" | tr ',' ' '); do
        echo "$t" | awk -v i="$id" '$1 == i || $1 == "cmp-" i {f = 1} END {exit !f}' || missing="$missing $id"
      done
      [ -z "$missing" ] || die "--cells: no comparison cell$missing (there are: $(echo "$t" | awk '{printf "%s ", $1}'))"
      echo "$t" | awk -v ids=",$CELLS," 'NF >= 2 && (index(ids, "," $1 ",") || ($1 ~ /^cmp-/ && index(ids, "," substr($1, 5) ",")))'
      ;;
  esac
}

shard_lane() { if [ "$1" = 1 ]; then echo A; else echo "A$1"; fi; }
shard_lanes() { local k; for k in $(seq 1 "$SHARDS"); do shard_lane "$k"; done; }
is_shard() { case "$1" in A|A[0-9]*) return 0 ;; *) return 1 ;; esac; }

# Splits the comparison's cells over the shards: longest first, each to the least loaded shard
# (ties to the lowest); each shard keeps the cells in run order. Writes SHARDS_FILE.
assign_shards() {
  local table n
  table=$(selected_table)
  n=$(echo "$table" | grep -c .)
  [ "$n" -gt 0 ] || die "no comparison cell for --cells $CELLS"
  [ "$SHARDS" -le "$n" ] || die "--shards $SHARDS: more shards than the $n comparison cell(s) of --cells $CELLS"
  [ -n "$SHARDS_FILE" ] || SHARDS_FILE=$(mktemp)
  echo "$table" | awk '{print NR, $1, $2}' | sort -k3,3nr -k1,1n \
    | awk -v n="$SHARDS" '{ best = 1; for (k = 2; k <= n; k++) if (load[k] < load[best]) best = k;
                            load[best] += $3; print $1, (best == 1 ? "A" : "A" best), $2, $3 }' \
    | sort -k1,1n | awk '{print $2, $3, $4}' > "$SHARDS_FILE"
}
shard_seconds() { awk -v l="$1" '$1==l {s+=$3} END {print s+0}' "$SHARDS_FILE"; }
shard_minutes() { echo $(( ($(shard_seconds "$1") + 59) / 60 )); }
shard_cells() { awk -v l="$1" '$1==l {printf "%s%s", (n++ ? " " : ""), $2}' "$SHARDS_FILE"; }
# A shard's compare phase timeout: half again its estimate, plus 10 min.
shard_budget() { echo $(( $(shard_minutes "$1") * 3 / 2 + 10 )); }

config_ref() {
  local k lane sfx gw_var comp gates
  MACHINES=""; PHASES=""; LANES=""
  # Three machines a shard set up in a few minutes (thirteen took under four in a reference run).
  SETUP_MIN=10; FETCH_MIN=3; DEADMAN_MARGIN_MIN=30
  if lane_selected B || lane_selected C; then SETUP_MIN=15; FETCH_MIN=5; fi
  if lane_selected A; then
    [ -n "$SHARDS_FILE" ] && [ -f "$SHARDS_FILE" ] || die "internal: the shards are not assigned"
    for k in $(seq 1 "$SHARDS"); do
      lane=$(shard_lane "$k")
      if [ "$k" = 1 ]; then sfx=""; gw_var=GWX; else sfx="$k"; gw_var="GW_$lane"; fi
      MACHINES="$MACHINES
gen-a$sfx c7i.4xlarge generator $lane GEN_$lane lane-$lane
mock-a$sfx c7i.4xlarge mock $lane MOCK_$lane lane-$lane
gw-a$sfx c7i.xlarge gateway $lane $gw_var lane-$lane"
      PHASES="$PHASES
$lane selftest-$(lc "$lane") 2 10
$lane compare $(shard_minutes "$lane") $(shard_budget "$lane")"
      LANES="$LANES $lane"
    done
    if [ "$WITH_GATES" = 1 ]; then
      # The release cells fill lane A up to the planned wall time (by default, all of them).
      comp=$(shard_minutes A)
      [ -n "$WALL_MIN" ] || WALL_MIN=$(( SETUP_MIN + 2 + comp + GATES5_FULL_MIN + FETCH_MIN ))
      gates=$(( WALL_MIN - SETUP_MIN - FETCH_MIN - 2 - comp ))
      [ "$gates" -lt 0 ] && gates=0
      PHASES="$PHASES
A gates $gates $gates"
    fi
  fi
  if lane_selected B; then
    MACHINES="$MACHINES
gen-b c7i.4xlarge generator B GEN_B lane-B
mock-b c7i.4xlarge mock B MOCK_B lane-B
cl-1 c7i.xlarge node B GWB lane-B
cl-2 c7i.xlarge node B CL2 lane-B
cl-3 c7i.xlarge node B CL3 lane-B
pg c7i.2xlarge postgres B PG lane-B
valkey c7i.2xlarge valkey B VALKEY lane-B"
    PHASES="$PHASES
B selftest-b 2 10
B c7-cluster 8 25"
    LANES="$LANES B"
  fi
  if lane_selected C; then
    MACHINES="$MACHINES
gen-c c7i.8xlarge generator C GEN_C lane-C
mock-c c7i.8xlarge mock C MOCK_C lane-C
gw-c c7i.4xlarge node C GW16 lane-C"
    PHASES="$PHASES
C selftest-c 2 10
C c15 15 35"
    LANES="$LANES C"
  fi
  MACHINES=$(echo "$MACHINES" | grep .); PHASES=$(echo "$PHASES" | grep .)
  LANES="${LANES# }"
}

configure() {
  case ",$SEL_LANES," in
    *[!ABC,]*|,,) die "--lanes takes a comma-separated list of A, B and C" ;;
  esac
  [ "$WITH_GATES" = 0 ] || lane_selected A || die "--with-gates runs on lane A: --lanes A,..."
  case "$SHARDS" in
    ''|*[!0-9]*|0) die "--shards takes a whole number of at least 1" ;;
  esac
  case "$TOPOLOGY" in
    t3) [ "$CELLS" = compare ] && [ "$SHARDS" = 1 ] && [ "$SEL_LANES" = A ] && [ "$WITH_GATES" = 0 ] && [ -z "$DEADLINE_MIN" ] \
          || die "--cells, --shards, --lanes, --with-gates and --deadline run on the reference topology: --topology ref"
        config_t3 ;;
    ref) config_ref ;;
    *) die "unknown topology $TOPOLOGY (t3 or ref)" ;;
  esac
}

price_of() { # on-demand Linux, us-east-1, USD per hour (other regions differ by a few per cent)
  case "$1" in
    t3.xlarge) echo 0.1664 ;;
    t3.medium) echo 0.0416 ;;
    c7i.xlarge) echo 0.1785 ;;
    c7i.2xlarge) echo 0.357 ;;
    c7i.4xlarge) echo 0.714 ;;
    c7i.8xlarge) echo 1.428 ;;
    *) echo "no price for $1" >&2; exit 1 ;;
  esac
}
vcpus_of() {
  case "$1" in
    t3.medium) echo 2 ;; t3.xlarge|c7i.xlarge) echo 4 ;; c7i.2xlarge) echo 8 ;; c7i.4xlarge) echo 16 ;; c7i.8xlarge) echo 32 ;;
  esac
}
baseline_of() { case "$1" in t3.xlarge) echo 0.40 ;; t3.medium) echo 0.20 ;; *) echo 1 ;; esac; }
SURPLUS_PER_VCPU_H=0.05
PUBLIC_IP_PER_H=0.005
GP3_PER_GB_MONTH=0.08

lc() { echo "$1" | tr '[:upper:]' '[:lower:]'; }
lane_minutes() { echo "$PHASES" | awk -v l="$1" -v c="$(( $2 + 1 ))" '$1==l {s+=$c} END {print s+0}'; }
# t3: one dead-man time for every machine (the longest lane's budgets plus a margin, at most 300 min).
deadman_minutes() {
  local m=0 l x
  for l in $LANES; do x=$(lane_minutes "$l" 3); [ "$x" -gt "$m" ] && m=$x; done
  m=$(( SETUP_MIN + 10 + m + DEADMAN_MARGIN_MIN ))
  [ "$m" -gt 300 ] && m=300
  echo "$m"
}
# Minutes a lane is expected to keep its machines: setup, its phases, fetch.
lane_planned() { echo $(( SETUP_MIN + $(lane_minutes "$1" 2) + FETCH_MIN )); }
# The dead-man time of a lane's machines, in minutes after launch.
lane_deadman() {
  if [ "$TOPOLOGY" = ref ]; then echo $(( $(lane_planned "$1") + DEADMAN_MARGIN_MIN )); else deadman_minutes; fi
}
wall_planned() { local m=0 l x; for l in $LANES; do x=$(lane_planned "$l"); [ "$x" -gt "$m" ] && m=$x; done; echo "$m"; }
peak_vcpus() { echo "$MACHINES" | while read -r _ type _; do vcpus_of "$type"; done | awk '{s+=$1} END {print s+0}'; }

log() { echo "[$(date -u +%H:%M:%S)] $*"; }
die() { echo "run.sh: $*" >&2; exit 1; }
aws_() { aws --region "$REGION" "$@"; }

# ---------------------------------------------------------------------------------------------
# Credentials

# The newest `aws login` cache file and the time its session started (the identity token's iat: the
# CLI refreshes the 15-minute access key without reissuing it).
# shellcheck disable=SC2012  # newest first: ls -t
login_cache() { ls -t "${AWS_LOGIN_CACHE_DIRECTORY:-$HOME/.aws/login/cache}"/*.json 2>/dev/null | head -1; }
login_started() {
  local f
  f=$(login_cache); [ -n "$f" ] || return 1
  python3 - "$f" 2>/dev/null <<'PY'
import base64, json, sys
t = json.load(open(sys.argv[1])).get("idToken", "").split(".")
p = t[1]
print(int(json.loads(base64.urlsafe_b64decode(p + "=" * (-len(p) % 4)))["iat"]))
PY
}
cred_type() { aws configure list 2>/dev/null | awk -F' : ' '$1 ~ /^ *access_key/ {gsub(/ /, "", $3); print $3}'; }
hm() { date -u -r "$1" +%Y-%m-%dT%H:%MZ; }

# Prints the credentials' state; CRED_LEFT_MIN: minutes until they can no longer be renewed by the
# CLI ("" when they do not expire or that cannot be told).
CRED_LEFT_MIN=""
cred_status() {
  local type exp exp_s start end now arn
  CRED_LEFT_MIN=""
  now=$(date +%s)
  if ! arn=$(aws sts get-caller-identity --query Arn --output text 2>&1); then
    echo "credentials: NOT VALID ($(echo "$arn" | tail -1))"
    return 1
  fi
  type=$(cred_type)
  exp=$(aws configure export-credentials --format process 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin).get("Expiration",""))' 2>/dev/null || true)
  exp_s=""
  [ -n "$exp" ] && exp_s=$(python3 -c 'import sys,datetime; print(int(datetime.datetime.fromisoformat(sys.argv[1].replace("Z","+00:00")).timestamp()))' "$exp" 2>/dev/null || true)
  echo "credentials: valid, $arn, type ${type:-unknown}"
  if [ "$type" = login ]; then
    [ -n "$exp_s" ] && echo "  access key: renewed by the CLI every 15 min; the current one until $(hm "$exp_s")"
    if start=$(login_started) && [ -n "$start" ]; then
      end=$(( start + LOGIN_SESSION_MIN * 60 ))
      CRED_LEFT_MIN=$(( (end - now) / 60 ))
      echo "  login session: started $(hm "$start"); assumed lifetime ${LOGIN_SESSION_MIN} min (AB_LOGIN_SESSION_MIN) -> ends ~$(hm "$end"), ${CRED_LEFT_MIN} min left"
    else
      CRED_LEFT_MIN=0
      echo "  login session: its start cannot be read from $(login_cache || echo 'the login cache'); treated as ending now"
    fi
  elif [ -n "$exp_s" ]; then
    CRED_LEFT_MIN=$(( (exp_s - now) / 60 ))
    echo "  expire at $(hm "$exp_s"), ${CRED_LEFT_MIN} min left"
  else
    echo "  no expiry reported (long-term keys)"
  fi
}

# Stops unless the credentials last `need` minutes.
cred_require() { # minutes what
  local need="$1" what="$2"
  if ! cred_status; then
    echo >&2
    echo "run.sh: the AWS credentials are not valid. Sign in, then run the same command again:" >&2
    echo "" >&2
    echo "    aws login" >&2
    exit 1
  fi
  if [ -n "$CRED_LEFT_MIN" ] && [ "$CRED_LEFT_MIN" -lt "$need" ]; then
    if [ "${AB_IGNORE_CRED_EXPIRY:-0}" = 1 ]; then
      log "WARNING: the credentials have ${CRED_LEFT_MIN} min left, less than the ${need} min $what needs (AB_IGNORE_CRED_EXPIRY=1)"
      return 0
    fi
    echo >&2
    echo "run.sh: the AWS login session has ~${CRED_LEFT_MIN} min left, less than the ${need} min $what needs." >&2
    echo "When it ends, the release and teardown calls fail (the phases go on and the dead-man switch still" >&2
    echo "ends the machines). Start a fresh session, then run the same command again:" >&2
    echo "" >&2
    echo "    aws login" >&2
    exit 1
  fi
}

# During a run: the login session's minutes left, from the cache only (no AWS call); "" when unknown.
cred_left_quiet() {
  local start
  [ "$(cred_type)" = login ] || return 0
  start=$(login_started) || return 0
  echo $(( (start + LOGIN_SESSION_MIN * 60 - $(date +%s)) / 60 ))
}

# The On-Demand Standard vCPU quota left (quota minus the vCPUs of running instances).
QUOTA=""; QUOTA_USED=""
quota_status() {
  QUOTA=$(aws_ service-quotas get-service-quota --service-code ec2 --quota-code "$QUOTA_CODE" --query 'Quota.Value' --output text 2>/dev/null | cut -d. -f1) || QUOTA=""
  QUOTA_USED=$(aws_ ec2 describe-instances --filters Name=instance-state-name,Values=pending,running \
      --query 'Reservations[].Instances[].[InstanceType,CpuOptions.CoreCount,CpuOptions.ThreadsPerCore]' --output text 2>/dev/null \
    | awk '$1 !~ /^(f|g|inf|p|vt|x|u-|trn|dl|mac)/ {s += $2 * $3} END {print s+0}') || QUOTA_USED=""
}
quota_require() {
  local peak
  peak=$(peak_vcpus)
  quota_status
  [ -n "$QUOTA" ] && [ -n "$QUOTA_USED" ] || die "cannot read the vCPU quota $QUOTA_CODE (service-quotas:GetServiceQuota)"
  log "vCPU quota $QUOTA_CODE: $QUOTA, in use $QUOTA_USED, this run's peak $peak"
  [ $(( QUOTA - QUOTA_USED )) -ge "$peak" ] || die "the vCPU quota $QUOTA_CODE leaves $(( QUOTA - QUOTA_USED )) vCPUs, this run needs $peak at once"
}

# ---------------------------------------------------------------------------------------------
# Plan and cost

# Minutes a machine is up, expected: setup, then its lane's phases until it is released.
machine_minutes() { # lane released-after column(2 expected, 3 budget)
  local lane="$1" rel="$2" col="$3" total
  if [ "$rel" = "lane-$lane" ]; then
    total=$(lane_minutes "$lane" "$col")
  else
    total=$(echo "$PHASES" | awk -v l="$lane" -v c="$(( col + 1 ))" -v r="$rel" '$1==l {s+=$c} $1==l && $2==r {print s; exit}')
  fi
  echo $(( SETUP_MIN + total + FETCH_MIN ))
}

# Expected average CPU utilisation of a machine's role while its lane runs (t3 surplus credits).
util_of() {
  case "$1" in generator) echo 0.60 ;; mock) echo 0.40 ;; gateway) echo 0.60 ;; cluster) echo 0.55 ;; store) echo 0.25 ;; *) echo 0.5 ;; esac
}

# One line per machine: name type role vCPUs hours on-demand surplus total, for `minutes_of` (a
# function given lane and released-after) and a utilisation override (empty: per role).
cost_lines() { # minutes-function util-override
  echo "$MACHINES" | while read -r name type role lane _ rel; do
    awk -v n="$name" -v t="$type" -v r="$role" -v m="$($1 "$lane" "$rel")" -v p="$(price_of "$type")" \
        -v v="$(vcpus_of "$type")" -v b="$(baseline_of "$type")" -v u="${2:-$(util_of "$role")}" -v s="$SURPLUS_PER_VCPU_H" \
        -v ip="$PUBLIC_IP_PER_H" -v gb="$VOLUME_GB" -v gbm="$GP3_PER_GB_MONTH" -v setup="$SETUP_MIN" 'BEGIN {
      h = m / 60
      busy = (m - setup) / 60; if (busy < 0) busy = 0
      sur = (u > b ? (u - b) * v * busy * s : 0)
      od = h * p + h * ip + gb * gbm * h / 730
      printf "  %-7s %-11s %-9s %3d %5.2f %8.3f %8.3f %8.3f\n", n, t, r, v, h, od, sur, od + sur }'
  done
}
# cost_lines for one lane's machines only.
cost_lane_lines() { # minutes-function lane
  local MACHINES
  MACHINES=$(echo "$ALL_MACHINES" | awk -v l="$2" '$4==l')
  cost_lines "$1"
}
cost_total() { awk '{t+=$8} END {printf "%.2f", t}'; }
mm_expected() { machine_minutes "$1" "$2" 2; }
mm_budget() { machine_minutes "$1" "$2" 3; }
mm_deadman() { lane_deadman "$1"; }

plan_t3() {
  local dm col label
  dm=$(deadman_minutes)
  echo "Run plan: AWS $REGION, default VPC, one AZ, t3 instances in unlimited-credit mode"
  echo "(burstable machines: these results reproduce a published setup and are never reference results)"
  echo
  echo "Lane A (generator gen-a, mock mock-a):"
  echo "$PHASES" | awk '$1=="A" {printf "  %-12s expected %3d min, budget %3d min\n", $2, $3, $4}'
  echo "  (a) compare: cells C1 C2 C4 C5 C8 C11 C13 C16 C17 C21.401 C21.404 C32.c64 at P-min, 3 reps, every target interleaved on gw-xl"
  echo "  (b) C20 (S1 at 5,000 RPS, L1500 mock), every target, on gw-xl then gw-med"
  echo "Lane B (generator gen-b, mock mock-b):"
  echo "$PHASES" | awk '$1=="B" {printf "  %-12s expected %3d min, budget %3d min\n", $2, $3, $4}'
  echo "  (d) C7 on gw-b, cl-2, cl-3 (P-cluster) with PostgreSQL and Valkey on store; cl-2, cl-3 and store are released after it"
  echo "  (c) the release cells (27, gates) on gw-b"
  echo
  echo "Wall: setup ~${SETUP_MIN} min + longest lane ~$(lane_minutes A 2) min + fetch ~${FETCH_MIN} min = ~$(( SETUP_MIN + $(lane_minutes A 2) + FETCH_MIN )) min expected"
  echo "Dead-man switch: every instance shuts down (and terminates) $dm min after boot"
  echo
  for col in 2 3; do
    if [ "$col" = 2 ]; then label="expected"; else label="budget (every phase at its timeout, 100 % CPU)"; fi
    echo "Cost, $label:"
    printf "  %-7s %-11s %-9s %3s %5s %8s %8s %8s\n" machine type role cpu hours on-demand surplus total
    if [ "$col" = 2 ]; then lines=$(cost_lines mm_expected); else lines=$(cost_lines mm_budget 1.0); fi
    echo "$lines"
    echo "$lines" | awk '{od+=$6; s+=$7; t+=$8} END {printf "  %-39s %8.2f %8.2f %8.2f USD\n", "total", od, s, t}'
    echo
  done
  echo "Prices: t3.xlarge \$$(price_of t3.xlarge)/h, t3.medium \$$(price_of t3.medium)/h (on-demand, Linux, us-east-1),"
  echo "T3 unlimited surplus \$$SURPLUS_PER_VCPU_H per vCPU-hour above the baseline (40 % per vCPU on xlarge, 20 % on medium),"
  echo "public IPv4 \$$PUBLIC_IP_PER_H/h, gp3 ${VOLUME_GB} GB at \$$GP3_PER_GB_MONTH/GB-month. Expected surplus assumes average CPU"
  echo "generator 60 %, mock 40 %, gateway 60 %, cluster 55 %, store 25 %; the budget case assumes 100 % on every machine."
  echo "Ceiling (all machines until the dead-man switch, 100 % CPU): \$$(cost_lines mm_deadman 1.0 | cost_total)"
}

# The expected cost of the ref run at another planned wall time.
cost_at_wall() { local WALL_MIN="$1"; config_ref; cost_lines mm_expected | cost_total; }

plan_ref() {
  local l wall exp ceil peak gates_min full_wall full_cost
  wall=$(wall_planned)
  echo "Run plan: AWS $REGION, default VPC, one AZ, one cluster placement group, c7i on-demand"
  echo "(topology ref; lanes $SEL_LANES; cells $CELLS; shards $SHARDS; release gates $([ "$WITH_GATES" = 1 ] && echo on || echo off)${DEADLINE_MIN:+; deadline $DEADLINE_MIN min})"
  echo
  echo "Machines (all launched together; each lane's are terminated as soon as the lane is done):"
  printf "  %-7s %-11s %-9s %4s %-5s %s\n" machine type role vCPU lane "hosts.env"
  echo "$MACHINES" | while read -r name type role lane var _; do
    printf "  %-7s %-11s %-9s %4s %-5s %s\n" "$name" "$type" "$role" "$(vcpus_of "$type")" "$lane" "$var"
  done
  echo
  if lane_selected A; then
  echo "The comparison: the targets of bench/targets (each with a target.yaml) and antirion, interleaved per cell"
  echo "and repetition on one shard's machines. H1: S1 at 5,000 RPS open loop, P-min, mock L0, 10 s warm-up, 30 s"
  echo "measurement, 5 reps; compare cells (cmp-*): the matrix's cells at P-min, 5 s warm-up, 15 s measurement,"
  echo "3 reps (ladders 1 rep, 7 s steps). Selected: $(awk '{printf "%s ", $2}' "$SHARDS_FILE")"
  echo "$SHARDS shard(s), balanced by expected duration (cells are compared within a shard, never across):"
  for l in $(shard_lanes); do
    echo "Lane $l, ref-4 (gw-$(lc "$l") c7i.xlarge; gen-$(lc "$l"), mock-$(lc "$l") c7i.4xlarge; generator workers 16, mock workers 16):"
    echo "$PHASES" | awk -v l="$l" '$1==l {printf "  %-12s expected %3d min, budget %3d min\n", $2, $3, $4}'
    awk -v l="$l" '$1==l {printf "    %-14s %5.1f min\n", $2, $3 / 60}' "$SHARDS_FILE"
  done
  fi
  if [ "$WITH_GATES" = 1 ]; then
    gates_min=$(echo "$PHASES" | awk '$1=="A" && $2=="gates" {print $3}')
    echo "  gates (lane A, --with-gates): Antirion's 27 release cells at 5 reps (ladders 1 rep), cut at ${gates_min} min, when"
    echo "         lane A reaches the planned wall time; all of them need ~${GATES5_FULL_MIN} min (WALL_MIN=$(( WALL_MIN - gates_min + GATES5_FULL_MIN )) lets them finish)"
  fi
  if lane_selected B; then
    echo "Lane B, ref-4 with P-cluster (cl-1..3 c7i.xlarge; pg, valkey c7i.2xlarge; gen-b, mock-b c7i.4xlarge):"
    echo "$PHASES" | awk '$1=="B" {printf "  %-12s expected %3d min, budget %3d min\n", $2, $3, $4}'
    echo "  C7: S1 at 10,000 RPS on three gateway nodes, PostgreSQL on pg, Valkey on valkey"
  fi
  if lane_selected C; then
    echo "Lane C, ref-16 (gw-c c7i.4xlarge; gen-c, mock-c c7i.8xlarge; generator workers 32, mock workers 32):"
    echo "$PHASES" | awk '$1=="C" {printf "  %-12s expected %3d min, budget %3d min\n", $2, $3, $4}'
    echo "  C15: S1 saturation ladder (10,000 RPS steps, 2,500 fine) on P-std; its gate reads C12 from lane A's gates (--with-gates)"
  fi
  echo
  echo "Every machine: the reference system settings (ulimit -n 1048576, somaxconn 65535, ip_local_port_range"
  echo "1024 65535, tcp_tw_reuse 1, governor performance where exposed), the same on every machine, read back"
  echo "into the inventory; the harness's RTT to the mock (connect p50/p99) from each phase's inventory into rtt.txt."
  echo
  echo "Wall (setup ${SETUP_MIN} + phases + fetch ${FETCH_MIN}), dead-man (planned + ${DEADMAN_MARGIN_MIN} min after launch):"
  for l in $LANES; do
    printf "  lane %s: planned %3d min, dead-man %3d min\n" "$l" "$(lane_planned "$l")" "$(lane_deadman "$l")"
  done
  echo "  run: ~${wall} min wall; the last dead-man switch fires $(( wall + DEADMAN_MARGIN_MIN )) min after launch"
  echo
  peak=$(peak_vcpus)
  echo "Peak concurrent vCPUs: $peak (all lanes at once), against quota $QUOTA_CODE (Running On-Demand Standard instances)"
  echo
  echo "Cost, expected (each machine for its lane's planned time):"
  printf "  %-7s %-11s %-9s %3s %5s %8s %8s %8s\n" machine type role cpu hours on-demand surplus total
  exp=$(cost_lines mm_expected)
  echo "$exp"
  echo "  total $(echo "$exp" | cost_total) USD"
  ceil=$(cost_lines mm_deadman | cost_total)
  echo "Ceiling (every machine until its dead-man switch): $ceil USD"
  echo "By lane (expected / ceiling, USD):"
  ALL_MACHINES="$MACHINES"
  for l in $LANES; do
    printf "  lane %-3s %6s / %6s\n" "$l" "$(cost_lane_lines mm_expected "$l" | cost_total)" "$(cost_lane_lines mm_deadman "$l" | cost_total)"
  done
  if [ "$WITH_GATES" = 1 ] && [ "$gates_min" -lt "$GATES5_FULL_MIN" ]; then
    full_wall=$(( WALL_MIN - gates_min + GATES5_FULL_MIN ))
    full_cost=$(cost_at_wall "$full_wall")
    echo "With every release cell at 5 reps (WALL_MIN=$full_wall): expected $full_cost USD"
  fi
  echo "Prices (on-demand, Linux, us-east-1): c7i.xlarge \$0.1785/h, c7i.2xlarge \$0.357, c7i.4xlarge \$0.714, c7i.8xlarge \$1.428;"
  echo "public IPv4 \$$PUBLIC_IP_PER_H/h per machine, gp3 ${VOLUME_GB} GB at \$$GP3_PER_GB_MONTH/GB-month."
  [ "$REGION" = us-east-1 ] || echo "The run is in $REGION: its prices differ from these by a few per cent."
  if awk -v e="$(echo "$exp" | cost_total)" -v c="$COST_CAP" 'BEGIN {exit !(e <= c)}'; then
    echo "Expected cost within the cap of \$$COST_CAP (COST_CAP)."
  else
    echo "Expected cost ABOVE the cap of \$$COST_CAP (COST_CAP): run.sh refuses to start."
  fi
}

plan() {
  if [ "$TOPOLOGY" = ref ]; then plan_ref; else plan_t3; fi
}

# The read-only checks --plan adds (they need valid credentials; nothing is created).
plan_checks() {
  local peak
  echo
  echo "Checks (read-only):"
  if [ -n "$ACCOUNT" ]; then echo "  account: must be $ACCOUNT (AB_ACCOUNT)"; fi
  local out
  out=$(mktemp)
  if cred_status > "$out" 2>&1; then
    sed 's/^/  /' "$out"
    if [ -n "$CRED_LEFT_MIN" ]; then
      local need=$(( $(wall_planned) + 30 ))
      if [ "$CRED_LEFT_MIN" -ge "$need" ]; then echo "  enough for this run (needs $need min: planned wall + 30)"
      else echo "  NOT enough for this run (needs $need min: planned wall + 30): run 'aws login' first"; fi
    fi
  else
    sed 's/^/  /' "$out"
    echo "  run 'aws login' first"
  fi
  rm -f "$out"
  peak=$(peak_vcpus)
  quota_status
  if [ -n "$QUOTA" ] && [ -n "$QUOTA_USED" ]; then
    echo "  vCPU quota $QUOTA_CODE: $QUOTA, in use $QUOTA_USED, peak $peak: $([ $(( QUOTA - QUOTA_USED )) -ge "$peak" ] && echo enough || echo 'NOT ENOUGH (run.sh refuses to start)')"
  else
    echo "  vCPU quota $QUOTA_CODE: not readable (peak $peak)"
  fi
}

# ---------------------------------------------------------------------------------------------
# State

STATE=""
RUN_ID=""
init_state() {
  STATE="$STATE_ROOT/runs/$RUN_ID"
  mkdir -p "$STATE"
  chmod 700 "$STATE"
  touch "$STATE/state"
}
set_state() { # key value
  grep -v "^$1=" "$STATE/state" > "$STATE/state.tmp" || true
  echo "$1=$2" >> "$STATE/state.tmp"
  mv "$STATE/state.tmp" "$STATE/state"
}
get_state() { sed -n "s/^$1=//p" "$STATE/state" | tail -1; }
machine_field() { # name field(id|priv|pub|type|role|lane|var|rel|t0)
  awk -v n="$1" -v f="$2" '$1==n {
    if (f=="type") print $2; else if (f=="role") print $3; else if (f=="lane") print $4; else if (f=="var") print $5;
    else if (f=="rel") print $6; else if (f=="id") print $7; else if (f=="priv") print $8; else if (f=="pub") print $9;
    else if (f=="t0") print $10 }' "$STATE/machines.tsv"
}
lane_t0() { awk -v l="$1" '$4==l {print $10; exit}' "$STATE/machines.tsv"; }
# A lane's dead-man minutes and the run's last dead-man time (runs started before the ref topology
# recorded only DEADMAN_MIN).
lane_dm_state() { local d; d=$(get_state "DEADMAN_MIN_$1"); echo "${d:-$(get_state DEADMAN_MIN)}"; }
deadman_at() {
  local d; d=$(get_state DEADMAN_AT)
  [ -n "$d" ] || d=$(( $(awk 'NR==1 {print $10}' "$STATE/machines.tsv") + $(get_state DEADMAN_MIN) * 60 ))
  echo "$d"
}

KEY_FILE=""
SSH_OPTS=()
ssh_setup() {
  # The run's own key, created with it and deleted at teardown (the state directory is gitignored).
  KEY_FILE="$STATE/key.pem"
  SSH_OPTS=(-i "$KEY_FILE" -o StrictHostKeyChecking=no -o UserKnownHostsFile="$STATE/known_hosts" -o ConnectTimeout=10
            -o ServerAliveInterval=15 -o ServerAliveCountMax=4 -o BatchMode=yes -o LogLevel=ERROR)
}
# ssh, once more after allowing this machine's current address when the first attempt cannot connect.
# The remote commands are built here on purpose (values expand on this side).
# shellcheck disable=SC2029
on() {
  local host="$1" rc=0; shift
  ssh "${SSH_OPTS[@]}" "ubuntu@$host" "$@" || rc=$?
  if [ "$rc" = 255 ] && [ -n "$STATE" ] && [ -n "$(get_state SG)" ]; then
    ensure_ingress
    rc=0; ssh "${SSH_OPTS[@]}" "ubuntu@$host" "$@" || rc=$?
  fi
  return "$rc"
}
scp_to() { # host source destination
  scp "${SSH_OPTS[@]}" -q "$2" "ubuntu@$1:$3" || { ensure_ingress; scp "${SSH_OPTS[@]}" -q "$2" "ubuntu@$1:$3"; }
}

# ---------------------------------------------------------------------------------------------
# Creation

tags() { # resource-type [name role lane]
  local t="{Key=Project,Value=$PROJECT},{Key=RunId,Value=$RUN_ID}"
  if [ -n "${2:-}" ]; then t="$t,{Key=Name,Value=ab-$RUN_ID-$2},{Key=Role,Value=$3},{Key=Lane,Value=$4}"; fi
  echo "ResourceType=$1,Tags=[$t]"
}

create_network() { # instance types needed
  local myip vpc az subnet sg types n cand want
  types=$(echo "$1" | tr ' ' '\n' | grep . | sort -u | paste -sd, -)
  want=$(echo "$types" | tr ',' '\n' | wc -l | tr -d ' ')
  myip=$(curl -fsS https://checkip.amazonaws.com | tr -d '[:space:]')
  [ -n "$myip" ] || die "cannot find this machine's public IP"
  vpc=$(aws_ ec2 describe-vpcs --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text)
  [ "$vpc" != None ] || die "no default VPC in $REGION"
  az=""
  for cand in $(aws_ ec2 describe-availability-zones --filters Name=state,Values=available Name=zone-type,Values=availability-zone \
                  --query 'sort(AvailabilityZones[].ZoneName)' --output text); do
    n=$(aws_ ec2 describe-instance-type-offerings --location-type availability-zone \
          --filters Name=location,Values="$cand" Name=instance-type,Values="$types" \
          --query 'length(InstanceTypeOfferings)' --output text)
    if [ "$n" = "$want" ]; then az=$cand; break; fi
  done
  [ -n "$az" ] || die "no AZ offers all of $types"
  subnet=$(aws_ ec2 describe-subnets --filters Name=vpc-id,Values="$vpc" Name=availability-zone,Values="$az" Name=default-for-az,Values=true \
             --query 'Subnets[0].SubnetId' --output text)
  [ "$subnet" != None ] || die "no default subnet in $az"
  sg=$(aws_ ec2 create-security-group --group-name "ab-$RUN_ID" --description "antirion-bench $RUN_ID" --vpc-id "$vpc" \
         --tag-specifications "$(tags security-group)" --query GroupId --output text)
  set_state SG "$sg"
  aws_ ec2 authorize-security-group-ingress --group-id "$sg" \
    --ip-permissions "IpProtocol=-1,UserIdGroupPairs=[{GroupId=$sg,Description=members}]" \
                     "IpProtocol=tcp,FromPort=22,ToPort=22,IpRanges=[{CidrIp=$myip/32,Description=orchestrator}]" >/dev/null
  set_state VPC "$vpc"; set_state AZ "$az"; set_state SUBNET "$subnet"; set_state MY_IP "$myip"
  echo "$myip" >> "$STATE/ingress"
  log "network: $vpc $az $subnet, security group $sg (members, ssh from $myip)"
  if [ "$TOPOLOGY" = ref ]; then
    aws_ ec2 create-placement-group --group-name "ab-$RUN_ID" --strategy cluster \
      --tag-specifications "$(tags placement-group)" >/dev/null
    set_state PLACEMENT_GROUP "ab-$RUN_ID"
    log "cluster placement group ab-$RUN_ID"
  fi
}

create_key() {
  ssh_setup
  ( umask 077; aws_ ec2 create-key-pair --key-name "ab-$RUN_ID" --key-type ed25519 \
      --tag-specifications "$(tags key-pair)" --query KeyMaterial --output text > "$KEY_FILE" )
  chmod 600 "$KEY_FILE"
  set_state KEY_NAME "ab-$RUN_ID"
  log "key pair ab-$RUN_ID, private key $KEY_FILE"
}

user_data() { # dead-man-minutes -> file
  local f="$STATE/user-data-$1.sh"
  cat > "$f" <<EOF
#!/bin/bash
# Dead-man switch: with instance-initiated shutdown behaviour "terminate", this ends the instance.
shutdown -h +$1 "antirion-bench $RUN_ID dead-man switch"
echo "$(date -u +%FT%TZ) +$1 min" > /etc/ab-deadman
EOF
  echo "$f"
}

run_instance() { # name type role lane dead-man-minutes -> instance id
  local name="$1" type="$2" role="$3" lane="$4" ud extra=() attempt out rc
  ud=$(user_data "$5")
  if [ "$TOPOLOGY" = ref ]; then
    extra=(--placement "GroupName=$(get_state PLACEMENT_GROUP)")
  else
    extra=(--credit-specification CpuCredits=unlimited)
  fi
  for attempt in 1 2 3 4; do
    rc=0
    out=$(aws_ ec2 run-instances --image-id "$(get_state AMI)" --instance-type "$type" --count 1 \
          --key-name "ab-$RUN_ID" \
          --network-interfaces "DeviceIndex=0,SubnetId=$(get_state SUBNET),Groups=$(get_state SG),AssociatePublicIpAddress=true,DeleteOnTermination=true" \
          "${extra[@]}" \
          --instance-initiated-shutdown-behavior terminate \
          --metadata-options HttpTokens=required,HttpEndpoint=enabled \
          --block-device-mappings "DeviceName=/dev/sda1,Ebs={VolumeSize=$VOLUME_GB,VolumeType=gp3,DeleteOnTermination=true}" \
          --user-data "file://$ud" \
          --tag-specifications "$(tags instance "$name" "$role" "$lane")" "$(tags volume "$name" "$role" "$lane")" \
          --query 'Instances[0].InstanceId' --output text 2>&1) || rc=$?
    if [ "$rc" = 0 ]; then echo "$out"; return 0; fi
    if echo "$out" | grep -q 'InsufficientInstanceCapacity' && [ "$attempt" -lt 4 ]; then
      echo "[$(date -u +%H:%M:%S)] no $type capacity in the placement group yet (attempt $attempt); retrying in 30 s" >&2
      sleep 30; continue
    fi
    echo "$out" >&2; return "$rc"
  done
}

launch() { # machines (lines)
  local ami name type role lane var rel id dm ids last=0
  ami=$(aws_ ssm get-parameter --name "$SSM_AMI" --query Parameter.Value --output text)
  set_state AMI "$ami"
  : > "$STATE/machines.tsv"
  while read -r name type role lane var rel; do
    [ -n "$name" ] || continue
    dm=$(lane_deadman "$lane")
    set_state "DEADMAN_MIN_$lane" "$dm"
    id=$(run_instance "$name" "$type" "$role" "$lane" "$dm") || die "cannot launch $name ($type)"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t-\t-\t%s\t-\n' "$name" "$type" "$role" "$lane" "$var" "$rel" "$id" "$(date +%s)" >> "$STATE/machines.tsv"
    log "launched $name ($type, $role, lane $lane, dead-man +$dm min): $id"
    [ $(( $(date +%s) + dm * 60 )) -gt "$last" ] && last=$(( $(date +%s) + dm * 60 ))
    set_state DEADMAN_AT "$last"
  done <<< "$1"
  set_state DEADMAN_MIN "$(deadman_minutes)"
  ids=$(awk '{print $7}' "$STATE/machines.tsv" | tr '\n' ' ')
  # shellcheck disable=SC2086
  aws_ ec2 wait instance-running --instance-ids $ids
  # shellcheck disable=SC2086
  aws_ ec2 describe-instances --instance-ids $ids \
    --query 'Reservations[].Instances[].[InstanceId,PrivateIpAddress,PublicIpAddress,Placement.GroupName]' --output text > "$STATE/ips.tsv"
  awk 'NR==FNR {p[$1]=$2; q[$1]=$3; next} {OFS="\t"; $8=p[$7]; $9=q[$7]; print}' "$STATE/ips.tsv" "$STATE/machines.tsv" > "$STATE/m.tmp"
  mv "$STATE/m.tmp" "$STATE/machines.tsv"
  { awk '{print $5"="$8}' "$STATE/machines.tsv"; echo "TOPOLOGY=$TOPOLOGY"; echo "REGION=$REGION"; } > "$STATE/hosts.env"
  if [ "$TOPOLOGY" = t3 ]; then
    # shellcheck disable=SC2086
    aws_ ec2 describe-instance-credit-specifications --instance-ids $ids --output json > "$STATE/credit-specifications.json"
  fi
  log "running: $(awk '{printf "%s=%s ", $1, $9}' "$STATE/machines.tsv")"
}

wait_ssh() { # names...
  local n host i
  for n in "$@"; do
    host=$(machine_field "$n" pub)
    for i in $(seq 1 60); do
      if on "$host" true 2>/dev/null; then break; fi
      [ "$i" = 60 ] && die "no ssh to $n ($host)"
      sleep 5
    done
  done
  log "ssh ready on $*"
}

all_names() { awk '{print $1}' "$STATE/machines.tsv"; }

# Whether every machine reports the same system settings (the sysctl and limit lines of its inventory).
settings_record() {
  local f out="$RESULTS/inventory/settings.txt"
  {
    echo "System settings as read back on each machine after setup (vm/setup.sh tune):"
    for f in "$RESULTS"/inventory/*.txt; do
      case "$f" in */machines.txt|*/settings.txt) continue ;; esac
      grep -E '^(settings_of|sysctl |ulimit_n_setup|governor):' "$f" > "$f.settings" 2>/dev/null || true
    done
    local first
    first=$(find "$RESULTS/inventory" -name '*.txt.settings' | sort | head -1)
    if [ -z "$first" ]; then
      echo "  no settings recorded"
    else
      cat "$first" | sed 's/^/  /'
      local diff_n=0
      for f in "$RESULTS"/inventory/*.txt.settings; do
        cmp -s "$first" "$f" || { diff_n=$((diff_n + 1)); echo "  DIFFERENT on $(basename "$f" .txt.settings):"; diff "$first" "$f" | sed 's/^/    /'; }
      done
      [ "$diff_n" = 0 ] && echo "  identical on all $(find "$RESULTS/inventory" -name '*.txt.settings' | wc -l | tr -d ' ') machines"
    fi
    rm -f "$RESULTS"/inventory/*.txt.settings
  } > "$out"
  log "settings: $(tail -1 "$out" | sed 's/^ *//')"
}

distribute_and_setup() {
  local gen_pub gen_priv token names n host pids failed
  names=$(all_names)
  # shellcheck disable=SC2086
  wait_ssh $names
  token=$(get_state TOKEN)
  gen_pub=$(machine_field gen-a pub); gen_priv=$(machine_field gen-a priv)
  log "uploading the bundle ($(du -h "$BUNDLE" | cut -f1)) to gen-a"
  scp_to "$gen_pub" "$BUNDLE" /tmp/ab-bundle.tgz
  on "$gen_pub" "mkdir -p /tmp/abdist && mv /tmp/ab-bundle.tgz /tmp/abdist/ && \
     (setsid nohup python3 -m http.server 8000 --bind $gen_priv --directory /tmp/abdist > /tmp/abdist.log 2>&1 < /dev/null &) && sleep 1"
  pids=""
  for n in $names; do
    host=$(machine_field "$n" pub)
    (
      on "$host" "set -e
        cd /tmp && { [ -f /tmp/abdist/ab-bundle.tgz ] && cp /tmp/abdist/ab-bundle.tgz /tmp/ab.tgz || curl -sSf --retry 5 -o /tmp/ab.tgz http://$gen_priv:8000/ab-bundle.tgz; }
        sudo rm -rf /opt/ab && sudo tar -C /opt -xzf /tmp/ab.tgz && sudo mkdir -p /opt/ab/run
        echo '$token' | sudo tee /opt/ab/run/agent.token >/dev/null && sudo chmod 600 /opt/ab/run/agent.token
        echo '$(base64 < "$STATE/hosts.env" | tr -d '\n')' | base64 -d | sudo tee /opt/ab/run/hosts.env >/dev/null"
      on "$host" "sudo /opt/ab/vm/setup.sh $(machine_field "$n" role) $n"
    ) > "$STATE/setup-$n.log" 2>&1 &
    pids="$pids $!"
  done
  failed=0
  for p in $pids; do wait "$p" || failed=1; done
  # The bracket keeps the pattern from matching this command's own shell, which pkill would kill.
  on "$gen_pub" "pkill -f '[h]ttp.server 8000' || true; rm -rf /tmp/abdist" || true
  for n in $names; do tail -1 "$STATE/setup-$n.log" | sed "s/^/  /"; done
  [ "$failed" = 0 ] || die "setup failed on a machine; see $STATE/setup-*.log"
  mkdir -p "$RESULTS/inventory"
  for n in $names; do
    on "$(machine_field "$n" pub)" "cat /opt/ab/inventory/$n.txt" > "$RESULTS/inventory/$n.txt" 2>/dev/null || true
  done
  [ -f "$STATE/credit-specifications.json" ] && cp "$STATE/credit-specifications.json" "$RESULTS/inventory/"
  awk '{print $1, $2, $3, $4, $7, $8}' "$STATE/machines.tsv" > "$RESULTS/inventory/machines.txt"
  cp "$STATE/ips.tsv" "$RESULTS/inventory/placement.tsv"
  settings_record
  log "setup done; inventory in $RESULTS/inventory"
}

gen_of() { echo "gen-$(lc "$1")"; }

start_lanes() {
  local lane gen t0 dm envs
  for lane in $LANES; do
    gen=$(machine_field "$(gen_of "$lane")" pub)
    envs=""
    if [ "$TOPOLOGY" = ref ]; then
      # No phase runs past HARD_END (the fetch happens before the dead-man switch); the release cells
      # (lane A's fill phase) stop at the planned wall time.
      t0=$(lane_t0 "$lane"); dm=$(lane_deadman "$lane")
      envs="HARD_END=$(( t0 + (dm - FETCH_MIN - 5) * 60 ))"
      if is_shard "$lane"; then
        envs="$envs
CELLS=$CELLS
SHARD_CELLS=\"$(shard_cells "$lane")\"
COMPARE_BUDGET=$(shard_budget "$lane")"
        [ "$lane" = A ] && [ "$WITH_GATES" = 1 ] && envs="$envs
WITH_GATES=1
FILL_END=$(( t0 + (WALL_MIN - FETCH_MIN) * 60 ))"
      fi
    fi
    on "$gen" "echo '$(echo "$envs" | base64 | tr -d '\n')' | base64 -d | sudo tee /opt/ab/run/lane-$lane.env >/dev/null && \
      sudo mkdir -p /var/tmp/ab-out && sudo bash -c 'setsid nohup /opt/ab/vm/lane.sh $lane > /var/tmp/ab-out/lane-$lane.log 2>&1 < /dev/null &'"
    log "lane $lane started on $(gen_of "$lane")"
  done
  set_state LANES_STARTED "$(date +%s)"
}

# ---------------------------------------------------------------------------------------------
# Follow, fetch, release

released() { grep -q "^$1$" "$STATE/released" 2>/dev/null; }

release_machines() { # released-after-key
  local ids
  ids=$(awk -v r="$1" '$6==r {printf "%s ", $7}' "$STATE/machines.tsv")
  [ -n "$ids" ] || return 0
  # shellcheck disable=SC2086
  if aws_ ec2 terminate-instances --instance-ids $ids >/dev/null; then
    log "released (terminating) $(awk -v r="$1" '$6==r {printf "%s ", $1}' "$STATE/machines.tsv")"
  else
    echo "$1" >> "$STATE/release-failed"
    log "could NOT terminate $(awk -v r="$1" '$6==r {printf "%s ", $1}' "$STATE/machines.tsv")(credentials? run 'aws login'); the teardown retries, and the dead-man switch ends them by $(hm "$(deadman_at)")"
  fi
  echo "$1" >> "$STATE/released"
  awk -v r="$1" -v t="$(date +%s)" 'BEGIN{OFS="\t"} $6==r && $11=="-" {$11=t} {print}' "$STATE/machines.tsv" > "$STATE/m.tmp"
  mv "$STATE/m.tmp" "$STATE/machines.tsv"
}

# Where a lane's output lands: the results directory, or shards/<lane>/ in it for shards 2..N (their
# compare/ is merged into the results' compare/ by merge_shards).
lane_dest() { if is_shard "$1" && [ "$1" != A ]; then echo "$RESULTS/shards/$1"; else echo "$RESULTS"; fi; }

fetch_lane() { # lane
  local lane="$1" gen dest
  gen=$(machine_field "$(gen_of "$lane")" pub)
  dest=$(lane_dest "$lane")
  mkdir -p "$dest"
  if on "$gen" "sudo tar -C /var/tmp/ab-out -czf - ." > "$STATE/lane-$lane.tgz" 2>/dev/null; then
    tar -xzf "$STATE/lane-$lane.tgz" -C "$dest" && log "fetched lane $lane into $dest"
    # Every lane writes "status"; keep each one.
    [ -f "$dest/status" ] && mv "$dest/status" "$dest/status-$lane"
  else
    log "lane $lane: nothing fetched (generator unreachable)"
  fi
  # shellcheck disable=SC2046
  fetch_logs $(awk -v l="$lane" '$4==l && $6=="lane-"l && $3!="generator" {print $1}' "$STATE/machines.tsv")
  rtt_record
  echo "$lane" >> "$STATE/fetched"
}

# The harness's RTT measurement (connect round trips to the mock, in each phase's generator inventory).
rtt_record() {
  python3 - "$RESULTS" > "$RESULTS/rtt.txt" 2>/dev/null <<'PY' || true
import glob, json, os, sys
root = sys.argv[1]
print("phase          target                 samples  connect p50 us  p99 us  (reference bound: p99 <= 100 us)")
files = glob.glob(os.path.join(root, "*", "inventory", "generator.json"))
files += glob.glob(os.path.join(root, "shards", "*", "*", "inventory", "generator.json"))
for f in sorted(files):
    r = (json.load(open(f)).get("rtt_to_mock") or {})
    if not r:
        continue
    p99 = r.get("connect_p99_us")
    ok = "ok" if p99 is not None and p99 <= 100 else "OVER"
    parts = os.path.relpath(f, root).split(os.sep)
    phase = "/".join(parts[1:3]) if parts[0] == "shards" else parts[0]
    print(f"{phase:<14} {r.get('target', ''):<22} {r.get('samples', 0):>7}  {r.get('connect_p50_us')!s:>14}  {p99!s:>6}  {ok}")
PY
  [ -s "$RESULTS/rtt.txt" ] && log "rtt: $(tail -n +2 "$RESULTS/rtt.txt" | awk '{printf "%s p99 %s us; ", $1, $5}')"
}

# The agents' logs and the logs of the processes they started, from each machine.
fetch_logs() {
  local n host
  for n in "$@"; do
    host=$(machine_field "$n" pub)
    mkdir -p "$RESULTS/logs/$n"
    on "$host" "sudo tar -C / -czf - var/log/ab \$(cd / && ls var/tmp/ab-agent/*.log 2>/dev/null) 2>/dev/null" 2>/dev/null \
      | tar -xzf - -C "$RESULTS/logs/$n" 2>/dev/null || true
  done
}

# Lets this machine's current public address reach the instances over ssh (it can change during a
# long run, or between a run and its resume).
ensure_ingress() {
  local ip
  ip=$(curl -fsS -m 5 https://checkip.amazonaws.com 2>/dev/null | tr -d '[:space:]') || return 0
  [ -n "$ip" ] || return 0
  grep -qx "$ip" "$STATE/ingress" 2>/dev/null && return 0
  if aws_ ec2 authorize-security-group-ingress --group-id "$(get_state SG)" \
       --ip-permissions "IpProtocol=tcp,FromPort=22,ToPort=22,IpRanges=[{CidrIp=$ip/32,Description=orchestrator}]" >/dev/null 2>&1; then
    log "ssh allowed from $ip"
  fi
  echo "$ip" >> "$STATE/ingress"
}
fetched() { grep -q "^$1$" "$STATE/fetched" 2>/dev/null; }

lane_status() { # lane -> status text ("" when unreachable)
  on "$(machine_field "$(gen_of "$1")" pub)" "cat /var/tmp/ab-out/status 2>/dev/null || true" 2>/dev/null || true
}

all_fetched() { local l; for l in $LANES; do fetched "$l" || return 1; done; }

follow() {
  local now started lane st seen beat left deadline
  started=$(get_state LANES_STARTED)
  beat=0
  while :; do
    now=$(date +%s)
    for lane in $LANES; do
      fetched "$lane" && continue
      st=$(lane_status "$lane")
      seen=$(cat "$STATE/seen-$lane" 2>/dev/null || true)
      if [ -n "$st" ] && [ "$st" != "$seen" ]; then
        { diff <(echo "$seen") <(echo "$st") || true; } | sed -n "s/^> /  lane $lane: /p"
        echo "$st" > "$STATE/seen-$lane"
      fi
      [ -n "$st" ] || ensure_ingress
      if [ "$lane" = B ] && echo "$st" | grep -q '^c7-cluster done' && awk '$6=="c7-cluster"' "$STATE/machines.tsv" | grep -q . && ! released c7-cluster; then
        # shellcheck disable=SC2046
        fetch_logs $(awk '$6=="c7-cluster" && $3=="cluster" {print $1}' "$STATE/machines.tsv")
        release_machines c7-cluster
      fi
      deadline=$(( $(lane_t0 "$lane") + $(lane_dm_state "$lane") * 60 - 300 ))
      if echo "$st" | grep -q '^lane done'; then
        fetch_lane "$lane"
        release_machines "lane-$lane"
      elif [ "$now" -ge "$deadline" ]; then
        log "lane $lane: its dead-man switch is 5 minutes away: fetching what is done"
        fetch_lane "$lane"
        release_machines "lane-$lane"
      fi
    done
    all_fetched && break
    beat=$((beat + 1))
    if [ $((beat % 10)) = 0 ]; then
      log "running for $(( (now - started) / 60 )) min; the last dead-man switch in $(( ($(deadman_at) - now) / 60 )) min"
      left=$(cred_left_quiet)
      if [ -n "$left" ] && [ "$left" -lt 20 ]; then
        log "WARNING: the AWS login session has ~$left min left: run 'aws login' now so the release and teardown calls work (the phases go on regardless)"
      fi
    fi
    sleep "$POLL_S"
  done
}

# Puts every shard's comparison into the results' compare/: each target's cell directories, a
# targets.json listing the cells in run order, each target's run.json spanning every shard, and
# shards.json (which shard ran which cell). The shards' own output stays under shards/<lane>/.
merge_shards() {
  [ "$TOPOLOGY" = ref ] && [ "$SHARDS" -gt 1 ] || return 0
  [ -f "$STATE/shards.tsv" ] || { log "no shard assignment recorded: nothing merged"; return 0; }
  python3 - "$RESULTS" "$STATE/shards.tsv" <<'PY2' && log "merged the shards' comparison into $RESULTS/compare"
import json, os, shutil, sys
root, assign = sys.argv[1], sys.argv[2]
order, lane_of = [], {}
for line in open(assign):
    lane, cell, _ = line.split()
    order.append(cell)
    lane_of[cell] = lane
srcs = [("A", os.path.join(root, "compare"))]
sdir = os.path.join(root, "shards")
if os.path.isdir(sdir):
    srcs += [(l, os.path.join(sdir, l, "compare")) for l in sorted(os.listdir(sdir))]
srcs = [(l, d) for l, d in srcs if os.path.isfile(os.path.join(d, "targets.json"))]
if not srcs:
    sys.exit("no shard has a comparison")
dest = os.path.join(root, "compare")
os.makedirs(dest, exist_ok=True)
targets, spans = [], {}
for lane, src in srcs:
    tf = json.load(open(os.path.join(src, "targets.json")))
    for t in tf["targets"]:
        if t not in targets:
            targets.append(t)
        sd, dd = os.path.join(src, t), os.path.join(dest, t)
        if not os.path.isfile(os.path.join(sd, "run.json")):
            continue
        run = json.load(open(os.path.join(sd, "run.json")))
        lo, hi = spans.get(t, (run["started_at"], run["ended_at"]))
        spans[t] = (min(lo, run["started_at"]), max(hi, run["ended_at"]))
        if src == dest:
            continue
        os.makedirs(os.path.join(dd, "cells"), exist_ok=True)
        for c in os.listdir(os.path.join(sd, "cells")):
            shutil.copytree(os.path.join(sd, "cells", c), os.path.join(dd, "cells", c), dirs_exist_ok=True)
        if not os.path.isfile(os.path.join(dd, "run.json")):
            shutil.copy2(os.path.join(sd, "run.json"), os.path.join(dd, "run.json"))
for t, (lo, hi) in spans.items():
    p = os.path.join(dest, t, "run.json")
    run = json.load(open(p))
    run["started_at"], run["ended_at"] = lo, hi
    json.dump(run, open(p, "w"), indent=2)
present = {c for t in targets for c in (os.listdir(os.path.join(dest, t, "cells")) if os.path.isdir(os.path.join(dest, t, "cells")) else [])}
json.dump({"targets": targets, "cells": [c for c in order if c in present]}, open(os.path.join(dest, "targets.json"), "w"), indent=2)
json.dump({"shards": len({l for l in lane_of.values()}), "cell_shard": lane_of,
           "note": "cells are compared within a shard (one machine set), never across shards"},
          open(os.path.join(dest, "shards.json"), "w"), indent=2)
PY2
}

# A failed or missing self-test does not stop the cells (they run with --skip-selftest after it),
# but it must not go unseen: each failed check goes into the not_publishable list of every run.json
# the self-test's machines produced, which the reports print as "Not publishable", and into
# selftest.txt. Run again after a re-fetch, it adds nothing twice.
annotate_selftests() {
  [ -n "${RESULTS:-}" ] && [ -d "$RESULTS" ] || return 0
  python3 - "$RESULTS" <<'PY2' || log "could not annotate the self-tests"
import glob, json, os, sys
root = sys.argv[1]
def selftests(pattern):
    out = []
    for d in sorted(glob.glob(os.path.join(root, pattern))):
        f = os.path.join(d, "selftest.json")
        name = os.path.relpath(d, root)
        if not os.path.isfile(f):
            out.append(f"self-test {name}: no result (it did not finish)")
            continue
        for c in json.load(open(f)).get("checks", []):
            if not c.get("passed"):
                out.append(f"self-test {name}: {c.get('name')} failed: {c.get('detail')}")
    return out
groups = {
    "compare": selftests("selftest-a") + selftests("shards/*/selftest-a*"),
    "gates": selftests("selftest-a"),
    "c7-cluster": selftests("selftest-b"),
    "c15": selftests("selftest-c"),
}
lines = []
for phase, notes in groups.items():
    runs = glob.glob(os.path.join(root, phase, "run.json")) + glob.glob(os.path.join(root, phase, "*", "run.json"))
    if not runs:
        continue
    lines += [f"{phase}: {n}" for n in notes] or [f"{phase}: every self-test check passed"]
    for f in runs:
        run = json.load(open(f))
        np = run.setdefault("not_publishable", [])
        add = [n for n in notes if n not in np]
        if add:
            np.extend(add)
            json.dump(run, open(f, "w"), indent=2)
with open(os.path.join(root, "selftest.txt"), "w") as f:
    f.write("\n".join(lines) + "\n")
for l in lines:
    print(l)
PY2
}

render_reports() {
  local d extra
  if [ ! -x "$REPORT_BIN" ]; then
    log "no native antirion-bench at $REPORT_BIN: reports not rendered locally"; return 0
  fi
  for d in "$RESULTS"/*/; do
    d=${d%/}
    if [ -f "$d/run.json" ] || [ -f "$d/targets.json" ]; then
      extra=()
      # C15's gate reads C12 from the ref-4 run.
      [ "$(basename "$d")" = c15 ] && [ -d "$RESULTS/gates" ] && extra=(--with "$RESULTS/gates")
      if "$REPORT_BIN" --bench-dir "$BENCH_SRC/bench" report "$d" ${extra[@]+"${extra[@]}"} > "$d.report.txt" 2>&1; then
        log "rendered $(basename "$d"): $(tail -1 "$d.report.txt")"
      else
        log "rendering $(basename "$d") failed: $(tail -1 "$d.report.txt")"
      fi
    fi
  done
}

# ---------------------------------------------------------------------------------------------
# Teardown and cost

cost_summary() {
  local now
  now=$(date +%s)
  echo "Resources of run $RUN_ID (topology $TOPOLOGY):"
  echo "  security group $(get_state SG), key pair $(get_state KEY_NAME), placement group $(get_state PLACEMENT_GROUP), AZ $(get_state AZ), AMI $(get_state AMI)"
  [ -s "$STATE/machines.tsv" ] || { echo "  no instances"; return 0; }
  printf "  %-7s %-11s %-9s %-20s %6s %9s\n" machine type role instance hours on-demand
  while IFS="$(printf '\t')" read -r name type role _ _ _ id _ _ t0 t1; do
    [ "$t1" = "-" ] && t1=$now
    awk -v n="$name" -v t="$type" -v r="$role" -v i="$id" -v s="$(( t1 - t0 ))" -v p="$(price_of "$type")" -v ip="$PUBLIC_IP_PER_H" \
        -v gb="$VOLUME_GB" -v gbm="$GP3_PER_GB_MONTH" 'BEGIN {h = s / 3600;
        printf "  %-7s %-11s %-9s %-20s %6.2f %9.3f\n", n, t, r, i, h, h * p + h * ip + gb * gbm * h / 730}'
  done < "$STATE/machines.tsv" | tee "$STATE/cost-lines.txt"
  local od
  od=$(awk '{s+=$6} END {printf "%.2f", s}' "$STATE/cost-lines.txt")
  if [ "$TOPOLOGY" = t3 ]; then
    echo "  on-demand (with public IPv4 and gp3): \$$od; T3 unlimited surplus: $(surplus_charged)"
  else
    echo "  on-demand (with public IPv4 and gp3): \$$od"
  fi
}

# Surplus credits spent, from CloudWatch (one credit = one vCPU-minute); an estimate until AWS
# settles the charge.
surplus_charged() {
  local ids start credits c
  ids=$(awk '{print $7}' "$STATE/machines.tsv")
  start=$(date -u -r "$(awk 'NR==1 {print $10}' "$STATE/machines.tsv")" +%Y-%m-%dT%H:%M:%SZ)
  credits=0
  for i in $ids; do
    c=$(aws_ cloudwatch get-metric-statistics --namespace AWS/EC2 --metric-name CPUSurplusCreditBalance \
          --dimensions Name=InstanceId,Value="$i" --start-time "$start" --end-time "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
          --period 300 --statistics Maximum --query 'max(Datapoints[].Maximum)' --output text 2>/dev/null || echo 0)
    case "$c" in None|"") c=0 ;; esac
    credits=$(echo "$credits + $c" | bc -l)
  done
  awk -v c="$credits" -v s="$SURPLUS_PER_VCPU_H" 'BEGIN {printf "%.0f credits = $%.2f (CloudWatch CPUSurplusCreditBalance maxima)", c, c / 60 * s}'
}

TORN=0
TEARDOWN_RC=0
teardown() {
  [ "$TORN" = 1 ] && return 0
  TORN=1
  log "tearing down run $RUN_ID"
  set +e
  AB_REGION="$REGION" AB_STATE_DIR="$STATE_ROOT" "$HERE/teardown.sh" "$RUN_ID" 2>&1 | sed 's/^/  /'
  TEARDOWN_RC=${PIPESTATUS[0]}
  set -e
  local now
  now=$(date +%s)
  if [ -s "$STATE/machines.tsv" ]; then
    awk -v t="$now" 'BEGIN{OFS="\t"} $11=="-" {$11=t} {print}' "$STATE/machines.tsv" > "$STATE/m.tmp" && mv "$STATE/m.tmp" "$STATE/machines.tsv"
  fi
  cost_summary | tee "$STATE/cost.txt" || true
  [ -n "${RESULTS:-}" ] && [ -d "$RESULTS" ] && cp "$STATE/cost.txt" "$RESULTS/cost.txt"
  if [ "$TEARDOWN_RC" = 0 ]; then
    set_state TORN_DOWN "$now"
    # The key pair is gone from AWS: its private key opens nothing any more.
    rm -f "$STATE/key.pem"
  else
    set_state TEARDOWN_FAILED "$now"
    log "TEARDOWN INCOMPLETE (teardown.sh exit $TEARDOWN_RC): see what may remain and the commands above; after 'aws login': AB_REGION=$REGION $HERE/teardown.sh $RUN_ID"
  fi
}

on_exit() {
  local rc=$? lane
  trap - EXIT INT TERM HUP
  [ -n "$RUN_ID" ] && [ -n "$STATE" ] || exit "$rc"
  if [ "$rc" != 0 ] && [ -n "$(get_state LANES_STARTED 2>/dev/null)" ]; then
    log "stopping early (rc=$rc): fetching what is done"
    for lane in $LANES; do fetched "$lane" || fetch_lane "$lane" || true; done
    merge_shards || true
    annotate_selftests || true
  fi
  teardown
  [ "$rc" = 0 ] && [ "$TEARDOWN_RC" != 0 ] && rc=3
  exit "$rc"
}
on_signal() { log "signal received"; exit 130; }

# ---------------------------------------------------------------------------------------------
# Commands

preflight() { # minutes the credentials must last, what for
  command -v aws >/dev/null || die "aws CLI missing"
  cred_require "$1" "$2"
  local acct
  acct=$(aws sts get-caller-identity --query Account --output text)
  log "AWS account $acct, region $REGION"
  if [ -n "$ACCOUNT" ] && [ "$acct" != "$ACCOUNT" ]; then
    die "the credentials are for account $acct, not $ACCOUNT (AB_ACCOUNT)"
  fi
}

new_run_id() { echo "aws-$(date -u +%Y%m%dt%H%M)-$(openssl rand -hex 2)"; }

bundle_check() {
  [ -f "$BUNDLE" ] || die "no bundle at $BUNDLE ($HERE/build-bundle.sh)"
  local f listing
  listing=$(tar -tzf "$BUNDLE")
  for f in ab/bin/antirion ab/vm/lane.sh ab/vm/targets.sh ab/cells/comparison.tsv ab/cells/headline.yaml ab/cells/gates5.yaml ab/cells/c15.yaml; do
    echo "$listing" | grep -qx "$f" || die "the bundle has no $f: REPACK=1 $HERE/build-bundle.sh (or a full build) first"
  done
  echo "$listing" | grep -q '^ab/targets/[^/]*/target.yaml$' || die "the bundle has no comparison target (ab/targets/*/target.yaml)"
  tar -xzOf "$BUNDLE" ab/vm/lane.sh | grep -q 'SHARD_CELLS' || die "the bundle's lane.sh predates shards: REPACK=1 $HERE/build-bundle.sh first"
  if [ -n "$SHARDS_FILE" ]; then
    while read -r _ f _; do
      echo "$listing" | grep -qx "ab/cells/cmp/$f.yaml" || die "the bundle has no cells/cmp/$f.yaml: REPACK=1 $HERE/build-bundle.sh first"
    done < "$SHARDS_FILE"
  fi
}

cmd_run() {
  local need
  need=$(( $(wall_planned) + 30 ))
  preflight "$need" "this run (planned wall $(wall_planned) min + 30)"
  bundle_check
  if [ "$TOPOLOGY" = ref ]; then
    local e
    e=$(cost_lines mm_expected | cost_total)
    awk -v e="$e" -v c="$COST_CAP" 'BEGIN {exit !(e <= c)}' || die "expected cost \$$e is above COST_CAP=\$$COST_CAP"
    log "expected cost \$$e (cap \$$COST_CAP)"
  fi
  quota_require
  RUN_ID="${RUN_ID:-$(new_run_id)}"
  init_state
  RESULTS="$RESULTS_ROOT/$RUN_ID"
  mkdir -p "$RESULTS"
  set_state TOPOLOGY "$TOPOLOGY"
  set_state CELLS "$CELLS"
  set_state SHARDS "$SHARDS"
  set_state SEL_LANES "$SEL_LANES"
  set_state WITH_GATES "$WITH_GATES"
  set_state REGION "$REGION"
  set_state WALL_MIN "$WALL_MIN"
  set_state RESULTS "$RESULTS"
  if [ -n "$SHARDS_FILE" ]; then
    cp "$SHARDS_FILE" "$STATE/shards.tsv"; rm -f "$SHARDS_FILE" "$TABLE_FILE"; SHARDS_FILE="$STATE/shards.tsv"
    cp "$SHARDS_FILE" "$RESULTS/shards.tsv"
  fi
  set_state TOKEN "$(openssl rand -hex 24)"
  chmod 600 "$STATE/state"
  [ -f "${BUNDLE%.tgz}.manifest.json" ] && cp "${BUNDLE%.tgz}.manifest.json" "$RESULTS/bundle.manifest.json"
  plan > "$RESULTS/plan.txt" 2>/dev/null
  log "run $RUN_ID ($TOPOLOGY, cells $CELLS, $REGION): state $STATE, results $RESULTS"
  trap on_exit EXIT
  trap on_signal INT TERM HUP
  create_network "$(echo "$MACHINES" | awk '{print $2}' | tr '\n' ' ')"
  create_key
  launch "$MACHINES"
  distribute_and_setup
  start_lanes
  follow
  merge_shards
  annotate_selftests
  render_reports
  log "results: $RESULTS"
}

cmd_resume() {
  RUN_ID="$1"
  STATE="$STATE_ROOT/runs/$RUN_ID"
  [ -s "$STATE/state" ] || die "no state for run $RUN_ID under $STATE_ROOT/runs (AB_STATE_DIR)"
  TOPOLOGY=$(get_state TOPOLOGY); TOPOLOGY="${TOPOLOGY:-t3}"
  CELLS=$(get_state CELLS); CELLS="${CELLS:-all}"
  SHARDS=$(get_state SHARDS); SHARDS="${SHARDS:-1}"
  SEL_LANES=$(get_state SEL_LANES); SEL_LANES="${SEL_LANES:-A}"
  WITH_GATES=$(get_state WITH_GATES); WITH_GATES="${WITH_GATES:-0}"
  REGION=$(get_state REGION); REGION="${REGION:-us-east-1}"
  WALL_MIN=$(get_state WALL_MIN)
  if [ "$TOPOLOGY" = ref ] && lane_selected A; then
    [ -f "$STATE/shards.tsv" ] || die "run $RUN_ID has no shard assignment ($STATE/shards.tsv)"
    SHARDS_FILE="$STATE/shards.tsv"
  fi
  configure
  local left_run
  left_run=$(( ($(deadman_at) - $(date +%s)) / 60 ))
  [ "$left_run" -lt 0 ] && left_run=0
  preflight $(( left_run + 30 )) "resuming (the run's last dead-man switch in $left_run min, + 30)"
  ssh_setup
  RESULTS=$(get_state RESULTS)
  trap on_exit EXIT
  trap on_signal INT TERM HUP
  if [ -z "$(get_state LANES_STARTED)" ]; then
    log "run $RUN_ID never started its lanes: tearing down"; return 0
  fi
  ensure_ingress
  if [ "${NOW:-0}" = 1 ]; then
    for lane in $LANES; do fetched "$lane" || fetch_lane "$lane"; done
  else
    log "following run $RUN_ID ($TOPOLOGY)"
    follow
  fi
  merge_shards
  annotate_selftests
  render_reports
  log "results: $RESULTS"
}

cmd_smoke() {
  local type host
  preflight 40 "the smoke test (10 min + 30)"
  RUN_ID="smoke-$(date -u +%Y%m%dt%H%M)-$(openssl rand -hex 2)"
  init_state
  set_state TOPOLOGY "$TOPOLOGY"
  set_state REGION "$REGION"
  RESULTS=""
  if [ "$TOPOLOGY" = ref ]; then type=c7i.xlarge; else type=t3.medium; fi
  trap on_exit EXIT
  trap on_signal INT TERM HUP
  create_network "$type"
  create_key
  # The smoke machine: lane S, a 30 min dead-man switch.
  MACHINES="smoke $type smoke S SMOKE lane-S"
  PHASES="S smoke 0 0"; LANES="S"; SETUP_MIN=0; FETCH_MIN=0; DEADMAN_MARGIN_MIN=30
  if [ "$TOPOLOGY" = t3 ]; then DEADMAN_MARGIN_MIN=0; PHASES="S smoke 0 20"; fi
  launch "$MACHINES"
  log "deadman: $(get_state DEADMAN_MIN_S) min"
  host=$(machine_field smoke pub)
  wait_ssh smoke
  log "uname -a: $(on "$host" 'uname -a')"
  log "dead-man: $(on "$host" 'cat /etc/ab-deadman; cat /run/systemd/shutdown/scheduled 2>/dev/null' | tr '\n' ' ')"
  log "placement: $(awk '{print $1, "group=" $4}' "$STATE/ips.tsv")"
  log "tags: $(aws_ ec2 describe-tags --filters "Name=tag:RunId,Values=$RUN_ID" --query 'Tags[].[ResourceType,ResourceId,Key,Value]' --output text | awk '{printf "%s:%s:%s=%s; ", $1, $2, $3, $4}')"
  if [ "${SMOKE_SETUP:-0}" = 1 ] && [ "$TOPOLOGY" = ref ]; then
    log "the system settings of the reference topology on this machine (vm/setup.sh base)"
    scp_to "$host" "$HERE/vm/setup.sh" /tmp/ab-setup.sh
    on "$host" "sudo mkdir -p /opt/ab/run && sudo AB_TOPOLOGY=ref bash /tmp/ab-setup.sh base smoke && cat /opt/ab/inventory/smoke.txt" 2>&1 | sed 's/^/  /'
  elif [ "${SMOKE_SETUP:-0}" = 1 ]; then
    [ -f "$BUNDLE" ] || die "no bundle at $BUNDLE"
    set_state TOKEN "$(openssl rand -hex 24)"
    log "uploading the bundle and running the gateway setup and a three-target check on this one machine"
    scp_to "$host" "$BUNDLE" /tmp/ab.tgz
    on "$host" "sudo tar -C /opt -xzf /tmp/ab.tgz && sudo mkdir -p /opt/ab/run && echo '$(get_state TOKEN)' | sudo tee /opt/ab/run/agent.token >/dev/null"
    on "$host" "sudo /opt/ab/vm/setup.sh gateway smoke" 2>&1 | tail -4 | sed 's/^/  /'
    on "$host" "cat /opt/ab/inventory/smoke.txt" | sed 's/^/  /'
    on "$host" "sudo /opt/ab/vm/smoke.sh" 2>&1 | tee "$STATE/smoke-check.log" | grep -v '^$' | sed 's/^/  /'
  fi
  if [ -n "${SMOKE_HOLD:-}" ]; then
    log "holding the machine for $SMOKE_HOLD s (ssh -i $KEY_FILE ubuntu@$host)"; sleep "$SMOKE_HOLD"
  fi
  if [ "$TOPOLOGY" = t3 ]; then
    log "credit: $(aws_ ec2 describe-instance-credit-specifications --instance-ids "$(machine_field smoke id)" --query 'InstanceCreditSpecifications[0].CpuCredits' --output text)"
  fi
  log "shutdown behaviour: $(aws_ ec2 describe-instance-attribute --instance-id "$(machine_field smoke id)" --attribute instanceInitiatedShutdownBehavior --query 'InstanceInitiatedShutdownBehavior.Value' --output text)"
}

# 45m, 1h, 1h30m or plain minutes -> minutes.
minutes_of() {
  local v="$1" h=0 m=0
  case "$v" in
    *h*m) h=${v%%h*}; m=${v#*h}; m=${m%m} ;;
    *h) h=${v%h} ;;
    *m) m=${v%m} ;;
    *) m=$v ;;
  esac
  case "$h$m" in *[!0-9]*|'') die "cannot read the duration $v (45m, 1h, 1h30m)" ;; esac
  echo $(( h * 60 + m ))
}

# --deadline: the fewest shards whose planned wall time fits, within the vCPU quota left (when it
# can be read).
pick_shards() {
  local n max wall peak avail="" best=""
  max=$(selected_table | grep -c .)
  quota_status
  [ -n "$QUOTA" ] && [ -n "$QUOTA_USED" ] && avail=$(( QUOTA - QUOTA_USED ))
  for n in $(seq 1 "$max"); do
    SHARDS=$n; assign_shards; configure
    wall=$(wall_planned); peak=$(peak_vcpus)
    if [ -n "$avail" ] && [ "$peak" -gt "$avail" ]; then
      die "--deadline ${DEADLINE_MIN}m: $n shards need $peak vCPUs, the quota leaves $avail; ${best:-no shard count} fits the quota (raise $QUOTA_CODE or the deadline)"
    fi
    best="$n shard(s) take ~${wall} min"
    if [ "$wall" -le "$DEADLINE_MIN" ]; then
      log "--deadline ${DEADLINE_MIN}m: $n shard(s), ~${wall} min planned, $peak vCPUs" >&2
      return 0
    fi
  done
  die "--deadline ${DEADLINE_MIN}m: even one cell a shard takes ~${wall} min (the longest cell sets the floor)"
}

# The whole file is read before anything runs (an edit during a run cannot change it).
main() {
  local cmd="run" arg
  while [ $# -gt 0 ]; do
    arg="$1"; shift
    case "$arg" in
      --topology) TOPOLOGY="${1:?--topology t3|ref}"; shift ;;
      --cells|--only) CELLS="${1:?--cells compare|headline|<id,id,...>}"; shift ;;
      --shards) SHARDS="${1:?--shards N}"; shift ;;
      --lanes) SEL_LANES=$(echo "${1:?--lanes A,B,C}" | tr -d ' ' | tr '[:lower:]' '[:upper:]'); shift ;;
      --with-gates) WITH_GATES=1 ;;
      --deadline) DEADLINE_MIN=$(minutes_of "${1:?--deadline 45m}"); shift ;;
      --plan) cmd=plan ;;
      --smoke) cmd=smoke ;;
      --setup) SMOKE_SETUP=1 ;;
      --resume) cmd=resume; RUN_ID="${1:?--resume <run id>}"; shift ;;
      --now) NOW=1 ;;
      --run-id) RUN_ID="${1:?--run-id <id>}"; shift ;;
      *) sed -n '4,26p' "$0"; exit 2 ;;
    esac
  done
  case "$cmd" in
    resume) cmd_resume "$RUN_ID"; return ;;
  esac
  case "$SHARDS" in
    ''|*[!0-9]*|0) die "--shards takes a whole number of at least 1" ;;
  esac
  if [ "$TOPOLOGY" = ref ] && lane_selected A; then
    TABLE_FILE=$(mktemp); comparison_table > "$TABLE_FILE.t"; mv "$TABLE_FILE.t" "$TABLE_FILE"
    if [ -n "$DEADLINE_MIN" ]; then pick_shards; else assign_shards; fi
  fi
  configure
  case "$cmd" in
    plan) plan; plan_checks; rm -f "$SHARDS_FILE" "$TABLE_FILE" ;;
    smoke) cmd_smoke ;;
    run) cmd_run ;;
  esac
}
main "$@"
exit $?
