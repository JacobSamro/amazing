#!/usr/bin/env bash
# Before input: two original layout files from fixed LAYOUT-39 base 12cf44b4,
# copied to $OUT/layout40/baseline; CSS dependencies remain unchanged.
set -euo pipefail
RESULT=$OUT/layout40/final
NATIVE=/tmp/amazing-layout37-source
mkdir -p "$RESULT/candidate-src" "$NATIVE"
rsync -rlp --checksum --delete --exclude 'target*/' --exclude .git --exclude .agents --exclude corpora "$PWD/" "$NATIVE/"
ln -sfn "$HOME/corpora" "$NATIVE/corpora"
cd "$NATIVE"
export CARGO_TARGET_DIR=/tmp/amazing-layout36-target
bash scripts/ensure-wpt.sh
for file in lib.rs position.rs; do
 cp "crates/amazing-layout/src/$file" "$RESULT/candidate-src/$file"
 cp "$OUT/layout40/baseline/$file" "crates/amazing-layout/src/$file"
done
cargo build -p amazing --release --locked 2>&1 | tee "$RESULT/before-build.log"
sha256sum "$CARGO_TARGET_DIR/release/amazing" > "$RESULT/before-renderer.sha256"
for subdir in css/CSS2/floats css/CSS2/normal-flow css/css-tables css/css-sizing css/css-display css/css-contain css/css-overflow; do
 slug=${subdir//\//-}
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "$subdir" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/before-$slug" --jobs 4 --timeout 20 --compact 2>&1 | tee "$RESULT/before-$slug.log"
done
for file in lib.rs position.rs; do cp "$RESULT/candidate-src/$file" "crates/amazing-layout/src/$file"; done
sha256sum crates/amazing-layout/src/{lib,position}.rs > "$RESULT/candidate-source.sha256"
cargo nextest run --workspace --locked --no-fail-fast 2>&1 | tee "$RESULT/nextest.log"
cargo test --workspace --locked --doc 2>&1 | tee "$RESULT/doctests.log"
python3 crates/amazing-layout/evidence/layout36/test_reftests.py 2>&1 | tee "$RESULT/harness.log"
cargo build -p amazing --release --locked 2>&1 | tee "$RESULT/after-build.log"
sha256sum "$CARGO_TARGET_DIR/release/amazing" > "$RESULT/after-renderer.sha256"
result=0
for subdir in css/CSS2/floats css/CSS2/normal-flow css/css-tables css/css-sizing css/css-display css/css-contain css/css-overflow; do
 slug=${subdir//\//-}
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "$subdir" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/after-$slug" --baseline "$RESULT/before-$slug/compact.json" --jobs 4 --timeout 20 --compact 2>&1 | tee "$RESULT/after-$slug.log" || result=1
done
exit "$result"
