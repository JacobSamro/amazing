#!/usr/bin/env bash
set -euo pipefail
TAG=${1:-final-after}
SOURCE=${2:?source checkpoint}
E=crates/amazing-layout/evidence
RUN="$OUT/layout22/$TAG"
mkdir -p "$RUN"
test ! -e "$RUN/amazing"
printf 'source=%s\ntarget=%s\n' "$SOURCE" "$CARGO_TARGET_DIR" > "$RUN/provenance.txt"
sha256sum crates/amazing-layout/src/{block,box_tree,inline,inline_alignment,inline_bidi,inline_combine,lib}.rs Cargo.lock >> "$RUN/provenance.txt"
cargo build -p amazing --release --locked
cp "$CARGO_TARGET_DIR/release/amazing" "$RUN/amazing"
status=0
cargo nextest run --workspace --locked --no-fail-fast && cargo test --workspace --locked --doc || status=$?
for corpus in writing ruby tables retained; do
  case "$corpus" in
    writing) MANIFEST="$E/layout22/manifest.json"; BEFORE="$E/layout22/before-final/report.json";;
    ruby) MANIFEST="$E/layout21/manifest.json"; BEFORE="$E/layout21/final-repaired/ruby/report.json";;
    tables) MANIFEST="$E/layout19/manifest.json"; BEFORE="$E/layout21/final-repaired/tables/report.json";;
    retained) MANIFEST="$E/layout16/layout16-full-manifest.json"; BEFORE="$E/layout21/final-repaired/retained/report.json";;
  esac
  # Unsupported XHTML DTD cases are baseline limitations, recorded in reports.
  python3 "$E/layout18/render-linux.py" --manifest "$MANIFEST" --wpt "$HOME/corpora/wpt-layout" \
    --binary "$RUN/amazing" --source "$SOURCE" --output "$RUN/$corpus" --compress-images || true
  test -f "$RUN/$corpus/report.json"
  python3 "$E/layout18/compare.py" "$BEFORE" "$RUN/$corpus/report.json" "$RUN/$corpus-comparison.json"
  python3 - "$RUN/$corpus-comparison.json" <<'CHECK' || status=1
import json,sys
r=json.load(open(sys.argv[1]))
failures=r['losses'] or any(r['after_counts'][key] > r['before_counts'][key]
    for key in ('error','timeout','crash','unsupported'))
if failures:
    print('CORPUS GATE HELD:',sys.argv[1],r['losses'],flush=True)
    raise SystemExit(1)
CHECK
done
exit "$status"
