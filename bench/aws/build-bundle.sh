#!/usr/bin/env bash
# Builds the bundle the AWS run unpacks at /opt/ab on every machine: Linux x86_64 binaries (glibc
# <= 2.35), the harness data (matrix, profiles, corpus, topologies), the comparison targets (every
# bench/targets/<name>/ with a target.yaml), the cell files (gen-cells.py) and the VM scripts (vm/).
# docs/benchmarks.md has the whole procedure.
#
#   GATEWAY_BIN=<file>        the gateway under test: a Linux x86_64 release binary, normally the
#                             `antirion` file of the CI build's `binaries` artifact (pr.yml). Without
#                             it, the gateway is cross-built from GATEWAY_SRC with cargo zigbuild.
#   GATEWAY_COMMIT=<sha>      the commit GATEWAY_BIN was built from, for the manifest (the harness
#                             records the binary's own --version in every run either way)
#   GATEWAY_SRC=<checkout>    default: this repository
#   BENCH_SRC=<checkout>      the checkout holding bench/ (default: this repository)
#   BUNDLE_OUT=<dir>          where ab-bundle.tgz and ab-bundle.manifest.json go (default: bench/aws/.build)
#   BUILD_TARGET_ROOT=<dir>   the cargo target directories (default: <repository>/target/bench-aws)
#   REPACK=1                  no build at all: the binaries and harness data of the bundle already in
#                             BUNDLE_OUT, with this checkout's VM scripts, targets and fresh cell files
#
# Needs cargo, cargo-zigbuild with zig, and uv (for gen-cells.py's PyYAML). CARGO_BUILD_JOBS is
# passed through to cargo.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
GATEWAY_SRC="${GATEWAY_SRC:-$REPO}"
BENCH_SRC="${BENCH_SRC:-$REPO}"
TRIPLE="x86_64-unknown-linux-gnu"
GLIBC="2.35"
TARGET_ROOT="${BUILD_TARGET_ROOT:-$REPO/target/bench-aws}"
OUT="${BUNDLE_OUT:-$HERE/.build}"
TARGETS_SRC="$BENCH_SRC/bench/targets"

die() { echo "build-bundle.sh: $*" >&2; exit 1; }
command -v uv >/dev/null || die "uv is not installed (gen-cells.py runs with it)"
mkdir -p "$OUT"
STAGE="$OUT/stage.$$"
rm -rf "$STAGE" && mkdir -p "$STAGE"
trap 'rm -rf "$STAGE"' EXIT

# The comparison targets: each directory with a target.yaml, without anything its setup.sh installs.
copy_targets() {
  local d name
  mkdir -p "$STAGE/ab/targets"
  for d in "$TARGETS_SRC"/*/; do
    [ -f "$d/target.yaml" ] || continue
    name=$(basename "$d")
    mkdir -p "$STAGE/ab/targets/$name"
    ( cd "$d" && find . -type f ! -path './venv/*' ! -name 'bifrost-http*' ! -name 'datasheet.json' \
        ! -name 'model-parameters.json' -exec cp {} "$STAGE/ab/targets/$name/" \; )
    echo "target $name: $(cat "$d/VERSION")"
  done
}

# The parts every bundle takes from this checkout: VM scripts, targets, cell files.
fresh_parts() {
  rm -rf "$STAGE/ab/vm" "$STAGE/ab/cells" "$STAGE/ab/targets" "$STAGE/ab/gw-a" "$STAGE/ab/gw-b"
  mkdir -p "$STAGE/ab/vm" "$STAGE/ab/cells"
  cp "$HERE"/vm/*.sh "$STAGE/ab/vm/"
  copy_targets
  uv run --quiet --with pyyaml python "$HERE/gen-cells.py" "$BENCH_SRC/bench/matrix/cells.yaml" "$STAGE/ab/cells"
  find "$STAGE/ab" -name '*.sh' -exec chmod 755 {} +
}

pack() {
  COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -C "$STAGE" -czf "$OUT/ab-bundle.tgz.part" ab 2>/dev/null \
    || tar -C "$STAGE" -czf "$OUT/ab-bundle.tgz.part" ab
  mv "$OUT/ab-bundle.tgz.part" "$OUT/ab-bundle.tgz"
}
sha() { shasum -a 256 "$1" | cut -d' ' -f1; }

if [ "${REPACK:-0}" = 1 ]; then
  [ -f "$OUT/ab-bundle.tgz" ] || die "REPACK=1: no bundle at $OUT/ab-bundle.tgz to start from"
  tar -C "$STAGE" -xzf "$OUT/ab-bundle.tgz"
  fresh_parts
  pack
  python3 - "$OUT/ab-bundle.manifest.json" "$(sha "$OUT/ab-bundle.tgz")" <<'PY'
import json, sys, datetime
path, sha = sys.argv[1], sys.argv[2]
try:
    m = json.load(open(path))
except Exception:
    m = {"sha256": {}}
m.setdefault("sha256", {})["bundle"] = sha
m["repacked_at"] = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
json.dump(m, open(path, "w"), indent=2)
PY
  ls -la "$OUT/ab-bundle.tgz"; cat "$OUT/ab-bundle.manifest.json"; echo
  exit 0
fi

for d in "$GATEWAY_SRC" "$BENCH_SRC/bench"; do
  [ -d "$d" ] || die "missing $d"
done
command -v cargo >/dev/null || die "cargo is not on PATH"
command -v cargo-zigbuild >/dev/null || die "cargo-zigbuild is not installed (cargo install cargo-zigbuild; it needs zig)"
command -v zig >/dev/null || die "zig is not installed (cargo zigbuild links with it)"

commit_of() {
  local c
  c=$(git -C "$1" rev-parse HEAD 2>/dev/null || echo unknown)
  if [ -n "$(git -C "$1" status --porcelain --untracked-files=no 2>/dev/null)" ]; then c="$c-dirty"; fi
  echo "$c"
}
BENCH_COMMIT=$(commit_of "$BENCH_SRC")
if [ -n "${GATEWAY_BIN:-}" ]; then
  [ -f "$GATEWAY_BIN" ] || die "GATEWAY_BIN=$GATEWAY_BIN does not exist"
  GW_COMMIT="${GATEWAY_COMMIT:-unknown (prebuilt binary)}"
  GW_FROM="prebuilt"
else
  GW_COMMIT=$(commit_of "$GATEWAY_SRC")
  GW_FROM="built from source"
fi
echo "gateway: $GW_FROM, commit $GW_COMMIT"
echo "harness: commit $BENCH_COMMIT"

toolchain_of() { sed -n 's/^channel *= *"\(.*\)"/\1/p' "$1/rust-toolchain.toml" 2>/dev/null || true; }
for src in "$GATEWAY_SRC" "$BENCH_SRC"; do
  tc=$(toolchain_of "$src")
  if [ -n "$tc" ] && command -v rustup >/dev/null; then rustup target add "$TRIPLE" --toolchain "$tc" >/dev/null 2>&1 || true; fi
done

if [ -n "${GATEWAY_BIN:-}" ]; then
  # A Linux release binary built natively (the CI build job): jemalloc's C build does not cross-link with zig.
  echo "== using the prebuilt gateway $GATEWAY_BIN"
  GW_PATH="$GATEWAY_BIN"
else
  echo "== building the gateway (release, $TRIPLE.$GLIBC)"
  ( cd "$GATEWAY_SRC" && ANTIRION_BUILD_COMMIT="${GW_COMMIT%-dirty}" CARGO_TARGET_DIR="$TARGET_ROOT/gateway" \
      nice -n 10 cargo zigbuild --release --locked -p antirion --target "$TRIPLE.$GLIBC" )
  GW_PATH="$TARGET_ROOT/gateway/$TRIPLE/release/antirion"
fi
echo "== building the harness (release, $TRIPLE.$GLIBC)"
( cd "$BENCH_SRC/bench" && CARGO_TARGET_DIR="$TARGET_ROOT/bench" \
    nice -n 10 cargo zigbuild --release --locked --target "$TRIPLE.$GLIBC" \
      -p antirion-bench -p antirion-bench-mock )

echo "== building the harness for this machine (report rendering: $TARGET_ROOT/native/debug/antirion-bench)"
( cd "$BENCH_SRC/bench" && CARGO_TARGET_DIR="$TARGET_ROOT/native" nice -n 10 cargo build --locked -p antirion-bench -p antirion-bench-mock )

mkdir -p "$STAGE/ab/bin" "$STAGE/ab/bench"
cp "$GW_PATH" "$STAGE/ab/bin/antirion"
chmod 755 "$STAGE/ab/bin/antirion"
cp "$TARGET_ROOT/bench/$TRIPLE/release/antirion-bench" "$TARGET_ROOT/bench/$TRIPLE/release/antirion-bench-mock" "$STAGE/ab/bin/"
for d in matrix profiles topologies; do cp -R "$BENCH_SRC/bench/$d" "$STAGE/ab/bench/"; done
mkdir -p "$STAGE/ab/bench/corpus" && cp -R "$BENCH_SRC/bench/corpus/data" "$STAGE/ab/bench/corpus/"
echo "$GW_COMMIT" > "$STAGE/ab/COMMIT"
echo "$BENCH_COMMIT" > "$STAGE/ab/bench/COMMIT"
fresh_parts

# The binaries must load on Ubuntu 22.04+ (glibc 2.35): check the highest versioned symbol.
for b in antirion antirion-bench antirion-bench-mock; do
  max=$(strings "$STAGE/ab/bin/$b" | sed -n 's/^GLIBC_\([0-9][0-9.]*\)$/\1/p' | sort -t. -k1,1n -k2,2n | tail -1)
  echo "$b: needs glibc $max"
  if [ "$(printf '%s\n%s\n' "$max" "$GLIBC" | sort -t. -k1,1n -k2,2n | tail -1)" != "$GLIBC" ]; then
    die "$b needs glibc $max > $GLIBC"
  fi
done

pack
cat > "$OUT/ab-bundle.manifest.json" <<EOF
{
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "gateway": "$GW_FROM",
  "gateway_commit": "$GW_COMMIT",
  "bench_commit": "$BENCH_COMMIT",
  "sha256": {
    "antirion": "$(sha "$STAGE/ab/bin/antirion")",
    "antirion-bench": "$(sha "$STAGE/ab/bin/antirion-bench")",
    "antirion-bench-mock": "$(sha "$STAGE/ab/bin/antirion-bench-mock")",
    "bundle": "$(sha "$OUT/ab-bundle.tgz")"
  }
}
EOF
ls -la "$OUT/ab-bundle.tgz"
cat "$OUT/ab-bundle.manifest.json"
