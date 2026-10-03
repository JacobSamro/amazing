#!/usr/bin/env bash
# Before input: six original layout consumer files from fixed LAYOUT42A/CSS41 base d65d636f.
# CSS38–41 dependencies remain unchanged; baseline lib does not include display_policy.
set -euo pipefail
RESULT=$OUT/layout43/final
NATIVE=/tmp/amazing-layout37-source
mkdir -p "$RESULT/candidate-src" "$NATIVE"
rsync -rlp --checksum --delete --exclude 'target*/' --exclude .git --exclude .agents --exclude corpora "$PWD/" "$NATIVE/"
ln -sfn "$HOME/corpora" "$NATIVE/corpora"
cd "$NATIVE"
export CARGO_TARGET_DIR=/tmp/amazing-layout36-target
bash scripts/ensure-wpt.sh
for file in box_tree.rs block.rs box_tree_ruby.rs containment.rs lib.rs table_tree.rs; do
 cp "crates/amazing-layout/src/$file" "$RESULT/candidate-src/$file"
 cp "$OUT/layout43/baseline/$file" "crates/amazing-layout/src/$file"
done
cargo build -p amazing --release --locked 2>&1 | tee "$RESULT/before-build.log"
sha256sum "$CARGO_TARGET_DIR/release/amazing" > "$RESULT/before-renderer.sha256"
for subdir in css/CSS2/floats css/CSS2/normal-flow css/css-tables css/css-sizing css/css-display css/css-contain css/css-overflow css/css-ruby css/CSS2/positioning css/css-position css/css-transforms css/css-fonts; do
 slug=${subdir//\//-}
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "$subdir" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/before-$slug" --jobs 4 --timeout 20 --compact 2>&1 | tee "$RESULT/before-$slug.log"
done
for file in box_tree.rs block.rs box_tree_ruby.rs containment.rs lib.rs table_tree.rs; do cp "$RESULT/candidate-src/$file" "crates/amazing-layout/src/$file"; done
sha256sum crates/amazing-layout/src/{box_tree,block,box_tree_ruby,containment,lib,table_tree,display_policy,display_policy_tests}.rs > "$RESULT/candidate-source.sha256"
cargo nextest run --workspace --locked --no-fail-fast 2>&1 | tee "$RESULT/nextest.log"
cargo test --workspace --locked --doc 2>&1 | tee "$RESULT/doctests.log"
python3 crates/amazing-layout/evidence/layout36/test_reftests.py 2>&1 | tee "$RESULT/harness.log"
cargo build -p amazing --release --locked 2>&1 | tee "$RESULT/after-build.log"
sha256sum "$CARGO_TARGET_DIR/release/amazing" > "$RESULT/after-renderer.sha256"
result=0
for subdir in css/CSS2/floats css/CSS2/normal-flow css/css-tables css/css-sizing css/css-display css/css-contain css/css-overflow css/css-ruby css/CSS2/positioning css/css-position css/css-transforms css/css-fonts; do
 slug=${subdir//\//-}
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "$subdir" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/after-$slug" --baseline "$RESULT/before-$slug/compact.json" --jobs 4 --timeout 20 --compact 2>&1 | tee "$RESULT/after-$slug.log" || result=1
done
exit "$result"
