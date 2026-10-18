#!/usr/bin/env bash
# Renders the topologies and targets files of the AWS run from /opt/ab/run/hosts.env (private
# addresses by role) into /opt/ab/run/.
# t3 topology (TOPOLOGY=t3 or unset):
#   lane A: aws-a.yaml (gateway xlarge GWX, machines xlarge GWX and medium GWM, mock MOCK_A),
#           targets-xlarge.yaml, targets-medium.yaml
#   lane B: aws-b.yaml (gateway GWB, mock MOCK_B), cluster.yaml (GWB, CL2, CL3 and the STORE)
# Reference topology (TOPOLOGY=ref), named as the harness's reference topologies so it treats the
# results as reference (docs/benchmarks.md):
#   lane A: ref-4.yaml (gateway GWX, mock MOCK_A, 16 generator workers), targets-ref-4.yaml
#   lane A<k> (shard k of the comparison, on its own machines): the same files from GW_A<k>, MOCK_A<k>
#   lane B: ref-4-cluster.yaml (name ref-4: gateway nodes GWB, CL2, CL3, PostgreSQL on PG, Valkey on VALKEY)
#   lane C: ref-16.yaml (gateway GW16, mock MOCK_C, 32 generator workers)
set -euo pipefail
RUN=/opt/ab/run
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "$RUN/hosts.env"
TOPOLOGY="${TOPOLOGY:-t3}"
REGION="${REGION:-us-east-1}"

node() { echo "{data: \"$1:8080\", admin: \"$1:8081\", agent: \"$1:19500\"}"; }

# name reference generator-workers mock-workers selftest(rate p99 seconds) mock nodes... ; stdin: extra
# YAML under gateway:
topology() {
  local name="$1" reference="$2" gw="$3" mw="$4" rate="$5" p99="$6" dur="$7" mock="$8"; shift 8
  if [ "$TOPOLOGY" = ref ]; then
    echo "# Rendered by vm/render.sh: AWS $REGION, c7i instances, one AZ, one cluster placement group."
  else
    echo "# Rendered by vm/render.sh: AWS $REGION, t3 instances in unlimited-credit mode, one AZ."
  fi
  echo "name: $name"
  echo "reference: $reference"
  echo "gateway:"
  echo "  launch: local"
  echo "  bin: \"/opt/ab/bin/antirion\""
  echo "  nice: 0"
  echo "  nodes:"
  for n in "$@"; do echo "    - $(node "$n")"; done
  cat
  echo "mock:"
  echo "  launch: local"
  echo "  listen: \"$mock:19401\""
  echo "  agent: \"$mock:19500\""
  echo "  workers: $mw"
  echo "  nice: 0"
  echo "generator:"
  echo "  workers: $gw"
  echo "  max_connections: 8192"
  echo "selftest:"
  echo "  mock_rate: $rate"
  echo "  mock_p99_us: $p99"
  echo "  duration_s: $dur"
}

store() { # postgres-host valkey-host
  echo "store:"
  echo "  postgres_url: \"postgres://antirion:antirion@$1:5432/antirion\""
  echo "  valkey_url: \"redis://$2:6379\""
}

targets() { # gateway-ip mock-ip
  "$HERE/targets.sh" "$1" "$1" "$2:19401"
}

if [ "$TOPOLOGY" = ref ]; then
  # The selftest of the reference topologies (ref-4.yaml, ref-16.yaml): 50,000 RPS, <= 50 us p99, 30 s.
  case "${1:-}" in
    A|A[0-9]*)
      if [ "$1" = A ]; then gw="$GWX"; else v="GW_$1"; gw="${!v}"; fi
      v="MOCK_$1"; mock="${!v}"
      topology ref-4 true 16 16 50000 50 30 "$mock" "$gw" < /dev/null > "$RUN/ref-4.yaml"
      targets "$gw" "$mock" > "$RUN/targets-ref-4.yaml"
      ;;
    B)
      { topology ref-4 true 16 16 50000 50 30 "$MOCK_B" "$GWB" "$CL2" "$CL3" < /dev/null; store "$PG" "$VALKEY"; } \
        > "$RUN/ref-4-cluster.yaml"
      ;;
    C)
      topology ref-16 true 32 32 50000 50 30 "$MOCK_C" "$GW16" < /dev/null > "$RUN/ref-16.yaml"
      ;;
    *) echo "usage: render.sh A|A<k>|B|C" >&2; exit 2 ;;
  esac
else
  case "${1:-}" in
    A)
      topology aws-t3-a false 4 4 20000 300 10 "$MOCK_A" "$GWX" > "$RUN/aws-a.yaml" <<EOF
  machines:
    xlarge:
      - $(node "$GWX")
    medium:
      - $(node "$GWM")
EOF
      targets "$GWX" "$MOCK_A" > "$RUN/targets-xlarge.yaml"
      targets "$GWM" "$MOCK_A" > "$RUN/targets-medium.yaml"
      ;;
    B)
      topology aws-t3-b false 4 4 20000 300 10 "$MOCK_B" "$GWB" > "$RUN/aws-b.yaml" < /dev/null
      { topology aws-t3-cluster false 4 4 20000 300 10 "$MOCK_B" "$GWB" "$CL2" "$CL3" < /dev/null; store "$STORE" "$STORE"; } \
        > "$RUN/cluster.yaml"
      ;;
    *) echo "usage: render.sh A|B" >&2; exit 2 ;;
  esac
fi
ls "$RUN"
