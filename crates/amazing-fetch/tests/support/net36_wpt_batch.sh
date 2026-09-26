#!/usr/bin/env bash
# NET-36 batch driver: run the headless-capable (`.any.js`) top-level
# websockets/ WPT files through the installed wpt-harness at the pinned WPT
# revision, against the std-only WPT resource shim's RFC 6455 upgrade
# support (AMAZING_WPT_SHIM_ADDR). Excludes .window.js/back-forward-cache and
# nested interfaces/ tests, which need real browser navigation.
# Usage: net36_wpt_batch.sh <wpt-root> [timeout_ms]
set -u
WPT="${1:?usage: net36_wpt_batch.sh <wpt-root> [timeout_ms]}"
TMO="${2:-10000}"
TARGET="${CARGO_TARGET_DIR:-$HOME/amazing/target/net}"
BIN="$TARGET/debug/amazing"
OUT="${OUT:-$HOME/amazing/out}"
mkdir -p "$OUT"

cargo build -q -p amazing-net --example wpt_shim --locked
cargo build -q -p amazing --locked

SHIM_URL_FILE="$OUT/net36-shim-url.txt"
rm -f "$SHIM_URL_FILE"
"$TARGET/debug/examples/wpt_shim" --serve >"$SHIM_URL_FILE" 2>/dev/null &
SHIM_PID=$!
trap 'kill "$SHIM_PID" 2>/dev/null' EXIT
for _ in $(seq 1 200); do [ -s "$SHIM_URL_FILE" ] && break; sleep 0.05; done
base=$(cat "$SHIM_URL_FILE" 2>/dev/null)
if [ -z "$base" ]; then echo "SHIM-FAILED"; exit 3; fi
export AMAZING_WPT_SHIM_ADDR="${base#http://}"
AMAZING_WPT_SHIM_ADDR="${AMAZING_WPT_SHIM_ADDR%/}"
export AMAZING_WPT_SHIM_ADDR
export AMAZING_WPT_SHIM_ROOT="$WPT"
echo "SHIM $AMAZING_WPT_SHIM_ADDR root=$AMAZING_WPT_SHIM_ROOT"

mapfile -t FILES < <(cd "$WPT" && find websockets -maxdepth 1 -name '*.any.js' | sed 's#^\./##' | sort)
echo "FILE_COUNT ${#FILES[@]}"
PROFILE_ROOT="${PROFILE_ROOT:-$OUT/wpt-profiles}"
mkdir -p "$PROFILE_ROOT"
for t in "${FILES[@]}"; do
  echo "TEST $t"
  # A fresh per-test storage profile: the harness otherwise reuses a shared
  # profile and a stale/corrupt IndexedDB aborts the whole run.
  export AMAZING_STORAGE_DIR="$PROFILE_ROOT/${t//\//_}"
  rm -rf "$AMAZING_STORAGE_DIR"
  "$BIN" wpt-harness "$WPT" "$t" 800 600 "$TMO"
  echo "EXIT $?"
done
echo "NET36-WPT-DONE"
