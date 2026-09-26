#!/usr/bin/env bash
# NET-37 batch driver: run the widened selection of pinned fetch/api + xhr
# WPT files through the installed wpt-harness at WPT revision c2a2fce,
# against the std-only WPT resource shim (AMAZING_WPT_SHIM_ADDR). Supersedes
# net26_wpt_batch.sh's 38-file list with 24 additional files from
# fetch/api/{headers,response,basic,request}/ that pass cleanly against the
# runner as it stands today (62 files total). See PROVENANCE notes in
# report.md/questions.md for the many more files tried that are currently
# blocked on cross-lane gaps (String.prototype.split(regexp), ReadableStream,
# Proxy, ArrayBuffer/DataView edge cases, Request read-only attributes) --
# those are left out of this corpus so a run here stays green end to end.
# Usage: net37_wpt_batch.sh <wpt-root> [timeout_ms]
set -u
WPT="${1:?usage: net37_wpt_batch.sh <wpt-root> [timeout_ms]}"
TMO="${2:-10000}"
TARGET="${CARGO_TARGET_DIR:-$HOME/amazing/target/net}"
BIN="$TARGET/debug/amazing"
OUT="${OUT:-$HOME/amazing/out}"
mkdir -p "$OUT"

cargo build -q -p amazing-net --example wpt_shim --locked
cargo build -q -p amazing --locked

SHIM_URL_FILE="$OUT/net37-shim-url.txt"
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

FILES=(
# --- original NET-26 selection (38 files) ---
fetch/api/headers/headers-basic.any.js
fetch/api/headers/headers-casing.any.js
fetch/api/headers/headers-combine.any.js
fetch/api/headers/headers-no-cors.any.js
fetch/api/headers/headers-normalize.any.js
fetch/api/headers/header-values-normalize.any.js
fetch/api/response/response-init-001.any.js
fetch/api/response/response-consume-empty.any.js
fetch/api/basic/request-headers.any.js
fetch/api/headers/header-setcookie.any.js
fetch/api/redirect/redirect-mode.any.js
fetch/api/cors/cors-basic.any.js
fetch/api/cors/cors-preflight.any.js
fetch/api/credentials/cookies.any.js
fetch/api/abort/general.any.js
xhr/event-load.any.js
xhr/event-progress.any.js
xhr/event-timeout.any.js
xhr/event-readystatechange-loaded.any.js
xhr/responsetype.any.js
xhr/getresponseheader.any.js
xhr/getallresponseheaders.htm
xhr/getallresponseheaders-cookies.htm
xhr/status-basic.htm
xhr/send-redirect.htm
xhr/getresponseheader-chunked-trailer.htm
xhr/setrequestheader-before-open.htm
xhr/setrequestheader-after-send.htm
xhr/setrequestheader-open-setrequestheader.htm
xhr/setrequestheader-bogus-name.htm
xhr/setrequestheader-bogus-value.htm
xhr/setrequestheader-allow-empty-value.htm
xhr/setrequestheader-allow-whitespace-in-value.htm
xhr/setrequestheader-case-insensitive.htm
xhr/setrequestheader-combining.window.js
xhr/setrequestheader-content-type.htm
xhr/setrequestheader-header-allowed.htm
xhr/setrequestheader-header-forbidden.htm
# --- NET-37 additions (24 files, all-pass as of this measurement) ---
fetch/api/headers/headers-errors.any.js
fetch/api/headers/headers-forbidden-override.any.js
fetch/api/headers/headers-structure.any.js
fetch/api/headers/header-values.any.js
fetch/api/response/json.any.js
fetch/api/response/response-error.any.js
fetch/api/response/response-static-error.any.js
fetch/api/response/response-static-redirect.any.js
fetch/api/basic/header-value-combining.any.js
fetch/api/basic/historical.any.js
fetch/api/basic/mode-same-origin.any.js
fetch/api/basic/request-head.any.js
fetch/api/basic/request-headers-nonascii.any.js
fetch/api/basic/scheme-about.any.js
fetch/api/basic/scheme-others.sub.any.js
fetch/api/basic/stream-safe-creation.any.js
fetch/api/basic/text-utf8.any.js
fetch/api/request/forbidden-method.any.js
fetch/api/request/request-bad-port.any.js
fetch/api/request/request-consume.any.js
fetch/api/request/request-error.any.js
fetch/api/request/request-init-002.any.js
fetch/api/request/request-init-priority.any.js
)
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
echo "NET37-WPT-DONE"
