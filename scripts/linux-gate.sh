#!/usr/bin/env bash
# Integrator merge gate on Linux (Boat): workspace nextest + doctests at an exact ref.
# Usage: scripts/linux-gate.sh <ref>
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REF="${1:-HEAD}"
"$REPO/scripts/linux-run.sh" integrator-primary --ref "$REF" -- \
  'cargo nextest run --workspace --locked --no-fail-fast 2>&1 | grep -E "Summary|^\s+FAIL|^error" | head -30; cargo test --workspace --locked --doc 2>&1 | grep -E "^test result|FAILED|^error" | tail -4'
