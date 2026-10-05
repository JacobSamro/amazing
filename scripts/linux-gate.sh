#!/usr/bin/env bash
# Exact-ref Boat workspace gate; raw stays native, only compact receipts fetched.
# Usage: scripts/linux-gate.sh [ref] [--lane LANE] [--prepare-only] [--destination PATH]
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$REPO/scripts/linux-workspace-gate.py" "$@"
