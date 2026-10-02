#!/usr/bin/env bash
set -euo pipefail
mkdir -p /tmp/amazing-source-css
rsync -a --delete ./ /tmp/amazing-source-css/
cd /tmp/amazing-source-css
export CARGO_TARGET_DIR=/tmp/amazing-target-css
bash scripts/ensure-wpt.sh
cargo nextest run --workspace --locked --no-fail-fast > "$OUT/css36-workspace.log" 2>&1
cargo test --workspace --locked --doc >> "$OUT/css36-workspace.log" 2>&1
cargo build -p amazing-css --example wpt_conformance --locked > "$OUT/css36-build.log" 2>&1
WPT_ROOT="$HOME/corpora/wpt" "$CARGO_TARGET_DIR/debug/examples/wpt_conformance" \
 css-logical css-backgrounds css-color css-values css-cascade css-conditional \
 css-fonts css-text css-transforms css-flexbox css-grid css-images css-sizing css-contain > "$OUT/css36-final.txt"
cargo build -p amazing --bin amazing --locked >> "$OUT/css36-build.log" 2>&1
python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" \
 --subdir css/css-fonts --exe "$CARGO_TARGET_DIR/debug/amazing" \
 --out "$OUT/css36-fonts-after" --width 800 --height 600 --timeout 20 \
 --jobs 4 --compact --baseline "crates/amazing-css/evidence/css36/before/compact.json" > "$OUT/css36-fonts-after.log" 2>&1
tail -5 "$OUT/css36-fonts-after.log"
tail -1 "$OUT/css36-final.txt"
