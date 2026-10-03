#!/usr/bin/env bash
set -euo pipefail
NATIVE=/tmp/amazing-layout37-source
RESULT=$OUT/layout47/iterate-12
mkdir -p "$NATIVE" "$RESULT"
rsync -rlp --checksum --delete --exclude 'target*/' --exclude .git --exclude .agents --exclude corpora "$PWD/" "$NATIVE/"
ln -sfn "$HOME/corpora" "$NATIVE/corpora"
cd "$NATIVE"
export CARGO_TARGET_DIR=/tmp/amazing-layout36-target
bash scripts/ensure-wpt.sh
cargo build -p amazing-layout --locked 2>&1 | tee "$RESULT/crate-build.log"
cargo nextest run -p amazing-layout --locked --no-fail-fast 2>&1 | tee "$RESULT/nextest.log"
cargo test -p amazing-layout --locked --doc 2>&1 | tee "$RESULT/doctests.log"
cargo build -p amazing --release --locked 2>&1 | tee "$RESULT/build.log"
result=0
for subdir in css/CSS2/floats css/CSS2/normal-flow css/css-tables css/css-sizing css/css-display css/css-contain css/css-overflow css/css-ruby css/CSS2/positioning css/css-position css/css-transforms css/css-fonts css/css-images css/css-backgrounds css/css-viewport; do
 slug=${subdir//\//-}
 python3 scripts/wpt_render_compare.py --wpt-root "$HOME/corpora/wpt" --subdir "$subdir" --exe "$CARGO_TARGET_DIR/release/amazing" --out "$RESULT/$slug" --baseline "crates/amazing-layout/evidence/layout46/after-$slug.json" --jobs 4 --timeout 20 --compact 2>&1 | tee "$RESULT/$slug.log" || result=1
done

exit "$result"
