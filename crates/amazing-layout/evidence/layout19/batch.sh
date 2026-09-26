#!/usr/bin/env bash
# Invoke only through scripts/linux-run.sh layout -- 'bash .../batch.sh ...'.
set -euo pipefail
TAG=${1:?unique result tag}
SOURCE=${2:?source commit or checkpoint}
E=crates/amazing-layout/evidence
RUN="$OUT/layout19/$TAG"
mkdir -p "$RUN"
test ! -e "$RUN/amazing"
# The original baseline runner predates optional per-case compression.
find "$OUT/layout19-before-corpus" -name '*.ppm' -exec gzip -1 {} +
sha256sum crates/amazing-layout/src/table.rs crates/amazing-layout/src/table_tests.rs > "$RUN/source.sha256"
cargo build -p amazing --release --locked &&
  cp "$CARGO_TARGET_DIR/release/amazing" "$RUN/amazing" &&
  cargo nextest run -p amazing-layout --locked --no-fail-fast &&
  cargo test -p amazing-layout --locked --doc &&
  python3 "$E/layout18/validate-linux-runner.py" &&
  python3 "$E/layout18/render-linux.py" --manifest "$OUT/layout19-manifest.json" \
    --wpt "$HOME/corpora/wpt-layout" --binary "$RUN/amazing" --source "$SOURCE" \
    --output "$RUN/tables" --compress-images
python3 "$E/layout18/compare.py" "$OUT/layout19-before-corpus/report.json" \
  "$RUN/tables/report.json" "$RUN/comparison.json"
python3 "$E/layout19/triage.py" "$RUN/tables/report.json" "$RUN/triage.json"
if [ "${3:-}" = gate ]; then
  cargo nextest run --workspace --locked --no-fail-fast && cargo test --workspace --locked --doc
  for version in before after; do
    if [ "$version" = before ]; then BIN="$OUT/layout19-before"; REF=78ae0377b40c010aceec55992da06e3718989c3e
    else BIN="$RUN/amazing"; REF="$SOURCE"; fi
    python3 "$E/layout18/render-linux.py" --manifest "$E/layout16/layout16-full-manifest.json" \
      --wpt "$HOME/corpora/wpt-layout" --binary "$BIN" --source "$REF" \
      --output "$RUN/retained-$version" --compress-images
  done
  python3 "$E/layout18/compare.py" "$RUN/retained-before/report.json" \
    "$RUN/retained-after/report.json" "$RUN/retained-comparison.json"
fi
