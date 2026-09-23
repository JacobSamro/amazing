#!/usr/bin/env bash
# The primary integrator's authoritative merged-head gate, one at a time.
#
# Two gates in one tree at once corrupt each other: the second refresh rewrites
# the tree under the first one's build, and the first then reports numbers for
# a tree it did not start with. That happened twice on 2026-09-23. This holds a
# Mac-side lock for the whole transfer + build + test, so a second invocation
# waits instead of overwriting.
#
# The gate runs on win-002, a Windows box dedicated to merge gates, so it never
# queues behind lane builds in win-001's slots. GATE_HOST=win-001 runs it on
# win-001 in the integrator-primary tree under a build slot, as before.
#
# Usage:  scripts/primary-gate.sh [ref]      (default: origin/dev)
#   A local ref, such as a merge not yet pushed, gates that exact commit.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
MAIN="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
LOCK="$MAIN/.agents/.primary-gate.lock"
REF="${1:-origin/dev}"
HOST="${GATE_HOST:-win-002}"

until mkdir "$LOCK" 2>/dev/null; do
  holder="$(cat "$LOCK/pid" 2>/dev/null || true)"
  if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
    rm -rf "$LOCK"
    continue
  fi
  sleep 15
done
echo "$$" > "$LOCK/pid"
TAR="$(mktemp -t primary-gate).tar"
trap 'rm -rf "$LOCK" "$TAR"' EXIT INT TERM

git fetch -q origin
COMMIT="$(git rev-parse "$REF")"
echo "primary-gate: $REF ($COMMIT) on $HOST"

summarize() {
  tr -d '\r' | grep -E "Summary|FAIL|error|test result|acquired" |
    awk '/test result/{split($0,a," ");p+=a[4];f+=a[6];next}{print}
         END{print "doctests: " p " passed / " f " failed"}'
}

if [ "$HOST" = win-001 ]; then
  # Pin the exact commit. Passing a symbolic ref like HEAD let a later local
  # merge slip into a gate that was queued for an earlier commit.
  REFRESH_REF="$COMMIT" ./scripts/refresh-lane-trees.sh integrator-primary 2>&1 |
    grep -E "refreshing|lock=|FAIL|SKIP"
  PS=$(python3 -c "
import base64
s=r'''
\$env:CARGO_TARGET_DIR = 'C:\Users\server\target\integrator-primary'
Set-Location 'C:\Users\server\src\wt\integrator-primary'
cargo nextest run --workspace --locked 2>&1 | Select-String -Pattern 'Summary','error\[','error:','FAIL ' | Out-String -Width 220
cargo test --workspace --locked --doc 2>&1 | Select-String -Pattern '^test result:','^error' | Out-String -Width 220
'''
print(base64.b64encode(s.encode('utf-16-le')).decode())")
  ./scripts/with-build-slot.sh integrator-primary -- \
    ssh win-001 "powershell -NoProfile -EncodedCommand $PS" 2>&1 | summarize
else
  # A clean tree per gate from git archive, so nothing stale survives. Bulk
  # transfer on its own connection, never the shared master.
  git archive --format=tar -o "$TAR" "$COMMIT"
  scp -q -o ControlPath=none "$TAR" "$HOST:gate.tar" 2>&1 | grep -v -i -E 'post-quantum|store now|pq\.html'
  PS=$(python3 -c "
import base64
s=r'''
\$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User') + ';' + \$env:USERPROFILE + '\.cargo\bin'
foreach (\$n in 'CMAKE','LIBCLANG_PATH','VPX_INCLUDE_DIR','VPX_LIB_DIR','VPX_STATIC','VPX_VERSION','PKG_CONFIG','PKG_CONFIG_PATH') { Set-Item -Path Env:\$n -Value ([Environment]::GetEnvironmentVariable(\$n,'User')) }
\$tree = \$env:USERPROFILE + '\src\wt\integrator-primary'
if (Test-Path \$tree) { Remove-Item -Recurse -Force \$tree }
New-Item -ItemType Directory -Force \$tree | Out-Null
Set-Location \$tree
tar -xf (\$env:USERPROFILE + '\gate.tar')
\$env:CARGO_TARGET_DIR = \$env:USERPROFILE + '\target\integrator-primary'
cargo nextest run --workspace --locked 2>&1 | Select-String -Pattern 'Summary','error\[','error:','FAIL ' | Out-String -Width 220
cargo test --workspace --locked --doc 2>&1 | Select-String -Pattern '^test result:','^error' | Out-String -Width 220
'''
print(base64.b64encode(s.encode('utf-16-le')).decode())")
  ssh "$HOST" "powershell -NoProfile -EncodedCommand $PS" 2>&1 | summarize
fi
echo "primary-gate: done for $COMMIT on $HOST"
