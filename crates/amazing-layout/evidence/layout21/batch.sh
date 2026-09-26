#!/usr/bin/env bash
# Only invoke through scripts/linux-run.sh layout.
set -euo pipefail
TAG=${1:?unique tag}
SOURCE=${2:?source checkpoint}
E=crates/amazing-layout/evidence
RUN="$OUT/layout21/$TAG"
mkdir -p "$RUN"
test ! -e "$RUN/amazing"
printf 'source=%s\ntarget=%s\n' "$SOURCE" "$CARGO_TARGET_DIR" > "$RUN/provenance.txt"
sha256sum crates/amazing-layout/src/{block,box_tree,box_tree_ruby,inline,inline_ruby,lib,ruby_tests}.rs Cargo.lock >> "$RUN/provenance.txt"
cargo build -p amazing --release --locked
cp "$CARGO_TARGET_DIR/release/amazing" "$RUN/amazing"
status=0
cargo nextest run -p amazing-layout --locked --no-fail-fast || status=$?
cargo test -p amazing-layout --locked --doc || status=$?
python3 "$E/layout18/render-linux.py" --manifest "$E/layout21/manifest.json" \
  --wpt "$HOME/corpora/wpt-layout" --binary "$RUN/amazing" --source "$SOURCE" \
  --output "$RUN/ruby" --compress-images
python3 "$E/layout18/compare.py" "$OUT/layout21/before/report.json" \
  "$RUN/ruby/report.json" "$RUN/comparison.json"
exit "$status"
