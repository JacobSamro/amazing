#!/usr/bin/env bash
set -euo pipefail
mkdir -p /tmp/amazing-source-css
rsync -a --delete ./ /tmp/amazing-source-css/
cd /tmp/amazing-source-css
export CARGO_TARGET_DIR=/tmp/amazing-target-css
bash scripts/ensure-wpt.sh
cargo nextest run --workspace --locked --no-fail-fast > "$OUT/css44-workspace.log" 2>&1
cargo test --workspace --locked --doc >> "$OUT/css44-workspace.log" 2>&1
cargo build -p amazing-css --example wpt_conformance --locked > "$OUT/css44-build.log" 2>&1
WPT_ROOT="$HOME/corpora/wpt" "$CARGO_TARGET_DIR/debug/examples/wpt_conformance" css-logical css-backgrounds css-color css-values css-cascade css-conditional css-fonts css-text css-transforms css-flexbox css-grid css-images css-sizing css-contain css-display > "$OUT/css44-final.txt"
cargo build -p amazing --bin amazing --locked >> "$OUT/css44-build.log" 2>&1
for area in images transforms; do
    python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "css/css-$area" --exe "$CARGO_TARGET_DIR/debug/amazing" --out "$OUT/css44-$area-after" --width 800 --height 600 --timeout 20 --jobs 4 --compact --baseline "crates/amazing-css/evidence/css44/before/$area/compact.json" > "$OUT/css44-$area-after.log" 2>&1
    tail -5 "$OUT/css44-$area-after.log"
done
sha256sum "$CARGO_TARGET_DIR/debug/amazing"
tail -1 "$OUT/css44-final.txt"
