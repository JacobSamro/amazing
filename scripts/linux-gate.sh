#!/usr/bin/env bash
# Integrator merge gate on Linux (Boat): workspace nextest + doctests at an exact ref.
# Usage: scripts/linux-gate.sh <ref>
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REF="${1:-HEAD}"
"$REPO/scripts/linux-run.sh" integrator-primary --ref "$REF" -- \
  'set -euo pipefail
   cargo nextest run --workspace --locked --no-fail-fast 2>&1 | tee "$OUT/workspace-nextest.log"
   cargo test --workspace --locked --doc 2>&1 | tee "$OUT/workspace-doctests.log"'
