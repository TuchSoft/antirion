#!/usr/bin/env bash
# Prints the targets file of an interleaved comparison (antirion-bench cell --interleave): every
# comparison target of the bundle (/opt/ab/targets/<name>/target.yaml, in name order), then Antirion.
#   targets.sh <gateway address> <agent host> <mock host:port>
set -euo pipefail
AB="${AB:-/opt/ab}"
GW="$1"
AGENT="$2"
MOCK="$3"
echo "# Rendered by vm/targets.sh: the comparison's targets, in the order of every interleaved round."
for f in "$AB"/targets/*/target.yaml; do
  [ -f "$f" ] || continue
  dir=$(dirname "$f")
  sed -e '/^#/d' -e "s|@GW@|$GW|g" -e "s|@AGENT@|$AGENT|g" -e "s|@MOCK@|$MOCK|g" -e "s|@DIR@|$dir|g" \
      -e "s|@VERSION@|$(cat "$dir/VERSION")|g" "$f"
done
echo "- {name: antirion, kind: antirion}"
