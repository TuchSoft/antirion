#!/usr/bin/env bash
# Installs Bifrost (the version in VERSION) next to this script: the release binary for Linux x86_64,
# checked against its pinned SHA-256, and the pricing datasheet and model parameters it reads at
# startup, unpacked from the snapshots committed here.
#   [BIFROST_URL=...] [BIFROST_SHA256=...]   another build of the same release
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
URL="${BIFROST_URL:-https://downloads.getmaxim.ai/bifrost/v2.2.4/linux/amd64/bifrost-http}"
SHA256="${BIFROST_SHA256:-138edf4970f4a4fe6f40d7dac41b5e8966662e897b292070a8ef01e92fc35035}"

curl -sSfL --retry 3 -o "$HERE/bifrost-http.part" "$URL"
echo "$SHA256  $HERE/bifrost-http.part" | sha256sum -c --quiet -
mv "$HERE/bifrost-http.part" "$HERE/bifrost-http"
chmod 755 "$HERE/bifrost-http"
for f in datasheet model-parameters; do
  gzip -dc "$HERE/$f.json.gz" > "$HERE/$f.json"
done
echo "bifrost $(cat "$HERE/VERSION"): $HERE/bifrost-http"
