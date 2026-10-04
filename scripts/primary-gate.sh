#!/usr/bin/env bash
# The authoritative exact-ref workspace gate is Linux/Boat.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$REPO/scripts/linux-gate.sh" "${1:-origin/dev}"
