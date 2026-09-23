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
DIR="$REPO/.agents/.build-slots"
LANE="${1:?usage: with-build-slot.sh <lane> -- <command...>}"
shift
[ "${1:-}" = "--" ] && shift
[ $# -gt 0 ] || { echo "with-build-slot: no command given" >&2; exit 2; }

mkdir -p "$DIR"
held=""
waited=0

release() { [ -n "$held" ] && rm -rf "$held"; }
trap release EXIT INT TERM

while [ -z "$held" ]; do
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

echo "with-build-slot[$LANE]: acquired $(basename "$held") after ${waited}s" >&2
"$@"
