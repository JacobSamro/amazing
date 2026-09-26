#!/usr/bin/env bash
# Copy a lane's remote outputs back to the Mac from wherever linux-run.sh ran it
# (a Boat sandbox, or a venus host when .agents/.boat-off is set).
# Usage: scripts/lane-fetch.sh <lane> <dest-dir> <remote path under ~> [...]
#   Remote paths may use globs, e.g. 'amazing/out/shell/real-pages/*'.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
LANE="${1:?usage: lane-fetch.sh <lane> <dest-dir> <remote-path>...}"; DEST="${2:?dest}"; shift 2
mkdir -p "$DEST"
HOST="$("$MAIN/scripts/linux-run.sh" "$LANE" --which)"
rc=0
if [ "$HOST" = boat ]; then
  read -r H P _ < <("$MAIN/scripts/boat-lane.py" ensure "$LANE")
  for src in "$@"; do
    scp -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
      -i "$HOME/.ssh/ascii_box_ed25519" -P "$P" "$H:$src" "$DEST/" 2>/dev/null || rc=1
  done
else
  # A run started before the lane moved hosts left its outputs on the old one;
  # fall back to every other build host.
  for src in "$@"; do
    ok=0
    for h in "$HOST" venus-001 venus-003 root@46.224.130.223 root@159.69.124.56 root@178.104.111.133; do
      scp -q -o ControlPath=none -o ConnectTimeout=10 "$h:$src" "$DEST/" 2>/dev/null && { ok=1; break; }
    done
    [ "$ok" = 1 ] || rc=1
  done
fi
exit $rc
