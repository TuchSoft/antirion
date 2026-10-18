#!/usr/bin/env bash
# Installs LiteLLM's proxy (the version in VERSION) into a uv virtual environment next to this
# script; installs uv first when it is missing.
#   [LITELLM_VENV=<dir>] [LITELLM_PYTHON=3.12]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERSION="$(cat "$HERE/VERSION")"
export PATH="$HOME/.local/bin:$PATH"
if ! command -v uv >/dev/null; then
  curl -LsSf https://astral.sh/uv/install.sh | sh >/dev/null
fi
VENV="${LITELLM_VENV:-$HERE/venv}"
uv venv --quiet --python "${LITELLM_PYTHON:-3.12}" "$VENV"
uv pip install --quiet --python "$VENV/bin/python" "litellm[proxy]==$VERSION"
"$VENV/bin/python" -c "import importlib.metadata as m; print('litellm', m.version('litellm'))"
