#!/usr/bin/env bash
# Prepares one VM of the AWS run (as root). The bundle is already unpacked at /opt/ab.
#   setup.sh <role> <name>
# roles: generator | mock | gateway | cluster | store            (t3 topology)
#        generator | mock | gateway | node | postgres | valkey  (reference topology, run.sh --topology ref)
#        base  (the system settings and the inventory only)
# The agent token is read from /opt/ab/run/agent.token; the topology (TOPOLOGY=t3|ref) from
# /opt/ab/run/hosts.env, or from AB_TOPOLOGY.
set -euo pipefail
ROLE="$1"
NAME="$2"
AB=/opt/ab
export DEBIAN_FRONTEND=noninteractive
TOPOLOGY="${AB_TOPOLOGY:-$(sed -n 's/^TOPOLOGY=//p' "$AB/run/hosts.env" 2>/dev/null | tail -1)}"
TOPOLOGY="${TOPOLOGY:-t3}"
GOVERNOR=""

log() { echo "[$(date -u +%H:%M:%S)] $NAME: $*"; }

# The listen ports of the gateways, the mock, the agent and the stores are reserved: an outgoing
# connection (or a probe connecting to itself while nothing listens yet) that takes one of them as its
# local port leaves a TIME-WAIT socket that makes the next bind of that port fail. The reference
# topology uses the reference settings (docs/benchmarks.md: ephemeral range 1024-65535, governor
# performance), so its reserved list also covers 8080/8081, 5432 and 6379; the t3 topology keeps
# the range of its earlier runs. Every machine of a run gets the same file.
tune() {
  local range reserved g
  if [ "$TOPOLOGY" = ref ]; then
    range="1024 65535"; reserved="5432,6379,8080,8081,18080,18090,19401,19500"
  else
    range="10000 65535"; reserved="18080,18090,19401,19500"
  fi
  cat > /etc/sysctl.d/90-ab.conf <<CONF
net.core.somaxconn = 65535
net.core.netdev_max_backlog = 65535
net.ipv4.tcp_max_syn_backlog = 65535
net.ipv4.ip_local_port_range = $range
net.ipv4.ip_local_reserved_ports = $reserved
net.ipv4.tcp_tw_reuse = 1
net.ipv4.tcp_fin_timeout = 10
fs.file-max = 4194304
fs.nr_open = 4194304
CONF
  sysctl -q --system >/dev/null
  cat > /etc/security/limits.d/90-ab.conf <<'CONF'
* soft nofile 1048576
* hard nofile 1048576
root soft nofile 1048576
root hard nofile 1048576
CONF
  ulimit -n 1048576
  GOVERNOR="not exposed (the hypervisor manages the CPU frequency)"
  if [ -w /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor ]; then
    for g in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do echo performance > "$g" 2>/dev/null || true; done
    GOVERNOR=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor)
  fi
}

# The settings as the kernel reports them after tune (recorded in the inventory).
settings() {
  local k pid
  echo "settings_of: $TOPOLOGY"
  for k in net.core.somaxconn net.ipv4.ip_local_port_range net.ipv4.ip_local_reserved_ports net.ipv4.tcp_tw_reuse \
           net.ipv4.tcp_fin_timeout net.core.netdev_max_backlog net.ipv4.tcp_max_syn_backlog fs.file-max fs.nr_open; do
    echo "sysctl $k: $(sysctl -n "$k" | tr '\t' ' ')"
  done
  echo "ulimit_n_setup: $(ulimit -n)"
  pid=$(pgrep -of '[a]ntirion-bench agent' || true)
  [ -n "$pid" ] && echo "ulimit_n_agent: $(awk '/Max open files/ {print $4}' "/proc/$pid/limits")"
  echo "governor: $GOVERNOR"
}

inventory() {
  mkdir -p "$AB/inventory"
  local tok itype s1 s2
  tok=$(curl -s -m 2 -X PUT http://169.254.169.254/latest/api/token -H 'X-aws-ec2-metadata-token-ttl-seconds: 60' || true)
  itype=$(curl -s -m 2 -H "X-aws-ec2-metadata-token: $tok" http://169.254.169.254/latest/meta-data/instance-type || echo unknown)
  s1=$(awk '/^cpu /{print $9, $2+$3+$4+$5+$6+$7+$8+$9}' /proc/stat)
  sleep 5
  s2=$(awk '/^cpu /{print $9, $2+$3+$4+$5+$6+$7+$8+$9}' /proc/stat)
  {
    echo "name: $NAME"
    echo "role: $ROLE"
    echo "instance_type: $itype"
    echo "placement_group: $(curl -s -m 2 -H "X-aws-ec2-metadata-token: $tok" http://169.254.169.254/latest/meta-data/placement/group-name || echo none)"
    echo "availability_zone: $(curl -s -m 2 -H "X-aws-ec2-metadata-token: $tok" http://169.254.169.254/latest/meta-data/placement/availability-zone || echo unknown)"
    echo "nproc: $(nproc)"
    echo "cpu_model: $(lscpu | sed -n 's/^Model name: *//p')"
    echo "steal_pct_5s: $(echo "$s1 $s2" | awk '{d=$4-$2; if (d>0) printf "%.2f", 100*($3-$1)/d; else print 0}')"
    echo "kernel: $(uname -r)"
    echo "mem_total: $(sed -n 's/^MemTotal: *//p' /proc/meminfo)"
    settings
  } > "$AB/inventory/$NAME.txt"
}

start_agent() {
  local token
  token=$(cat "$AB/run/agent.token")
  mkdir -p /var/tmp/ab-agent /var/log/ab
  # The bracket keeps the pattern from matching a shell whose command line contains it.
  if pgrep -f '[a]ntirion-bench agent' >/dev/null; then return; fi
  ulimit -n 1048576
  ANTIRION_BENCH_AGENT_TOKEN="$token" setsid nohup "$AB/bin/antirion-bench" agent \
    --listen 0.0.0.0:19500 --work-dir /var/tmp/ab-agent > /var/log/ab/agent.log 2>&1 < /dev/null &
  for _ in $(seq 1 50); do
    if (exec 3<>/dev/tcp/127.0.0.1/19500) 2>/dev/null; then log "agent up"; return; fi
    sleep 0.2
  done
  log "agent did not start"; cat /var/log/ab/agent.log; exit 1
}

# The comparison targets of the bundle (bench/targets/<name>/ with a target.yaml): each one's setup.sh.
gateway_targets() {
  local d
  apt-get -qq update >/dev/null || true
  for d in "$AB"/targets/*/; do
    [ -f "$d/target.yaml" ] && [ -x "$d/setup.sh" ] || continue
    log "target $(basename "$d"): setup"
    "$d/setup.sh"
  done
}

docker_up() {
  apt-get -qq update >/dev/null
  apt-get -qq install -y docker.io >/dev/null
  systemctl enable --now docker >/dev/null 2>&1 || true
}
pg_run() { # shared_buffers
  docker rm -f ab-pg >/dev/null 2>&1 || true
  docker run -d --name ab-pg --network host --ulimit nofile=1048576:1048576 -e POSTGRES_USER=antirion \
    -e POSTGRES_PASSWORD=antirion -e POSTGRES_DB=antirion postgres:16 -c max_connections=1000 \
    -c shared_buffers="$1" -c synchronous_commit=on >/dev/null
}
valkey_run() {
  docker rm -f ab-valkey >/dev/null 2>&1 || true
  docker run -d --name ab-valkey --network host --ulimit nofile=1048576:1048576 valkey/valkey:8 \
    valkey-server --port 6379 --protected-mode no --save '' --appendonly no --maxclients 100000 >/dev/null
}
pg_ready() { docker exec ab-pg pg_isready -q -U antirion; }
valkey_ready() { docker exec ab-valkey valkey-cli ping 2>/dev/null | grep -q PONG; }
wait_ready() { # what check-function container
  for _ in $(seq 1 60); do
    if "$2" >/dev/null 2>&1; then log "$1 ready"; return; fi
    sleep 2
  done
  log "$1 did not become ready"; docker logs "$3" 2>&1 | tail -20; exit 1
}

# t3 topology: both on one machine.
store() {
  log "store: PostgreSQL and Valkey in Docker"
  docker_up; pg_run 512MB; valkey_run
  wait_ready PostgreSQL pg_ready ab-pg
  wait_ready Valkey valkey_ready ab-valkey
}
# Reference topology: each on its own 8 vCPU / 16 GiB machine.
postgres() { log "PostgreSQL in Docker"; docker_up; pg_run 4GB; wait_ready PostgreSQL pg_ready ab-pg; }
valkey() { log "Valkey in Docker"; docker_up; valkey_run; wait_ready Valkey valkey_ready ab-valkey; }

tune
case "$ROLE" in
  generator|base) ;;
  mock|cluster|node) start_agent ;;
  gateway) gateway_targets; start_agent ;;
  store) store ;;
  postgres) postgres ;;
  valkey) valkey ;;
  *) echo "unknown role $ROLE" >&2; exit 2 ;;
esac
inventory
log "setup done ($ROLE, $TOPOLOGY settings)"
