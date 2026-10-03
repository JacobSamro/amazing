#!/usr/bin/env bash
set -euo pipefail
mkdir -p /tmp/amazing-source-css
rsync -a --delete ./ /tmp/amazing-source-css/
cd /tmp/amazing-source-css
export CARGO_TARGET_DIR=/tmp/amazing-target-css
bash scripts/ensure-wpt.sh
{
  df -h "$HOME" /tmp
  du -sh /tmp/amazing-target-css "$OUT"/css49-{images,fonts,transforms}-{before,after} "$OUT"/css47a-fonts-after
  for folder in "$OUT"/css49-{images,fonts,transforms}-{before,after} "$OUT"/css47a-fonts-after; do
    find "$folder" -type f \( -name '*.ppm' -o -name '*.png' \) -delete
  done
  df -h "$HOME" /tmp
} > "$OUT/css50-disk-hygiene.log" 2>&1
cargo nextest run --workspace --locked --no-fail-fast > "$OUT/css50-workspace.log" 2>&1
cargo test --workspace --locked --doc >> "$OUT/css50-workspace.log" 2>&1
cargo build -p amazing-css --example wpt_conformance --locked > "$OUT/css50-build.log" 2>&1
WPT_ROOT="$HOME/corpora/wpt" "$CARGO_TARGET_DIR/debug/examples/wpt_conformance" css-logical css-backgrounds css-color css-values css-cascade css-conditional css-fonts css-text css-transforms css-flexbox css-grid css-images css-sizing css-contain css-display > "$OUT/css50-final.txt"
cargo build -p amazing --bin amazing --locked >> "$OUT/css50-build.log" 2>&1
render_rc=0
for dir in images fonts transforms; do
  python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "css/css-$dir" --exe "$CARGO_TARGET_DIR/debug/amazing" --out "$OUT/css50-$dir-after" --width 800 --height 600 --timeout 20 --jobs 4 --compact --baseline "crates/amazing-css/evidence/css50/before/$dir/compact.json" > "$OUT/css50-$dir-after.log" 2>&1 || render_rc=$?
  tail -5 "$OUT/css50-$dir-after.log"
done
sha256sum "$CARGO_TARGET_DIR/debug/amazing"
tail -1 "$OUT/css50-final.txt"
exit "$render_rc"
