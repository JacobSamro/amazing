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
ARCHIVE_LOCAL="/tmp/dev-refresh.tar"
ARCHIVE_REMOTE='C:\Users\server\dev-refresh.tar'

LANES=("$@")
if [ ${#LANES[@]} -eq 0 ]; then
  LANES=(js-lang dom-bindings layout css net replaced-content)
fi

cd "$REPO"
COMMIT="$(git rev-parse --short HEAD)"
echo "refreshing lane trees to $COMMIT: ${LANES[*]}"

tar -cf "$ARCHIVE_LOCAL" \
  --exclude='.git' --exclude='target' --exclude='.agents' --exclude='.DS_Store' \
  crates docs examples scripts tests \
  Cargo.toml Cargo.lock AGENTS.md ARCHITECTURE.md README.md todo.md CONTEXT.md goal.md

scp -q -i "$KEY" "$ARCHIVE_LOCAL" "$HOST:${ARCHIVE_REMOTE//\\//}"

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
  tar -xmf '$ARCHIVE_REMOTE' -C \$dest
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
  "powershell -NoProfile -EncodedCommand $ENCODED" 2>/dev/null |
  tr -d '\r' | grep -E '^[a-z-]+ lock=' || true

LOCAL_LOCK=$(shasum -a 256 Cargo.lock | cut -c1-8 | tr 'a-f' 'A-F')
echo "local lock=$LOCAL_LOCK  (each lane above must match)"
echo
echo "REMINDER: tell every refreshed lane to re-scp its own changed files"
echo "before gating, and to treat earlier Windows results as stale."
