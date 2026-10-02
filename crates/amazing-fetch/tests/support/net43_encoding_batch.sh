#!/usr/bin/env bash
# NET-43 pinned WPT encoding/XHR selection, against the existing resource shim.
set -euo pipefail
WPT="${1:?WPT root}"
BIN="$CARGO_TARGET_DIR/debug/amazing"
URL_FILE="$OUT/net43-shim-url.txt"
: > "$URL_FILE"
"$CARGO_TARGET_DIR/debug/examples/wpt_shim" --serve > "$URL_FILE" 2>/dev/null &
shim=$!
trap 'kill "$shim" 2>/dev/null || true' EXIT
for _ in $(seq 1 200); do [ -s "$URL_FILE" ] && break; sleep .05; done
base=$(cat "$URL_FILE")
[ -n "$base" ]
export AMAZING_WPT_SHIM_ADDR="${base#http://}"
export AMAZING_WPT_SHIM_ADDR="${AMAZING_WPT_SHIM_ADDR%/}"
export AMAZING_WPT_SHIM_ROOT="$WPT"
for file in encoding/unsupported-encodings.any.js encoding/big5-encoder.html encoding/legacy-mb-tchinese/big5/big5-enc-ascii.html encoding/legacy-mb-schinese/gbk/gbk-encoder.html encoding/legacy-mb-schinese/gb18030/gb18030-encoder.html encoding/iso-2022-jp-encoder.html; do
  echo "TEST $file"
  AMAZING_STORAGE_DIR="$OUT/net43-profiles/${file//\//_}" "$BIN" wpt-harness "$WPT" "$file" 800 600 15000 || true
done
