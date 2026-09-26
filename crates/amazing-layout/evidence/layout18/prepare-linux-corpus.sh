#!/usr/bin/env bash
set -euo pipefail
WPT_DIR="$HOME/corpora/wpt-layout"
REV=c2a2fce69e5c2509d9dc28440ec8b42532e5c293
if [ ! -d "$WPT_DIR/.git" ]; then
  git init "$WPT_DIR"
  git -C "$WPT_DIR" remote add origin https://github.com/web-platform-tests/wpt.git
fi
git -C "$WPT_DIR" sparse-checkout init --cone
git -C "$WPT_DIR" sparse-checkout set css fonts images resources common
if [ "$(git -C "$WPT_DIR" rev-parse HEAD 2>/dev/null || true)" = "$REV" ]; then exit 0; fi
git -C "$WPT_DIR" -c protocol.version=2 fetch --depth 1 --filter=blob:none origin "$REV"
git -C "$WPT_DIR" checkout --detach "$REV"
test "$(git -C "$WPT_DIR" rev-parse HEAD)" = "$REV"
