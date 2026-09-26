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
# TEST262_ROOT (pinned 3655e746 checkout), toolchain 1.97.1.
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

# Lane homes. Since 2026-09-26 every lane runs on its own Boat sandbox
# (scripts/boat-lane.py: one sandbox per lane from the amazing-builder template,
# stopped by boat-idle-reaper.py after 15 idle minutes). The venus hosts remain
# available with --host venus-001 / venus-003.
[ -z "$HOST" ] && HOST=boat
case "$HOST" in
  boat) SLOTS=0 JOBS=0 ;;
  venus-001) SLOTS=3 JOBS=10 ;;
  venus-003) SLOTS=2 JOBS=8 ;;
  *) echo "linux-run: unknown host $HOST" >&2; exit 2 ;;
esac

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

trap 'rm -rf "$LANE_LOCK"' EXIT
# Source: the lane's worktree (with uncommitted edits), or an exact commit.
if [ -n "$REF" ]; then
  COMMIT="$(git -C "$MAIN" rev-parse "$REF")" || exit 2
  SRC="$(mktemp -d)"
  trap 'rm -rf "$SRC" "$LANE_LOCK"' EXIT
  git -C "$MAIN" archive "$COMMIT" | tar x -C "$SRC"
  echo "linux-run[$LANE]: $HOST at $REF ($COMMIT)" >&2
else
  if [ "$LANE" = integrator-primary ]; then SRC="$MAIN"; else SRC="$(dirname "$MAIN")/amazing-wt/$LANE"; fi
  [ -d "$SRC" ] || { echo "linux-run: no worktree at $SRC" >&2; exit 2; }
  echo "linux-run[$LANE]: $HOST from $SRC ($(git -C "$SRC" rev-parse --short HEAD)$(git -C "$SRC" diff --quiet HEAD || echo ', with uncommitted edits'))" >&2
fi

if [ "$HOST" = boat ]; then
  read -r BOAT_HOST BOAT_PORT BOAT_ID < <("$REPO/scripts/boat-lane.py" ensure "$LANE") || {
    echo "linux-run: no Boat sandbox for $LANE" >&2; exit 3; }
  echo "linux-run[$LANE]: boat sandbox $BOAT_ID" >&2
  BSSH="ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ServerAliveInterval=30 -i $HOME/.ssh/ascii_box_ed25519 -p $BOAT_PORT"
  # The template keeps a warm target/ and the Test262 checkout outside the
  # synced tree; never let --delete touch them.
  rsync -rlpD --checksum --delete --exclude .git --exclude /target --exclude /out --exclude /corpora \
    --exclude /.agents --exclude .DS_Store -e "$BSSH" "$SRC/" "$BOAT_HOST:amazing/" || {
    echo "linux-run: rsync to $BOAT_ID failed" >&2; exit 3; }
  REMOTE=". ~/.amazing-env
cd ~/amazing || exit 3
export CARGO_TARGET_DIR=\$HOME/amazing/target JOBS=\$(nproc) CARGO_BUILD_JOBS=\$(nproc)
export TEST262_ROOT=\$HOME/corpora/test262 RUSTUP_TOOLCHAIN=1.97.1
mkdir -p ~/amazing/out; ln -sfn ~/corpora ~/amazing/corpora
$CMD"
  ENCODED="$(printf '%s' "$REMOTE" | base64 | tr -d '\n')"
  $BSSH "$BOAT_HOST" "echo $ENCODED | base64 -d | bash"
  exit $?
fi

# Bulk transfers get their own connection; a stalled one on the shared master
# blocks every other lane's ssh.
#
# No -t: modification times are NOT preserved. `git archive` stamps every file
# with the commit time, which is older than a previous build's artifacts, so
# cargo reused a stale binary after a --ref switch (JSRT-6's Test262 check ran
# the 7c6824b host). --checksum rewrites only files whose content changed, and
# a rewritten file gets the current time, so cargo always sees real changes.
rsync -rlpD --checksum --delete --exclude .git --exclude /target --exclude /.agents --exclude .DS_Store \
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
