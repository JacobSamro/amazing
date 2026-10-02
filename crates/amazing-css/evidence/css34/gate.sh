#!/usr/bin/env bash
set -euo pipefail
mkdir -p /tmp/amazing-source-css
rsync -a --delete ./ /tmp/amazing-source-css/
cd /tmp/amazing-source-css
export CARGO_TARGET_DIR=/tmp/amazing-target-css
bash scripts/ensure-wpt.sh
cargo nextest run --workspace --locked --no-fail-fast > "$OUT/css34-workspace.log" 2>&1
cargo test --workspace --locked --doc >> "$OUT/css34-workspace.log" 2>&1
cargo build -p amazing-css --example wpt_conformance --locked > "$OUT/css34-build.log" 2>&1
WPT_ROOT="$HOME/corpora/wpt" "$CARGO_TARGET_DIR/debug/examples/wpt_conformance" \
 css-logical css-backgrounds css-color css-values css-cascade css-conditional \
 css-fonts css-text css-transforms css-flexbox css-grid css-images css-sizing css-contain > "$OUT/css34-final.txt"
cargo build -p amazing --bin amazing --locked >> "$OUT/css34-build.log" 2>&1
python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" \
 --subdir css/css-transforms --exe "$CARGO_TARGET_DIR/debug/amazing" \
 --out "$OUT/css34-transforms-after" --width 800 --height 600 --timeout 20 \
 --jobs 4 --compact --baseline "$OUT/css34-transforms-before/compact.json" > "$OUT/css34-transforms-after.log" 2>&1
tail -5 "$OUT/css34-transforms-after.log"
tail -1 "$OUT/css34-final.txt"
