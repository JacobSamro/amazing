#!/usr/bin/env bash
set -euo pipefail
NATIVE=/tmp/amazing-layout37-source
RESULT=$OUT/layout46/final
mkdir -p "$NATIVE" "$RESULT/candidate-src"
rsync -rlp --checksum --delete --exclude 'target*/' --exclude .git --exclude .agents --exclude corpora "$PWD/" "$NATIVE/"
ln -sfn "$HOME/corpora" "$NATIVE/corpora"
cd "$NATIVE"
export CARGO_TARGET_DIR=/tmp/amazing-layout36-target
bash scripts/ensure-wpt.sh
files=( crates/amazing-paint/src/background.rs crates/amazing-paint/src/display_list_delta.rs crates/amazing-paint/src/display_list_view.rs crates/amazing-paint/src/lib.rs crates/amazing-paint/src/paint.rs crates/amazing-paint/src/retained_paint.rs crates/amazing/src/page.rs )
for file in "${files[@]}"; do mkdir -p "$RESULT/candidate-src/$(dirname "$file")"; cp "$file" "$RESULT/candidate-src/$file"; done
restore() { for file in "${files[@]}"; do cp "$RESULT/candidate-src/$file" "$file"; done; }
trap restore EXIT
for file in "${files[@]}"; do cp "$OUT/layout46/baseline/$file" "$file"; done
sha256sum "${files[@]}" > "$RESULT/before-source.sha256"
cargo build -p amazing --release --locked > "$RESULT/before-build.log" 2>&1
sha256sum "$CARGO_TARGET_DIR/release/amazing" > "$RESULT/before-renderer.sha256"
for subdir in css/CSS2/floats css/CSS2/normal-flow css/css-tables css/css-sizing css/css-display css/css-contain css/css-overflow css/css-ruby css/CSS2/positioning css/css-position css/css-transforms css/css-fonts css/css-images css/css-backgrounds css/css-viewport; do
 slug=${subdir//\//-}
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "$subdir" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/before-$slug" --jobs 4 --timeout 20 --compact > "$RESULT/before-$slug.log" 2>&1
done
python3 crates/amazing-layout/evidence/layout46/resources.py --wpt-root "$HOME/corpora/wpt" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/before-resources" > "$RESULT/before-resources.log" 2>&1
restore
sha256sum "${files[@]}" crates/amazing-paint/src/border_image{,_tests}.rs crates/amazing/src/page_border_images.rs > "$RESULT/candidate-source.sha256"
cargo nextest run --workspace --locked --no-fail-fast > "$RESULT/nextest.log" 2>&1
cargo test --workspace --locked --doc > "$RESULT/doctests.log" 2>&1
python3 crates/amazing-layout/evidence/layout36/test_reftests.py > "$RESULT/harness.log" 2>&1
cargo build -p amazing --release --locked > "$RESULT/after-build.log" 2>&1
sha256sum "$CARGO_TARGET_DIR/release/amazing" > "$RESULT/after-renderer.sha256"
result=0
for subdir in css/CSS2/floats css/CSS2/normal-flow css/css-tables css/css-sizing css/css-display css/css-contain css/css-overflow css/css-ruby css/CSS2/positioning css/css-position css/css-transforms css/css-fonts css/css-images css/css-backgrounds css/css-viewport; do
 slug=${subdir//\//-}
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "$subdir" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/after-$slug" --baseline "$RESULT/before-$slug/compact.json" --jobs 4 --timeout 20 --compact > "$RESULT/after-$slug.log" 2>&1 || result=1
done
python3 crates/amazing-layout/evidence/layout46/resources.py --wpt-root "$HOME/corpora/wpt" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/after-resources" --baseline "$RESULT/before-resources/compact.json" > "$RESULT/after-resources.log" 2>&1 || result=1
printf 'LAYOUT46 final exit=%s\n' "$result"
exit "$result"
