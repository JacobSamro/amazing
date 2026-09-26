#!/usr/bin/env bash
# LAYOUT-19 final-after is the exact before source for LAYOUT-20.
set -euo pipefail
SOURCE=${1:?source checkpoint}
E=crates/amazing-layout/evidence
RUN="$OUT/layout20/final-after"
mkdir -p "$RUN"
test ! -e "$RUN/amazing"
printf 'source=%s\ntarget=%s\n' "$SOURCE" "$CARGO_TARGET_DIR" > "$RUN/provenance.txt"
sha256sum crates/amazing-layout/src/{table,table_tree,table_tree_tests,box_tree,box_tree_table,block,inline,lib}.rs Cargo.lock >> "$RUN/provenance.txt"
cargo build -p amazing --release --locked
cp "$CARGO_TARGET_DIR/release/amazing" "$RUN/amazing"
cargo nextest run --workspace --locked --no-fail-fast
cargo test --workspace --locked --doc
python3 "$E/layout18/validate-linux-runner.py"
for corpus in tables retained; do
  if [ "$corpus" = tables ]; then MANIFEST="$E/layout19/manifest.json"
  else MANIFEST="$E/layout16/layout16-full-manifest.json"; fi
  python3 "$E/layout18/render-linux.py" --manifest "$MANIFEST" \
    --wpt "$HOME/corpora/wpt-layout" --binary "$RUN/amazing" --source "$SOURCE" \
    --output "$RUN/$corpus" --compress-images
  python3 "$E/layout18/compare.py" "$OUT/layout19/final-after/$corpus/report.json" \
    "$RUN/$corpus/report.json" "$RUN/$corpus-comparison.json"
done
python3 "$E/layout19/triage.py" "$RUN/tables/report.json" "$RUN/triage.json"
