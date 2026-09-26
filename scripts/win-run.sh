#!/usr/bin/env bash
# Run a lane's Windows build/test step on win-002 instead of win-001.
#
# win-001 has three slots and also carries every render sweep, so crate and
# workspace final gates queued behind 30-minute sweeps (10 deep on
# 2026-09-24). win-002 has the same toolchain (Rust 1.97.1 msvc, VS 2022 Build
# Tools, LLVM, vcpkg libvpx, nextest 0.9.145). Use it for crate and workspace
# nextest/doctest gates. Render sweeps, reftests and anything needing the WPT
# checkout, fonts or GPU stay on win-001.
#
# The lane's worktree (uncommitted edits included), or an exact commit with
# --ref, is extracted fresh into %USERPROFILE%\src\wt\<lane>. Extraction
# stamps files with the current time, so cargo always rebuilds changed
# sources. The lane's own CARGO_TARGET_DIR keeps dependencies cached.
#
# Usage:  scripts/win-run.sh <lane> [--ref REF] -- '<PowerShell commands>'
#   e.g.  scripts/win-run.sh css -- 'cargo nextest run --workspace --locked; cargo test --workspace --locked --doc'
# Commands run from the tree root with $env:CARGO_TARGET_DIR set.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
HOST="${WIN_HOST:-win-002}" SLOTS=2
# win-002 went offline on 2026-09-26; fall back to win-001 (same toolchain and
# %USERPROFILE% layout, its own three-slot pool) instead of failing the gate.
if [ -z "${WIN_HOST:-}" ] && ! ssh -o ConnectTimeout=8 -o BatchMode=yes win-002 'echo ok' >/dev/null 2>&1; then
  echo "win-run: win-002 unreachable, using win-001" >&2
  HOST=win-001 SLOTS=3
fi
LANE="${1:?usage: win-run.sh <lane> [--ref REF] -- '<PowerShell commands>'}"
shift
REF=""
while [ $# -gt 0 ]; do
  case "$1" in
    --ref) REF="$2"; shift 2 ;;
    --) shift; break ;;
    *) echo "win-run: unknown option $1" >&2; exit 2 ;;
  esac
done
[ $# -gt 0 ] || { echo "win-run: no command given" >&2; exit 2; }
CMD="$*"

# One run per lane tree at a time. Two runs of the same lane share the remote
# tree and output paths; on 2026-09-24 a second comparison synced a different
# commit into the tree mid-build and made a Test262 baseline measure the wrong
# source. Held for the whole sync + run.
LOCKDIR="$MAIN/.agents/.remote-locks"
mkdir -p "$LOCKDIR"
LANE_LOCK="$LOCKDIR/$(basename "$0" .sh)-$LANE"
until mkdir "$LANE_LOCK" 2>/dev/null; do
  holder="$(cat "$LANE_LOCK/pid" 2>/dev/null || true)"
  if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then rm -rf "$LANE_LOCK"; continue; fi
  sleep 5
done
echo "$$" > "$LANE_LOCK/pid"

TAR="$(mktemp -t win-run).tar"
trap 'rm -f "$TAR"; rm -rf "$LANE_LOCK"' EXIT
if [ -n "$REF" ]; then
  COMMIT="$(git -C "$MAIN" rev-parse "$REF")" || exit 2
  git -C "$MAIN" archive --format=tar -o "$TAR" "$COMMIT"
  echo "win-run[$LANE]: $HOST at $REF ($COMMIT)" >&2
else
  if [ "$LANE" = integrator-primary ]; then SRC="$MAIN"; else SRC="$(dirname "$MAIN")/amazing-wt/$LANE"; fi
  [ -d "$SRC" ] || { echo "win-run: no worktree at $SRC" >&2; exit 2; }
  COPYFILE_DISABLE=1 tar -cf "$TAR" -C "$SRC" --exclude ./.git --exclude ./target --exclude ./.agents --exclude .DS_Store .
  echo "win-run[$LANE]: $HOST from $SRC ($(git -C "$SRC" rev-parse --short HEAD)$(git -C "$SRC" diff --quiet HEAD || echo ', with uncommitted edits'))" >&2
fi

# Bulk transfer on its own connection; never the shared master.
scp -q -o ControlPath=none "$TAR" "$HOST:wt-$LANE.tar" 2>&1 | grep -v -i -E 'post-quantum|store now|pq\.html'

PS=$(LANE="$LANE" CMD="$CMD" python3 -c "
import base64, os
lane, cmd = os.environ['LANE'], os.environ['CMD']
s = r'''
\$ErrorActionPreference = 'Continue'
\$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User') + ';' + \$env:USERPROFILE + '\.cargo\bin'
foreach (\$n in 'CMAKE','LIBCLANG_PATH','VPX_INCLUDE_DIR','VPX_LIB_DIR','VPX_STATIC','VPX_VERSION','PKG_CONFIG','PKG_CONFIG_PATH') { Set-Item -Path Env:\$n -Value ([Environment]::GetEnvironmentVariable(\$n,'User')) }
\$tree = \$env:USERPROFILE + '\src\wt\LANE'
if (Test-Path \$tree) { Remove-Item -Recurse -Force \$tree }
New-Item -ItemType Directory -Force \$tree | Out-Null
Set-Location \$tree
tar -xmf (\$env:USERPROFILE + '\wt-LANE.tar')
\$env:CARGO_TARGET_DIR = \$env:USERPROFILE + '\target\LANE'
'''.replace('LANE', lane) + cmd + '\n'
print(base64.b64encode(s.encode('utf-16-le')).decode())")

BUILD_POOL="$HOST" BUILD_SLOTS="$SLOTS" "$REPO/scripts/with-build-slot.sh" "$LANE" -- \
  ssh "$HOST" "powershell -NoProfile -EncodedCommand $PS" 2>&1 | tr -d '\r' |
  grep -v -i -E 'post-quantum|store now|pq\.html|^#< CLIXML|<Objs '
