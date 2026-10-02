#!/usr/bin/env bash
set -euo pipefail
mkdir -p /tmp/amazing-source-css
rsync -a --delete ./ /tmp/amazing-source-css/
cd /tmp/amazing-source-css
export CARGO_TARGET_DIR=/tmp/amazing-target-css
bash scripts/ensure-wpt.sh
cargo nextest run --workspace --locked --no-fail-fast > "$OUT/css37-workspace.log" 2>&1
cargo test --workspace --locked --doc >> "$OUT/css37-workspace.log" 2>&1
cargo build -p amazing-css --example wpt_conformance --locked > "$OUT/css37-build.log" 2>&1
WPT_ROOT="$HOME/corpora/wpt" "$CARGO_TARGET_DIR/debug/examples/wpt_conformance" \
 css-logical css-backgrounds css-color css-values css-cascade css-conditional \
 css-fonts css-text css-transforms css-flexbox css-grid css-images css-sizing css-contain css-display > "$OUT/css37-after.txt"
cargo build -p amazing --bin amazing --locked >> "$OUT/css37-build.log" 2>&1
rc=0
for subdir in css-display css-tables css-sizing css-contain; do
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" \
 --subdir "css/$subdir" --exe "$CARGO_TARGET_DIR/debug/amazing" \
 --out "$OUT/css37-after-$subdir" --width 800 --height 600 --timeout 20 \
 --jobs 4 --compact --baseline "$OUT/css37-before-$subdir/compact.json" > "$OUT/css37-after-$subdir.log" 2>&1 || rc=1
 tail -5 "$OUT/css37-after-$subdir.log"
done
tail -1 "$OUT/css37-after.txt"
exit "$rc"
