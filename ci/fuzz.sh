#!/usr/bin/env bash
# Runs the named fuzz targets of fuzz/ (every target when none is named) for the given number of
# seconds each (default 60), from their seeds and the corpus kept between runs; a crash fails the
# script and leaves its input in fuzz/artifacts/.
#   ci/fuzz.sh [seconds] [target...]
# Needs a nightly toolchain and cargo-fuzz.
set -euo pipefail
seconds="${1:-60}"
shift || true
cd "$(dirname "$0")/../fuzz"
targets=("$@")
if [ ${#targets[@]} -eq 0 ]; then
  read -r -a targets <<< "$(cargo +nightly fuzz list | tr '\n' ' ')"
fi
for target in "${targets[@]}"; do
  mkdir -p "corpus/$target"
  seeds=()
  if [ -d "seeds/$target" ]; then seeds=("seeds/$target"); fi
  echo "fuzzing $target for ${seconds}s"
  cargo +nightly fuzz run "$target" "corpus/$target" "${seeds[@]}" -- \
    -max_total_time="$seconds" -rss_limit_mb=2048 -timeout=10
done
