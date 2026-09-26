#!/usr/bin/env bash
# Run only through scripts/linux-run.sh layout.
set -euo pipefail
TAG=${1:?unique output tag}
SOURCE=${2:?source checkpoint}
E=crates/amazing-layout/evidence
RUN="$OUT/layout20/$TAG"
mkdir -p "$RUN"
test ! -e "$RUN/amazing"
printf 'source=%s\ntarget=%s\n' "$SOURCE" "$CARGO_TARGET_DIR" > "$RUN/provenance.txt"
sha256sum crates/amazing-layout/src/{table,table_tree,table_tree_tests,box_tree,box_tree_table,block,inline,lib}.rs Cargo.lock >> "$RUN/provenance.txt"
cargo build -p amazing --release --locked
cp "$CARGO_TARGET_DIR/release/amazing" "$RUN/amazing"
status=0
cargo nextest run -p amazing-layout --locked --no-fail-fast || status=$?
cargo test -p amazing-layout --locked --doc || status=$?
python3 "$E/layout18/render-linux.py" --manifest "$E/layout19/manifest.json" \
  --wpt "$HOME/corpora/wpt-layout" --binary "$RUN/amazing" --source "$SOURCE" \
  --output "$RUN/tables" --compress-images
python3 "$E/layout18/compare.py" "$OUT/layout19/final-after/tables/report.json" \
  "$RUN/tables/report.json" "$RUN/comparison.json"
python3 "$E/layout19/triage.py" "$RUN/tables/report.json" "$RUN/triage.json"
exit "$status"
