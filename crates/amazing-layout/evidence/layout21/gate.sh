#!/usr/bin/env bash
# Linux final gate on one fixed source; baseline is LAYOUT-20 final.
set -euo pipefail
SOURCE=${1:?source checkpoint}
E=crates/amazing-layout/evidence
RUN="$OUT/layout21/${2:-final-after}"
mkdir -p "$RUN"
test ! -e "$RUN/amazing"
printf 'source=%s\ntarget=%s\n' "$SOURCE" "$CARGO_TARGET_DIR" > "$RUN/provenance.txt"
sha256sum crates/amazing-layout/src/{block,box_tree,box_tree_ruby,inline,inline_ruby,lib,ruby_tests}.rs Cargo.lock >> "$RUN/provenance.txt"
cargo build -p amazing --release --locked
cp "$CARGO_TARGET_DIR/release/amazing" "$RUN/amazing"
cargo nextest run --workspace --locked --no-fail-fast
cargo test --workspace --locked --doc
python3 "$E/layout18/validate-linux-runner.py"
for corpus in ruby tables retained; do
  case "$corpus" in
    ruby) MANIFEST="$E/layout21/manifest.json"; BEFORE="$E/layout21/before/report.json";;
    tables) MANIFEST="$E/layout19/manifest.json"; BEFORE="$E/layout20/final-after/tables/report.json";;
    retained) MANIFEST="$E/layout16/layout16-full-manifest.json"; BEFORE="$E/layout20/final-after/retained/report.json";;
  esac
  python3 "$E/layout18/render-linux.py" --manifest "$MANIFEST" \
    --wpt "$HOME/corpora/wpt-layout" --binary "$RUN/amazing" --source "$SOURCE" \
    --output "$RUN/$corpus" --compress-images
  python3 "$E/layout18/compare.py" "$BEFORE" "$RUN/$corpus/report.json" "$RUN/$corpus-comparison.json"
done
python3 "$E/layout21/triage.py" "$RUN/ruby/report.json" "$RUN/triage.json"
