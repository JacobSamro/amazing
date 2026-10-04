#!/usr/bin/env bash
# Exact-ref Boat workspace gate. Raw logs stay under $OUT; fetch only its receipt.
# Usage: scripts/linux-gate.sh [ref]
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
REF="$(git -C "$MAIN" rev-parse "${1:-HEAD}^{commit}")"
python3 "$REPO/scripts/merge-preflight.py" --base "${MERGE_BASE:-origin/dev}" --ref "$REF"
HELPER="$(base64 < "$REPO/scripts/gate-summary.py" | tr -d '\n')"
COMMAND_B64="$(base64 < "$REPO/scripts/gate-command.py" | tr -d '\n')"
set +e
"$REPO/scripts/linux-run.sh" integrator-primary --ref "$REF" -- "
  set -uo pipefail
  unset AMAZING_JS_VM AMAZING_JS_VM_COMPILE_LOG AMAZING_JS_VM_REJECT_CONTEXT
  export CARGO_INCREMENTAL=0
  receipt=\"\$OUT/workspace-$REF.manifest.json\"
  nextest=\"\$OUT/workspace-$REF-nextest.log\"
  docs=\"\$OUT/workspace-$REF-doctests.log\"
  echo $COMMAND_B64 | base64 -d > /tmp/amazing-gate-command.py
  python3 /tmp/amazing-gate-command.py --phase nextest --source-commit $REF --output \"\$nextest\" --receipt \"\$OUT/workspace-$REF-nextest.phase.manifest.json\" -- cargo nextest run --workspace --locked --no-fail-fast --status-level skip
  native_rc=\$?
  python3 /tmp/amazing-gate-command.py --phase doctests --source-commit $REF --output \"\$docs\" --receipt \"\$OUT/workspace-$REF-doctests.phase.manifest.json\" -- cargo test --workspace --locked --doc
  docs_rc=\$?
  rc=0; [ \$native_rc = 0 ] && [ \$docs_rc = 0 ] || rc=1
  echo $HELPER | base64 -d > /tmp/amazing-gate-summary.py
  python3 /tmp/amazing-gate-summary.py manifest --phase-receipt \"\$OUT/workspace-$REF-nextest.phase.manifest.json\" --phase-receipt \"\$OUT/workspace-$REF-doctests.phase.manifest.json\" --source-commit $REF --phase nextest=\$native_rc --phase doctests=\$docs_rc \"\$receipt\" \$rc \"\$nextest\" \"\$docs\" || exit 3
  grep -E 'Summary|test result:' \"\$nextest\" \"\$docs\" || true
  echo workspace_exit=\$native_rc doctests_exit=\$docs_rc
  exit \$rc"
rc=$?
set -e
# Wait above observes the actual SSH job exit, not the existence of a log.
bash "$REPO/scripts/lane-fetch.sh" integrator-primary "$MAIN/.agents/integrator-primary/gates" \
  "amazing/out/integrator-primary/workspace-$REF.manifest.json" || exit 3
exit "$rc"
