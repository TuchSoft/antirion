#!/usr/bin/env bash
# Starts LiteLLM's proxy for an interleaved comparison, one worker per vCPU, from a fresh directory.
#   MOCK=host:port  [PORT=18090] [HOST=0.0.0.0]
#   [LITELLM_WORKERS=<n>] [LITELLM_WORK=/var/tmp/ab-litellm] [LITELLM_VENV=<dir>] [LITELLM_LOG_LEVEL=ERROR]
# The model cost map is read from the installed package (no startup fetch); telemetry, the admin UI
# and the API docs are off.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${MOCK:?MOCK=host:port}"
PORT="${PORT:-18090}"
HOST="${HOST:-0.0.0.0}"
WORKERS="${LITELLM_WORKERS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc)}"
DIR="${LITELLM_WORK:-/var/tmp/ab-litellm}/$PORT"
rm -rf "$DIR" && mkdir -p "$DIR"
sed -e "s|@MOCK@|$MOCK|g" "$HERE/config.template.yaml" > "$DIR/config.yaml"
export LITELLM_LOCAL_MODEL_COST_MAP=True LITELLM_LOG="${LITELLM_LOG_LEVEL:-ERROR}" LITELLM_TELEMETRY=False
export DISABLE_ADMIN_UI=True NO_DOCS=True
cd "$DIR"
exec "${LITELLM_VENV:-$HERE/venv}/bin/litellm" --config "$DIR/config.yaml" --host "$HOST" --port "$PORT" --num_workers "$WORKERS"
