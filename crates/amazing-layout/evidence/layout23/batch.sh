#!/usr/bin/env bash
set -euo pipefail
TAG=${1:?unique tag}
SOURCE=${2:?source checkpoint}
E=crates/amazing-layout/evidence
RUN="$OUT/layout23/$TAG"
mkdir -p "$RUN"
test ! -e "$RUN/amazing"
printf 'source=%s\ntarget=%s\n' "$SOURCE" "$CARGO_TARGET_DIR" > "$RUN/provenance.txt"
sha256sum crates/amazing-layout/src/{position,position_constraints,block,box_tree,lib}.rs Cargo.lock >> "$RUN/provenance.txt"
cargo build -p amazing --release --locked
cp "$CARGO_TARGET_DIR/release/amazing" "$RUN/amazing"
status=0
cargo nextest run -p amazing-layout --locked --no-fail-fast || status=$?
cargo test -p amazing-layout --locked --doc || status=$?
for corpus in position writing ruby tables retained; do
  case "$corpus" in
    position) MANIFEST="$E/layout23/manifest.json"; BEFORE="$E/layout23/before/report.json";;
    writing) MANIFEST="$E/layout22/manifest.json"; BEFORE="$E/layout22/final-after/writing/report.json";;
    ruby) MANIFEST="$E/layout21/manifest.json"; BEFORE="$E/layout22/final-after/ruby/report.json";;
    tables) MANIFEST="$E/layout19/manifest.json"; BEFORE="$E/layout22/final-after/tables/report.json";;
    retained) MANIFEST="$E/layout16/layout16-full-manifest.json"; BEFORE="$E/layout22/final-after/retained/report.json";;
  esac
  # Unsupported XHTML DTD cases are baseline limitations, recorded in reports.
  python3 "$E/layout18/render-linux.py" --manifest "$MANIFEST" --wpt "$HOME/corpora/wpt-layout" \
    --binary "$RUN/amazing" --source "$SOURCE" --output "$RUN/$corpus" --compress-images || true
  test -f "$RUN/$corpus/report.json"
  python3 "$E/layout18/compare.py" "$BEFORE" "$RUN/$corpus/report.json" "$RUN/$corpus-comparison.json"
done
exit "$status"
