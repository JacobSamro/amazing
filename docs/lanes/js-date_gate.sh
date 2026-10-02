#!/usr/bin/env bash
# Recheck every baseline or newly passing RegExp file at the full deadline.
# The complete before/after corpus is measured separately at the iteration
# deadline; copies here keep the pinned shared checkout read-only.
set -euo pipefail
: "${OUT:?}" "${TEST262_ROOT:?}" "${CARGO_TARGET_DIR:?}"
baseline="$OUT/push-baseline.json"
after="$OUT/jsd6-after.json"
gate_root="$OUT/jsd6-pass-gate"
mkdir -p "$gate_root/test" "$gate_root/.git"
cp -a "$TEST262_ROOT/harness" "$gate_root/"
jq -rs '[.[].records[] | select(.status == "pass" and (.path | contains("built-ins/RegExp"))) | .path] | unique[]' \
    "$baseline" "$after" > "$OUT/jsd6-pass-gate-paths.txt"
while IFS= read -r path; do
    mkdir -p "$gate_root/test/$(dirname "$path")"
    cp "$TEST262_ROOT/test/$path" "$gate_root/test/$path"
done < "$OUT/jsd6-pass-gate-paths.txt"
jq -r .test262_revision "$after" > "$gate_root/.git/HEAD"
python3 scripts/test262.py "$gate_root" \
    --binary "$CARGO_TARGET_DIR/release/examples/test262_host" \
    --jobs 8 --timeout 60 --output "$OUT/jsd6-pass-gate.json"
jq -e '.summary.files.fail == 0 and .summary.files.timeout == 0 and .summary.files.unsupported == 0' \
    "$OUT/jsd6-pass-gate.json"
