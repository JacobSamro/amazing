#!/usr/bin/env bash
set -euo pipefail
mkdir -p /tmp/amazing-source-css
rsync -a --delete ./ /tmp/amazing-source-css/
cd /tmp/amazing-source-css
export CARGO_TARGET_DIR=/tmp/amazing-target-css
bash scripts/ensure-wpt.sh
cargo nextest run --workspace --locked --no-fail-fast > "$OUT/css42-workspace.log" 2>&1
cargo test --workspace --locked --doc >> "$OUT/css42-workspace.log" 2>&1
cargo build -p amazing-css --example wpt_conformance --locked > "$OUT/css42-build.log" 2>&1
WPT_ROOT="$HOME/corpora/wpt" "$CARGO_TARGET_DIR/debug/examples/wpt_conformance" css-logical css-backgrounds css-color css-values css-cascade css-conditional css-fonts css-text css-transforms css-flexbox css-grid css-images css-sizing css-contain css-display > "$OUT/css42-final.txt"
cargo build -p amazing --bin amazing --locked >> "$OUT/css42-build.log" 2>&1
python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir css/css-backgrounds --exe "$CARGO_TARGET_DIR/debug/amazing" --out "$OUT/css42-backgrounds-after" --width 800 --height 600 --timeout 20 --jobs 4 --compact --baseline crates/amazing-css/evidence/css42/before/compact.json > "$OUT/css42-backgrounds-after.log" 2>&1
tail -5 "$OUT/css42-backgrounds-after.log"
tail -1 "$OUT/css42-final.txt"
