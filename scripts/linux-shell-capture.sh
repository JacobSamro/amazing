#!/usr/bin/env bash
# Actual-window product proof, separate from compact conformance gate evidence.
# --ref is required: captures identify the exact built source commit.
# Usage: linux-shell-capture.sh LANE --ref REF [--size WxH] [--wait S]
#   [--name N] [--release] [--scroll N] [--actions FILE] -- [shell args...]
#   or: ... --real-pages [--limit N] [--only SLUG] [--baseline auto|DIR] [--fuzz PCT]
# Action lines: key TEXT / type TEXT / wait SECONDS / shot NAME.
# Each image <=512 KiB; the complete manifest and images <=2 MiB. Use --only
# or --limit for a bounded real-pages proof. Logs, diffs and full reports stay
# on Boat. Local artifacts are explicit PNGs and *.window.manifest.json only.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
LANE="${1:?lane required}"; shift
REF= SIZE=1280x800 WAIT=8 NAME="capture-$(date -u +%Y%m%dT%H%M%SZ)" PROFILE=debug SCROLL=0
REAL_PAGES=0 LIMIT=0 ONLY= BASELINE= FUZZ=1 ACTIONS=
while [ $# -gt 0 ]; do
  case "$1" in
    --ref) REF="$2"; shift 2;;
    --size) SIZE="$2"; shift 2;;
    --wait) WAIT="$2"; shift 2;;
    --name) NAME="$2"; shift 2;;
    --release) PROFILE=release; shift;;
    --scroll) SCROLL="$2"; shift 2;;
    --actions) ACTIONS="$2"; shift 2;;
    --real-pages) REAL_PAGES=1; BASELINE=auto; shift;;
    --limit) LIMIT="$2"; shift 2;;
    --only) ONLY="$2"; shift 2;;
    --baseline) BASELINE="$2"; shift 2;;
    --fuzz) FUZZ="$2"; shift 2;;
    --) shift; break;;
    *) echo "unknown capture option: $1" >&2; exit 2;;
  esac
done
[[ "$LANE" =~ ^[a-z0-9-]+$ && "$NAME" =~ ^[A-Za-z0-9_-]+$ && "$SIZE" =~ ^[1-9][0-9]*x[1-9][0-9]*$ ]] || { echo 'invalid lane/name/size' >&2; exit 2; }
[ -n "$REF" ] || { echo 'actual window proof requires --ref REF' >&2; exit 2; }
SOURCE="$(git -C "$REPO" rev-parse --verify "$REF^{commit}")"
# Quote every supplied argument before constructing the remote command.
ASSIGN="$(python3 - "$LANE" "$SOURCE" "$SIZE" "$WAIT" "$NAME" "$PROFILE" "$SCROLL" "$REAL_PAGES" "$LIMIT" "$ONLY" "$BASELINE" "$FUZZ" "$ACTIONS" "$@" <<'PY'
import shlex, sys
names = 'LANE SOURCE SIZE WAIT NAME PROFILE SCROLL REAL_PAGES LIMIT ONLY BASELINE FUZZ ACTIONS'.split()
for name, value in zip(names, sys.argv[1:14]): print(name + '=' + shlex.quote(value))
print('SHELL_ARGS=(' + shlex.join(sys.argv[14:]) + ')')
PY
)"
HELPER="$(base64 < "$REPO/scripts/window-proof.py" | tr -d '\n')"
REMOTE_BODY="$(cat "$REPO/scripts/linux-shell-capture-remote.sh")"
STATUS=0
bash "$REPO/scripts/linux-run.sh" "$LANE" --ref "$SOURCE" -- "$ASSIGN
HELPER='$HELPER'
$REMOTE_BODY" || STATUS=$?
if [ "$REAL_PAGES" = 1 ]; then SUBDIR=real-pages; else SUBDIR=captures; fi
# A comparator mismatch may still have valid actual-window artifacts; fetch
# those while preserving its real exit. Build/capture failures fail transfer.
FETCH_STATUS=0
bash "$REPO/scripts/lane-fetch-window.sh" "$LANE" "$MAIN/.agents/$LANE/$SUBDIR/$NAME" \
  "amazing/out/$LANE/$SUBDIR/$NAME/$NAME.window.manifest.json" || FETCH_STATUS=$?
[ "$STATUS" = 0 ] || exit "$STATUS"
exit "$FETCH_STATUS"
