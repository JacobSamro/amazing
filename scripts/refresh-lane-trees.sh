#!/usr/bin/env bash
# Refresh every lane's Windows build tree to the current local commit.
#
# A lane's build tree is seeded once and afterwards receives only that lane's
# own files, so it goes stale for every crate the lane does not own as `dev`
# advances. A full-workspace gate run against a stale tree measures old sibling
# code and is worthless. Rebasing on the Mac does not help: the server tree is
# a separate copy reached only by scp.
#
# WARNING: this overwrites lane-owned files in the server trees with the
# versions from this checkout. The Mac worktrees are the source of truth so
# nothing is lost, but each lane must re-scp its own changed files afterwards
# and treat earlier Windows results as stale. Tell them.
#
# Usage:  scripts/refresh-lane-trees.sh [lane ...]     (default: all lanes)

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEY="$HOME/.ssh/win001-build-key"
HOST="server@win-001"
REMOTE_ROOT='C:\Users\server\src\wt'
# Unique per invocation. Lanes refresh themselves concurrently, and a shared
# archive path means one run can be extracting while another overwrites or
# locks the file -- which already produced a silent no-op refresh that still
# printed matching hashes.
STAMP="$$-$(date +%s)"
ARCHIVE_LOCAL="/tmp/dev-refresh-$STAMP.tar"
ARCHIVE_REMOTE="C:\\Users\\server\\dev-refresh-$STAMP.tar"
cleanup() { rm -f "$ARCHIVE_LOCAL"; }
trap cleanup EXIT

# "integrator" and "integrator-primary" are not agent lanes: they are the trees
# the co-integrator and the primary integrator use for authoritative gates on a
# merged head. Borrowing a lane's tree would overwrite its synced work
# mid-ticket. Two integrators sharing one tree failed too: one's REFRESH-HOLD
# silently skipped the other's refresh, and a gate measured the wrong commit.
ALL_LANES=(js-lang js-runtime js-vm js-binary js-stdlib ipc compositor storage text dom-bindings layout css net replaced-content html integrator-assist integrator integrator-primary)

# Lanes run this for THEMSELVES, so a bare invocation must not touch anyone
# else's tree. Refreshing every lane is an integrator action and has to be
# asked for explicitly.
if [ $# -eq 0 ]; then
  echo "usage: $0 <lane> [lane ...]    # refresh specific lanes (what a lane runs)" >&2
  echo "       $0 --all                # refresh every lane (integrator only)" >&2
  exit 2
fi

if [ "$1" = "--all" ]; then
  LANES=("${ALL_LANES[@]}")
else
  LANES=("$@")
  for l in "${LANES[@]}"; do
    case " ${ALL_LANES[*]} " in
      *" $l "*) ;;
      *) echo "$0: unknown lane '$l'" >&2; exit 2 ;;
    esac
  done
fi

cd "$REPO"

# A lane mid-gate can create .agents/<lane>/REFRESH-HOLD to protect its tree.
# A refresh overwrites lane-owned files, so doing it during a final gate
# invalidates the run -- that has now happened twice, both times because the
# integrator simply forgot. Remembering is not a mechanism; this is.
# Holds live in the primary checkout; a lane running this from its worktree
# would otherwise look in its own private .agents/ and miss every hold.
HOLDS="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)")/.agents"
[ -d "$HOLDS" ] || HOLDS=".agents"
held=()
for l in "${LANES[@]}"; do
  if [ -f "$HOLDS/$l/REFRESH-HOLD" ]; then
    held+=("$l")
    echo "SKIP $l: REFRESH-HOLD set -- $(head -1 "$HOLDS/$l/REFRESH-HOLD" 2>/dev/null)" >&2
  fi
done
if [ ${#held[@]} -gt 0 ]; then
  remaining=()
  set +u
  for l in "${LANES[@]}"; do
    skip=0
    for h in "${held[@]}"; do [ "$l" = "$h" ] && skip=1; done
    [ $skip -eq 0 ] && remaining+=("$l")
  done
  LANES=("${remaining[@]}")
  set -u
  [ ${#LANES[@]} -eq 0 ] && { echo "all requested lanes are on REFRESH-HOLD; nothing to do" >&2; exit 0; }
fi

# Refresh from a REF, not from this checkout's working tree. Lanes run this by
# absolute path from the primary checkout, and the co-integrator pushes from its
# own clone, so the primary checkout can lag origin/dev -- which already sent a
# lane a stale tree while it believed it had refreshed to current dev. Every
# worktree shares one object store, so `git archive` of the fetched ref is
# identical no matter who runs this or what state their checkout is in.
git fetch -q origin
REF="${REFRESH_REF:-origin/dev}"
COMMIT="$(git rev-parse --short "$REF")"
echo "refreshing lane trees to $REF ($COMMIT): ${LANES[*]}"

git archive --format=tar -o "$ARCHIVE_LOCAL" "$REF"

# Bulk uploads bypass the shared ControlMaster. Every ssh/scp to win-001 rides
# one TCP connection, so a 45 MB archive starves every other lane's commands:
# a 5 MB probe took 69 s through the master against 3.7 s on its own
# connection (2026-09-23). MaxStartups is 50, so a dedicated connection per
# bulk transfer is safe; short commands still use the master.
scp -q -o ControlPath=none -i "$KEY" "$ARCHIVE_LOCAL" "$HOST:${ARCHIVE_REMOTE//\\//}"

# cmd.exe mangles loops and quoted paths, so drive this from PowerShell. The
# script is passed as UTF-16LE base64 per the AGENTS.md build-server notes.
build_ps() {
  local lanes_ps
  lanes_ps=$(printf "'%s'," "${LANES[@]}")
  cat <<PS
foreach (\$l in @(${lanes_ps%,})) {
  \$dest = Join-Path '$REMOTE_ROOT' \$l
  if (-not (Test-Path \$dest)) { New-Item -ItemType Directory -Force -Path \$dest | Out-Null }
  # -m sets extracted files to the current time. Without it tar restores the
  # archive's mtimes, which can be OLDER than the cached build artifacts, so
  # Cargo's fingerprints still look fresh and the gate silently links a stale
  # rlib against byte-correct source. Matching source hashes do NOT prove
  # artifact freshness.
  tar -xmf "$ARCHIVE_REMOTE" -C \$dest
  if (\$LASTEXITCODE -ne 0) { "REFRESH-ERROR \$l tar exit \$LASTEXITCODE"; continue }
  # Belt and braces: drop first-party fingerprints and artifacts so they must
  # rebuild, while leaving third-party deps cached so this stays cheap.
  \$t = Join-Path 'C:\Users\server\target' \$l
  foreach (\$p in @('debug','release')) {
    \$fp = Join-Path \$t "\$p\.fingerprint"
    if (Test-Path \$fp) {
      Get-ChildItem \$fp -Directory -Filter 'amazing-*' -EA SilentlyContinue |
        Remove-Item -Recurse -Force -EA SilentlyContinue
    }
    \$dp = Join-Path \$t "\$p\deps"
    if (Test-Path \$dp) {
      Get-ChildItem \$dp -Filter '*amazing*' -EA SilentlyContinue |
        Remove-Item -Recurse -Force -EA SilentlyContinue
    }
  }
  \$probe = Join-Path \$dest 'Cargo.lock'
  \$h = (Get-FileHash \$probe -Algorithm SHA256).Hash.Substring(0,8)
  "\$l lock=\$h"
}
PS
}

ENCODED=$(build_ps | python3 -c 'import sys,base64; print(base64.b64encode(sys.stdin.read().encode("utf-16-le")).decode())')
ssh -i "$KEY" -o BatchMode=yes -o ConnectTimeout=120 "$HOST" \
  "powershell -NoProfile -EncodedCommand $ENCODED" 2>&1 |
  tr -d '\r' | tee /tmp/refresh-out-$STAMP.txt |
  grep -E '^[a-z-]+ lock=|^REFRESH-ERROR'

# A refresh that fails must never look like one that worked. Verify every
# requested lane actually reported, and that none reported an error.
if grep -q '^REFRESH-ERROR' /tmp/refresh-out-$STAMP.txt; then
  echo "refresh FAILED -- see errors above" >&2
  rm -f /tmp/refresh-out-$STAMP.txt
  exit 1
fi
for l in "${LANES[@]}"; do
  if ! grep -q "^$l lock=" /tmp/refresh-out-$STAMP.txt; then
    echo "refresh FAILED: lane '$l' did not report a lock hash" >&2
    rm -f /tmp/refresh-out-$STAMP.txt
    exit 1
  fi
done
rm -f /tmp/refresh-out-$STAMP.txt

LOCAL_LOCK=$(git show "$REF:Cargo.lock" | shasum -a 256 | cut -c1-8 | tr 'a-f' 'A-F')
echo "$REF lock=$LOCAL_LOCK  (each lane above must match)"
echo
echo "REMINDER: tell every refreshed lane to re-scp its own changed files"
echo "before gating, and to treat earlier Windows results as stale."
