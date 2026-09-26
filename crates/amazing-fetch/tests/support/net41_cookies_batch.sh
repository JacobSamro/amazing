#!/usr/bin/env bash
# NET-41 batch driver: run a candidate selection of single-page-navigable
# cookies/ WPT files through the installed wpt-harness at the pinned WPT
# revision, against the std-only WPT resource shim (AMAZING_WPT_SHIM_ADDR).
# Most of cookies/ (samesite/, partitioned-cookies/, third-party-cookies/,
# domain/ child-window flows) needs real multi-page/testdriver navigation
# this single-page harness cannot do; those are left out. This list is the
# subset that is at least a single top-level document exercising
# document.cookie and/or a Set-Cookie round trip through fetch/XHR -- some
# may still report "unsupported" if they turn out to need an iframe/child
# window after all, which the harness reports explicitly rather than hanging.
# Usage: net41_cookies_batch.sh <wpt-root> [timeout_ms]
set -u
WPT="${1:?usage: net41_cookies_batch.sh <wpt-root> [timeout_ms]}"
TMO="${2:-10000}"
TARGET="${CARGO_TARGET_DIR:-$HOME/amazing/target/net}"
BIN="$TARGET/debug/amazing"
OUT="${OUT:-$HOME/amazing/out}"
mkdir -p "$OUT"

cargo build -q -p amazing-net --example wpt_shim --locked
cargo build -q -p amazing --locked

SHIM_URL_FILE="$OUT/net41-cookies-shim-url.txt"
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
cookies/attributes/expires.html
cookies/attributes/invalid.html
cookies/attributes/max-age.html
cookies/attributes/path.html
cookies/attributes/secure-non-secure.html
cookies/attributes/secure.https.html
cookies/attributes/attributes-ctl.sub.html
cookies/attributes/domain.sub.html
cookies/encoding/charset.html
cookies/meta-blocked.html
cookies/name/name.html
cookies/name/name-ctl.html
cookies/navigated-away.html
cookies/path/default.html
cookies/path/match.html
cookies/size/name-and-value.html
cookies/size/attributes.www.sub.html
cookies/value/value.html
cookies/value/value-ctl.html
cookies/domain/domain-attribute-missing.sub.html
cookies/domain/domain-attribute-matches-host.sub.https.html
cookies/origin-bound-cookies/port-bound-cookies.html
cookies/cookie-enabled-noncookie-frame.html
)
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
echo "NET41-COOKIES-WPT-DONE"
