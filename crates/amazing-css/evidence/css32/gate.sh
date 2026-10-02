#!/usr/bin/env bash
set -euo pipefail
export CARGO_TARGET_DIR=/tmp/amazing-target-css
bash scripts/ensure-wpt.sh
cargo nextest run --workspace --locked --no-fail-fast > "$OUT/css32-workspace.log" 2>&1
cargo test --workspace --locked --doc >> "$OUT/css32-workspace.log" 2>&1
cargo build -p amazing-css --example wpt_conformance --locked > "$OUT/css32-build.log" 2>&1
WPT_ROOT="$HOME/corpora/wpt" "$CARGO_TARGET_DIR/debug/examples/wpt_conformance" \
 css-logical css-backgrounds css-color css-values css-cascade css-conditional \
 css-fonts css-text css-transforms css-flexbox css-grid css-images > "$OUT/css32-final.txt"
cargo build -p amazing --bin amazing --locked >> "$OUT/css32-build.log" 2>&1
failed=0
for selection in backgrounds images; do
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" \
  --subdir "css/css-$selection" --exe "$CARGO_TARGET_DIR/debug/amazing" \
  --out "$OUT/css32-$selection-after" --width 800 --height 600 --timeout 20 \
  --jobs 4 --compact --baseline "$OUT/css32-$selection-before/compact.json" \
  > "$OUT/css32-$selection-after.log" 2>&1 || failed=1
 tail -5 "$OUT/css32-$selection-after.log"
done
tail -1 "$OUT/css32-final.txt"
exit "$failed"
