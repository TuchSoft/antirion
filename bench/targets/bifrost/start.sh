#!/usr/bin/env bash
# Starts Bifrost for an interleaved comparison: a fresh application directory per start, the mock's
# latency profiles as four providers, one virtual key allowed only the upstream model (so an unknown
# model is refused, with 400), no request logs.
#   MOCK=host:port  [PORT=18080] [HOST=0.0.0.0]
#   [BIFROST_BIN=<binary>] [BIFROST_WORK=/var/tmp/ab-bifrost] [BIFROST_CONCURRENCY=10000]
#   [BIFROST_LOG_LEVEL=error]
# Tuning follows the settings of Bifrost's own published t3 benchmark: initial pool and buffer sizes
# by machine (15000/20000 on 4 or more vCPUs, 10000/15000 below), with a per-provider concurrency
# above the requests in flight on the longest mock profile (5,000 RPS x 1.5 s).
# The pricing datasheet and model parameters are read from local files (setup.sh unpacks them): a
# failed startup fetch is fatal, and the mock sits on a private address, hence allow_private_network
# on every provider.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="${BIFROST_BIN:-$HERE/bifrost-http}"
: "${MOCK:?MOCK=host:port}"
PORT="${PORT:-18080}"
HOST="${HOST:-0.0.0.0}"
NPROC=$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc)
if [ "$NPROC" -ge 4 ]; then POOL=15000; BUFFER=20000; else POOL=10000; BUFFER=15000; fi
CONC="${BIFROST_CONCURRENCY:-10000}"
APP="${BIFROST_WORK:-/var/tmp/ab-bifrost}/$PORT"
rm -rf "$APP" && mkdir -p "$APP"
sed -e "s|@MOCK@|$MOCK|g" -e "s|@POOL@|$POOL|g" -e "s|@BUFFER@|$BUFFER|g" -e "s|@CONC@|$CONC|g" \
    -e "s|@DIR@|$HERE|g" -e "s|@APP@|$APP|g" "$HERE/config.template.json" > "$APP/config.json"
exec "$BIN" -app-dir "$APP" -host "$HOST" -port "$PORT" -log-level "${BIFROST_LOG_LEVEL:-error}" -log-style json
