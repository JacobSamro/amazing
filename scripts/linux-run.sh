#!/usr/bin/env bash
# Run a lane's build or test command on a Linux build host (venus-*).
#
# win-001 is the reference platform and the only place merge gates and render
# sweeps count. Everything platform-independent -- crate and workspace nextest
# while iterating, full Test262, the WPT dom/ harness, the net corpora -- runs
# here instead, so it stops queueing behind win-001's three slots.
#
# The lane's working tree, including uncommitted edits, is rsynced to
# ~/amazing/wt/<lane> on its host, and the command runs there with the lane's
# own CARGO_TARGET_DIR. Heavy commands take a slot from that host's pool.
#
# Usage:  scripts/linux-run.sh <lane> [--host H] [--ref REF] [--no-slot] -- <command>
#   <command> is one bash string, run from the synced tree root. For example:
#     scripts/linux-run.sh css -- 'cargo nextest run -p amazing-css --locked'
#     scripts/linux-run.sh js-lang -- 'cargo build -p amazing-js --example test262_host --release --locked &&
#       python3 scripts/test262.py "$TEST262_ROOT" --binary "$CARGO_TARGET_DIR/release/examples/test262_host" --jobs "$JOBS" --timeout 60 --output ~/amazing/out/js-lang.json'
#   --ref REF   sync that exact commit (git archive) instead of the working tree
#   --no-slot   skip the slot queue; for crate-scoped iterations only
#
# Environment on the host: CARGO_TARGET_DIR, JOBS (per-slot parallelism),
# TEST262_ROOT (pinned 3655e746 checkout, venus-003), toolchain 1.97.1.
# Results stay on the host; copy what you need back with
#   scp -o ControlPath=none <host>:<path> <local>

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"

LANE="${1:?usage: linux-run.sh <lane> [--host H] [--ref REF] [--no-slot] -- <command>}"
shift
HOST="" REF="" SLOT=1
while [ $# -gt 0 ]; do
  case "$1" in
    --host) HOST="$2"; shift 2 ;;
    --ref) REF="$2"; shift 2 ;;
    --no-slot) SLOT=0; shift ;;
    --) shift; break ;;
    *) echo "linux-run: unknown option $1" >&2; exit 2 ;;
  esac
done
[ $# -gt 0 ] || { echo "linux-run: no command given" >&2; exit 2; }
CMD="$*"

# Lane homes. Test262 lives on venus-003 (venus-001 has no git, which the
# runner needs for the revision check), so the JavaScript lanes run there.
if [ -z "$HOST" ]; then
  case "$LANE" in
    js-lang|js-runtime|js-vm|integrator-primary) HOST=venus-003 ;;
    *) HOST=venus-001 ;;
  esac
fi
case "$HOST" in
  venus-001) SLOTS=3 JOBS=10 ;;
  venus-003) SLOTS=2 JOBS=8 ;;
  *) echo "linux-run: unknown host $HOST" >&2; exit 2 ;;
esac

# Source: the lane's worktree (with uncommitted edits), or an exact commit.
if [ -n "$REF" ]; then
  COMMIT="$(git -C "$MAIN" rev-parse "$REF")" || exit 2
  SRC="$(mktemp -d)"
  trap 'rm -rf "$SRC"' EXIT
  git -C "$MAIN" archive "$COMMIT" | tar x -C "$SRC"
  echo "linux-run[$LANE]: $HOST at $REF ($COMMIT)" >&2
else
  if [ "$LANE" = integrator-primary ]; then SRC="$MAIN"; else SRC="$(dirname "$MAIN")/amazing-wt/$LANE"; fi
  [ -d "$SRC" ] || { echo "linux-run: no worktree at $SRC" >&2; exit 2; }
  echo "linux-run[$LANE]: $HOST from $SRC ($(git -C "$SRC" rev-parse --short HEAD)$(git -C "$SRC" diff --quiet HEAD || echo ', with uncommitted edits'))" >&2
fi

# Bulk transfers get their own connection; a stalled one on the shared master
# blocks every other lane's ssh.
rsync -a --delete --exclude .git --exclude /target --exclude /.agents --exclude .DS_Store \
  -e "ssh -o ControlPath=none" "$SRC/" "$HOST:amazing/wt/$LANE/" || {
  echo "linux-run: rsync to $HOST failed" >&2; exit 3; }

REMOTE=". ~/.amazing-env
cd ~/amazing/wt/$LANE || exit 3
export CARGO_TARGET_DIR=\$HOME/amazing/target/$LANE JOBS=$JOBS CARGO_BUILD_JOBS=$JOBS
export TEST262_ROOT=\$HOME/amazing/corpora/test262 RUSTUP_TOOLCHAIN=1.97.1
mkdir -p ~/amazing/out
$CMD"
ENCODED="$(printf '%s' "$REMOTE" | base64 | tr -d '\n')"
run() { ssh "$HOST" "echo $ENCODED | base64 -d | bash" 2>&1 | grep -v -i -E 'post-quantum|store now|pq\.html'; return "${PIPESTATUS[0]}"; }

if [ "$SLOT" = 1 ]; then
  BUILD_POOL="$HOST" BUILD_SLOTS="$SLOTS" "$REPO/scripts/with-build-slot.sh" "$LANE" -- bash -c "$(declare -f run); ENCODED='$ENCODED' HOST='$HOST' run"
else
  run
fi
