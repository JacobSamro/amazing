#!/usr/bin/env bash
# Fixed-base before/after measurements, only through linux-run.sh layout.
set -euo pipefail
MODE=${1:?before or after}
SOURCE=${2:?source commit or checkpoint}
case "$MODE" in before|after) ;; *) exit 2 ;; esac
E=crates/amazing-layout/evidence
RUN="$OUT/layout19/final-$MODE"
mkdir -p "$RUN"
test ! -e "$RUN/amazing"
printf 'source=%s\ntarget=%s\n' "$SOURCE" "$CARGO_TARGET_DIR" > "$RUN/provenance.txt"
sha256sum crates/amazing-layout/src/table.rs crates/amazing-layout/src/table_tests.rs Cargo.lock >> "$RUN/provenance.txt"
cargo build -p amazing --release --locked
cp "$CARGO_TARGET_DIR/release/amazing" "$RUN/amazing"
if [ "$MODE" = before ]; then
  cargo nextest run -p amazing-layout --locked --no-fail-fast
  cargo test -p amazing-layout --locked --doc
else
  cargo nextest run --workspace --locked --no-fail-fast
  cargo test --workspace --locked --doc
fi
python3 "$E/layout18/validate-linux-runner.py"
python3 "$E/layout18/render-linux.py" --manifest "$E/layout19/manifest.json" \
  --wpt "$HOME/corpora/wpt-layout" --binary "$RUN/amazing" --source "$SOURCE" \
  --output "$RUN/tables" --compress-images
python3 "$E/layout18/render-linux.py" --manifest "$E/layout16/layout16-full-manifest.json" \
  --wpt "$HOME/corpora/wpt-layout" --binary "$RUN/amazing" --source "$SOURCE" \
  --output "$RUN/retained" --compress-images
if [ "$MODE" = after ]; then
  for corpus in tables retained; do
    python3 "$E/layout18/compare.py" "$OUT/layout19/final-before/$corpus/report.json" \
      "$RUN/$corpus/report.json" "$RUN/$corpus-comparison.json"
  done
  python3 "$E/layout19/triage.py" "$RUN/tables/report.json" "$RUN/triage.json"
fi
