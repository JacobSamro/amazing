#!/usr/bin/env bash
# Run a heavy win-001 command only while holding one of N build slots.
#
# win-001 is one 16-core box that sustains about three concurrent full builds.
# Ten lanes that each launch `cargo nextest run --workspace` at the same moment
# -- which is exactly what happens when a fleet resumes and every lane re-gates
# -- turn three fast builds into ten slow ones that time out. Queueing is
# cheaper than thrashing.
#
# Slots are directories under .agents/.build-slots on the Mac, because every
# lane drives win-001 from this machine. `mkdir` is atomic, so claiming a slot
# needs no daemon. A slot whose owning process has died is reclaimed, so a
# killed lane cannot leak one.
#
# Usage:  scripts/with-build-slot.sh <lane> -- <command...>
#   e.g.  scripts/with-build-slot.sh css -- ssh win-001 "powershell -NoProfile -EncodedCommand ..."
#
# Only wrap the heavy step (a --workspace gate or a full render sweep). Crate-
# scoped `cargo nextest run -p <crate>` iterations do not need a slot.

set -uo pipefail

SLOTS="${BUILD_SLOTS:-3}"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Slots must be shared by every lane. From a lane worktree, REPO is that
# worktree, whose .agents/ is private -- which silently gave each lane its own
# three slots. Resolve the primary checkout through git's common dir instead.
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)")"
[ -d "$MAIN/.agents" ] || MAIN="$REPO"
# BUILD_POOL names a separate set of slots for another machine (linux-run.sh
# uses one pool per venus host). The default pool is win-001's.
DIR="$MAIN/.agents/.build-slots${BUILD_POOL:+-$BUILD_POOL}"
LANE="${1:?usage: with-build-slot.sh <lane> -- <command...>}"
shift
[ "${1:-}" = "--" ] && shift
[ $# -gt 0 ] || { echo "with-build-slot: no command given" >&2; exit 2; }

mkdir -p "$DIR" "$DIR/queue"
held=""
waited=0

# First come, first served. Without a queue, whichever waiter polled first
# after a release won, and a lane once waited 18+ minutes while later arrivals
# took every freed slot. Each waiter holds a ticket named by arrival time; only
# the oldest live tickets may claim a free slot.
ticket="$DIR/queue/$(date +%s)$(printf '%06d' $$)-$$"
: > "$ticket"

release() { [ -n "$held" ] && rm -rf "$held"; rm -f "$ticket"; }
trap release EXIT INT TERM

my_turn() {
  # Drop tickets whose waiter died, then check that we are among the oldest
  # waiters for the number of currently free slots.
  local free=0 rank=0 t pid
  for t in "$DIR"/queue/*; do
    [ -e "$t" ] || continue
    pid="${t##*-}"
    kill -0 "$pid" 2>/dev/null || rm -f "$t"
  done
  for i in $(seq 1 "$SLOTS"); do
    # Reclaim a slot whose holder died, or it would never count as free.
    pid="$(cut -d' ' -f1 "$DIR/slot-$i/owner" 2>/dev/null || true)"
    if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then rm -rf "$DIR/slot-$i"; fi
    [ -d "$DIR/slot-$i" ] || free=$((free + 1))
  done
  for t in $(ls "$DIR/queue" | sort); do
    rank=$((rank + 1))
    [ "$DIR/queue/$t" = "$ticket" ] && break
  done
  [ "$rank" -le "$free" ]
}

while [ -z "$held" ]; do
  if ! my_turn; then
    if [ $((waited % 60)) -eq 0 ]; then
      holders="$(cat "$DIR"/slot-*/owner 2>/dev/null | awk '{print $2}' | tr '\n' ' ')"
      ahead="$(ls "$DIR/queue" | sort | awk -v me="$(basename "$ticket")" '$0==me{exit} {n++} END{print n+0}')"
      echo "with-build-slot[$LANE]: queued, $ahead ahead (holders: ${holders}); waited ${waited}s" >&2
    fi
    sleep 10
    waited=$((waited + 10))
    continue
  fi
  for i in $(seq 1 "$SLOTS"); do
    slot="$DIR/slot-$i"
    if mkdir "$slot" 2>/dev/null; then
      printf '%s %s %s\n' "$$" "$LANE" "$(date -u +%FT%TZ)" > "$slot/owner"
      held="$slot"
      break
    fi
    # Reclaim a slot whose holder died without releasing it.
    owner_pid="$(cut -d' ' -f1 "$slot/owner" 2>/dev/null || true)"
    if [ -n "$owner_pid" ] && ! kill -0 "$owner_pid" 2>/dev/null; then
      rm -rf "$slot"
    fi
  done
  if [ -z "$held" ]; then
    if [ $((waited % 60)) -eq 0 ]; then
      holders="$(cat "$DIR"/slot-*/owner 2>/dev/null | awk '{print $2}' | tr '\n' ' ')"
      echo "with-build-slot[$LANE]: all $SLOTS slots busy (${holders}); waited ${waited}s" >&2
    fi
    sleep 10
    waited=$((waited + 10))
  fi
done

rm -f "$ticket"
echo "with-build-slot[$LANE]: acquired ${BUILD_POOL:+$BUILD_POOL/}$(basename "$held") after ${waited}s" >&2
BUILD_SLOT="${held##*-}" "$@"
