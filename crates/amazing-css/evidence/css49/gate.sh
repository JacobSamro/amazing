#!/usr/bin/env bash
set -euo pipefail
mkdir -p /tmp/amazing-source-css
rsync -a --delete ./ /tmp/amazing-source-css/
cd /tmp/amazing-source-css
export CARGO_TARGET_DIR=/tmp/amazing-target-css
bash scripts/ensure-wpt.sh
cargo nextest run --workspace --locked --no-fail-fast > "$OUT/css49-workspace.log" 2>&1
cargo test --workspace --locked --doc >> "$OUT/css49-workspace.log" 2>&1
cargo build -p amazing-css --example wpt_conformance --locked > "$OUT/css49-build.log" 2>&1
WPT_ROOT="$HOME/corpora/wpt" "$CARGO_TARGET_DIR/debug/examples/wpt_conformance" css-logical css-backgrounds css-color css-values css-cascade css-conditional css-fonts css-text css-transforms css-flexbox css-grid css-images css-sizing css-contain css-display > "$OUT/css49-final.txt"
cargo build -p amazing --bin amazing --locked >> "$OUT/css49-build.log" 2>&1
render_rc=0
for dir in images fonts transforms; do
  python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "css/css-$dir" --exe "$CARGO_TARGET_DIR/debug/amazing" --out "$OUT/css49-$dir-after" --width 800 --height 600 --timeout 20 --jobs 4 --compact --baseline "crates/amazing-css/evidence/css49/before/$dir/compact.json" > "$OUT/css49-$dir-after.log" 2>&1 || render_rc=$?
  tail -5 "$OUT/css49-$dir-after.log"
done
sha256sum "$CARGO_TARGET_DIR/debug/amazing"
tail -1 "$OUT/css49-final.txt"
exit "$render_rc"
