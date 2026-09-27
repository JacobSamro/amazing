#!/usr/bin/env bash
# NET-41/42 batch driver: run a candidate selection of single-page-navigable
# cookies/ WPT files through the installed wpt-harness at the pinned WPT
# revision. Most of cookies/ (samesite/, partitioned-cookies/,
# third-party-cookies/, domain/ child-window flows) needs real
# multi-page/testdriver navigation this single-page harness cannot do; those
# are left out. This list is the subset that is at least a single top-level
# document exercising document.cookie and/or a Set-Cookie round trip through
# fetch/XHR -- some may still report "unsupported" if they turn out to need
# an iframe/child window after all, which the harness reports explicitly
# rather than hanging.
#
# NET-42: deliberately does NOT set AMAZING_WPT_SHIM_ADDR/AMAZING_WPT_SHIM_ROOT
# (unlike net26/net36/net37's batch scripts). Setting either one makes
# wpt_harness.rs::execute() switch the document to `http://web-platform.test/
# <test>` and route every relative fetch() through dispatch.rs's
# web-platform.test DNS override to that shim -- and the shim started here
# (amazing-net's `wpt_shim` example, i.e. `WptServer`, a fetch/api+xhr+
# websocket resource double) has no route for `/cookies/resources/cookie.py`
# (that's `respond_cookie_py`, implemented only inside wpt_harness.rs's own
# `serve_fixture`). The result was a silent stall on every fetch() to
# cookie.py: 17 of 23 files timed out with harness_status:2 and an empty
# subtests array (the connection was accepted but never answered, so neither
# testharness.js's own per-test timeout nor the outer harness supervisor saw
# anything to report). Leaving these two variables unset makes execute() use
# its normal FixtureServer/serve_fixture loopback path instead, which serves
# cookie.py, testdriver-vendor.js and everything else this corpus needs
# directly -- confirmed by running these files directly with no env vars set
# and getting real pass/fail data in well under a second each.
# Usage: net41_cookies_batch.sh <wpt-root> [timeout_ms]
set -u
WPT="${1:?usage: net41_cookies_batch.sh <wpt-root> [timeout_ms]}"
TMO="${2:-10000}"
TARGET="${CARGO_TARGET_DIR:-$HOME/amazing/target/net}"
BIN="$TARGET/debug/amazing"
OUT="${OUT:-$HOME/amazing/out}"
mkdir -p "$OUT"

cargo build -q -p amazing --locked
unset AMAZING_WPT_SHIM_ADDR AMAZING_WPT_SHIM_ROOT

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
