#!/usr/bin/env bash
# Run ON a Boat sandbox (inside a linux-run.sh command): make sure the pinned WPT
# checkout exists at ~/corpora/wpt. Sandboxes created from the amazing-builder
# template after 2026-09-26 already have it; older pooled sandboxes fetch it once
# (about 1.1 GB, shallow). Prints the checkout path.
#   scripts/linux-run.sh <lane> -- 'bash scripts/ensure-wpt.sh && <your WPT command>'
set -euo pipefail
REV=c2a2fce69e5c2509d9dc28440ec8b42532e5c293
DIR="$HOME/corpora/wpt"
if [ "$(git -C "$DIR" rev-parse HEAD 2>/dev/null)" != "$REV" ]; then
  mkdir -p "$DIR"; cd "$DIR"
  [ -d .git ] || { git init -q; git remote add origin https://github.com/web-platform-tests/wpt.git; }
  git fetch -q --depth 1 origin "$REV"; git checkout -q --force FETCH_HEAD
fi
echo "$DIR"
