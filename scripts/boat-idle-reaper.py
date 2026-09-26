#!/usr/bin/env python3
"""Stop Boat sandboxes that have been idle for IDLE_MINUTES.

Boat has no idle timer: its auto-stop TTL counts from creation or resume, never
from the last activity (docs.boat.dev/faq). A sandbox's own in-VM token cannot
stop itself either (401 on POST /sandboxes/{id}/stop), so the countdown runs
here, with the account's CLI login, and no credential ever enters a sandbox.

A sandbox counts as busy while linux-run.sh holds a lease file on it, while its
1-minute load is at or above LOAD_BUSY, or while a build/test process (cargo, rustc, nextest, the Test262 host, a C
compiler or linker) is running. Anything else is idle. After IDLE_MINUTES of
continuous idleness it is stopped: Boat snapshots it first and billing ends; a
`boat resume` brings it back in seconds with its disk intact.

Sandboxes whose name contains "keep" are never stopped.

Usage: scripts/boat-idle-reaper.py [--idle-minutes 5] [--scarce-idle-minutes 15] [--poll 60] [--dry-run]
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
from datetime import datetime, timezone

BOAT = os.path.expanduser("~/.ascii/bin/boat")
LIVE_STATES = {"provisioned", "cloning", "ready", "idle", "running"}
LOAD_BUSY = 0.3
PROBE = (
    "cut -d' ' -f1 /proc/loadavg; "
    "pgrep -xc 'cargo|rustc|cargo-nextest|test262_host|cc|c\\+\\+|ld|ld.lld|clang|gcc' || true; "
    # linux-run.sh leases (removed when its command exits; stale after 3 h).
    "find ~/.amazing-busy -type f -mmin -180 2>/dev/null | wc -l"
)


def log(msg: str) -> None:
    stamp = datetime.now(timezone.utc).strftime("%H:%M:%SZ")
    print(f"{stamp} {msg}", flush=True)


def boat(*args: str, timeout: int = 60) -> dict | None:
    try:
        out = subprocess.run(
            [BOAT, *args, "--json"], capture_output=True, text=True, timeout=timeout
        ).stdout
    except subprocess.TimeoutExpired:
        return None
    lines = [l for l in out.splitlines() if l.startswith("{")]
    return json.loads(lines[-1]) if lines else None


def running_sandboxes() -> list[dict]:
    listing = boat("list") or {}
    return [s for s in listing.get("sandboxes", []) if s.get("state") in LIVE_STATES]


def is_busy(sandbox_id: str) -> bool | None:
    """True/False from a probe, None if the probe failed (treated as busy)."""
    result = boat("exec", sandbox_id, PROBE, timeout=60)
    if not result or not result.get("success"):
        return None
    lines = result.get("stdout", "").split()
    try:
        load, procs = float(lines[0]), int(lines[1])
        leases = int(lines[2]) if len(lines) > 2 else 0
    except (IndexError, ValueError):
        return None
    return load >= LOAD_BUSY or procs > 0 or leases > 0


START_RESERVE = 15  # keep this many hourly starts free for lanes and Test262 shards


def starts_left_this_hour() -> int | None:
    status = boat("billing", "status") or {}
    try:
        return int(status["limits"]["starts"]["hour"]["remaining"])
    except (KeyError, TypeError, ValueError):
        return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--idle-minutes", type=float, default=5)
    # Every resume counts against Boat's 60 starts per hour. When few are left,
    # stopping early just forces a resume that may be refused, so wait longer.
    ap.add_argument("--scarce-idle-minutes", type=float, default=15)
    ap.add_argument("--poll", type=int, default=60)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    idle_since: dict[str, float] = {}
    log(f"reaper started: stop after {args.idle_minutes:g} idle minutes, poll {args.poll}s")
    while True:
        now = time.time()
        left = starts_left_this_hour()
        limit = args.idle_minutes
        if left is not None and left < START_RESERVE:
            limit = args.scarce_idle_minutes
        live = running_sandboxes()
        for sb in live:
            sid, name = sb["id"], sb.get("name", "")
            if "keep" in name.lower():
                continue
            busy = is_busy(sid)
            if busy is not False:
                idle_since.pop(sid, None)
                continue
            first = idle_since.setdefault(sid, now)
            idle_min = (now - first) / 60
            if idle_min >= limit:
                # Probe once more: a run may have started since the last poll.
                if is_busy(sid) is not False:
                    idle_since.pop(sid, None)
                    continue
                log(f"STOP {sid} ({name}, {sb.get('type')}) idle {idle_min:.0f} min")
                if not args.dry_run:
                    boat("stop", sid, timeout=120)
                idle_since.pop(sid, None)
        # Forget sandboxes that stopped some other way.
        live_ids = {s["id"] for s in live}
        for sid in list(idle_since):
            if sid not in live_ids:
                idle_since.pop(sid)
        time.sleep(args.poll)


if __name__ == "__main__":
    raise SystemExit(main())
