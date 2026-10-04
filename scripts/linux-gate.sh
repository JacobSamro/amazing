#!/usr/bin/env bash
# Integrator merge gate on Linux (Boat): workspace nextest + doctests at an exact ref.
# Usage: scripts/linux-gate.sh <ref>
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REF="${1:-HEAD}"
python3 "$REPO/scripts/merge-preflight.py" --base "${MERGE_BASE:-origin/dev}" --ref "$REF"
"$REPO/scripts/linux-run.sh" integrator-primary --ref "$REF" -- \
  'set -euo pipefail
   export CARGO_TARGET_DIR=/tmp/amazing-target-integrator-primary CARGO_INCREMENTAL=0
   mkdir -p /tmp/amazing-src-integrator-primary
   rsync -a --delete --exclude /target --exclude "/target-*" --exclude /corpora --exclude /out ./ /tmp/amazing-src-integrator-primary/
   ln -sfn "$HOME/corpora" /tmp/amazing-src-integrator-primary/corpora
   cd /tmp/amazing-src-integrator-primary
   cargo nextest run --workspace --locked --no-fail-fast 2>&1 | tee "$OUT/workspace-nextest.log"
   cargo test --workspace --locked --doc 2>&1 | tee "$OUT/workspace-doctests.log"'
