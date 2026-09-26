#!/usr/bin/env python3
"""Give a lane its own Boat sandbox and print how to reach it.

Each lane gets one persistent sandbox deployed from the `amazing-builder`
template (Rust 1.97.1, libvpx 1.16, nextest, PyYAML, the pinned Test262
checkout, and a warm target directory). The lane -> sandbox id map lives in
.agents/.boat-lanes.json. A stopped sandbox is resumed; a missing one is
created. scripts/boat-idle-reaper.py stops sandboxes idle for 15 minutes, so
lanes never pay for time they do not use.

Lanes share pooled sandboxes (2026-09-26). One sandbox per lane kept 24
sandboxes billed while agents were thinking, not building; related lanes now
share one, each in its own ~/amazing/wt/<lane> tree against one shared
CARGO_TARGET_DIR. Cargo's build-directory lock serializes their compiles, so
two lanes never fight over the 4 vCPUs, and dependencies stay warm for all.
`pool-report` measures each pool's busy time and overlap from
.agents/.boat-runs.log (written by linux-run.sh) so hot pools can be split or
moved to a larger type.

Usage:
  boat-lane.py ensure <lane> [--type default]   print "<user@host> <port> <id> <pool>"
  boat-lane.py stop <lane>
  boat-lane.py list
  boat-lane.py pool-report [--hours 2]
"""

from __future__ import annotations

import fcntl
import json
import os
import subprocess
import sys
import time
from pathlib import Path

BOAT = os.path.expanduser("~/.ascii/bin/boat")
TEMPLATE = os.environ.get("BOAT_TEMPLATE", "amazing-builder")
REPO = Path(__file__).resolve().parent.parent
MAIN = Path(
    subprocess.run(
        ["git", "-C", str(REPO), "rev-parse", "--path-format=absolute", "--git-common-dir"],
        capture_output=True, text=True,
    ).stdout.strip()
).parent
MAP = MAIN / ".agents" / ".boat-lanes.json"
READY = {"ready", "idle", "running"}
RUNS = MAIN / ".agents" / ".boat-runs.log"

# Lane -> pool. Lanes absent here (integrator-primary, t262-shard-K) keep a
# sandbox of their own. Pools group lanes that build the same crates, so the
# shared target directory is warm for every member.
POOLS = {
    "pool-js-a": ["js-runtime", "js-vm"],
    "pool-js-b": ["js-lang", "js-date"],
    "pool-js-lib": ["js-binary", "js-stdlib", "js-intl"],
    "pool-dom": ["dom-bindings", "html", "integrator-assist"],
    "pool-layout": ["layout", "layout-flexgrid", "css", "text"],
    "pool-paint": ["replaced-content", "compositor"],
    "pool-sys": ["net", "ipc", "storage"],
    "pool-new": ["wasm", "a11y", "devtools"],
}
POOL_OF = {lane: pool for pool, lanes in POOLS.items() for lane in lanes}


def pool_of(lane: str) -> str:
    return POOL_OF.get(lane, lane)
STOPPED = {"archived", "stopped"}


def boat(*args: str, timeout: int = 180) -> dict:
    out = subprocess.run(
        [BOAT, *args, "--json"], capture_output=True, text=True, timeout=timeout
    ).stdout
    rows = [json.loads(l) for l in out.splitlines() if l.startswith("{")]
    for row in rows:
        status = row.get("status")
        # `status` is an HTTP code on errors but a word ("resuming", "forking")
        # on progress rows.
        if row.get("event") == "error" or (isinstance(status, int) and status >= 400):
            raise SystemExit(f"boat-lane: boat {' '.join(args)}: {row.get('error') or row}")
    return rows[-1] if rows else {}


def info(sid: str) -> dict:
    d = boat("info", sid)
    return d.get("sandbox", d)


def load_map() -> dict:
    return json.loads(MAP.read_text()) if MAP.exists() else {}


def save_map(m: dict) -> None:
    MAP.parent.mkdir(parents=True, exist_ok=True)
    MAP.write_text(json.dumps(m, indent=2) + "\n")


def wait_ready(sid: str, limit: int = 300) -> dict:
    deadline = time.time() + limit
    while time.time() < deadline:
        d = info(sid)
        if d.get("state") in READY and d.get("sshEndpoint"):
            if d.get("machineProvider") != "baremetal":
                print(f"boat-lane: WARNING {sid} runs on {d.get('machineProvider')} "
                      "(2-4x slower than bare metal)", file=sys.stderr)
            return d
        time.sleep(3)
    raise SystemExit(f"boat-lane: {sid} not ready after {limit}s")


def ensure(lane: str, size: str) -> dict:
    # Parallel shard runs create sandboxes at once; serialize map updates.
    MAP.parent.mkdir(parents=True, exist_ok=True)
    with open(MAP.with_suffix(".lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        sid = _ensure_locked(pool_of(lane), size)
    return wait_ready(sid)


def _ensure_locked(lane: str, size: str) -> str:
    m = load_map()
    sid = m.get(lane)
    d = None
    if sid:
        try:
            d = info(sid)
        except SystemExit:
            d = None
    if d is None or d.get("state") in ("deleted", "error"):
        created = boat("new", "--from", TEMPLATE, "--type", size, "--no-auto-stop",
                       "-e", f"AMAZING_LANE={lane}")
        sid = created.get("id")
        m[lane] = sid
        save_map(m)
        print(f"boat-lane: created {sid} for {lane} from {TEMPLATE}", file=sys.stderr)
    elif d.get("state") in STOPPED:
        boat("resume", sid, "--no-auto-stop")
        print(f"boat-lane: resumed {sid} for {lane}", file=sys.stderr)
    return sid


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    cmd = sys.argv[1]
    if cmd == "ensure":
        lane = sys.argv[2]
        size = sys.argv[4] if len(sys.argv) > 4 and sys.argv[3] == "--type" else "default"
        d = ensure(lane, size)
        host, port = d["sshEndpoint"].rsplit(":", 1)
        print(f"user@{host} {port} {d['id']} {pool_of(lane)}")
        return 0
    if cmd == "stop":
        sid = load_map().get(pool_of(sys.argv[2]))
        if sid:
            boat("stop", sid)
        return 0
    if cmd == "list":
        for lane, sid in sorted(load_map().items()):
            try:
                d = info(sid)
                print(f"{lane:<20} {sid} {d.get('state'):<9} {d.get('type'):<8} {d.get('machineProvider')}")
            except SystemExit as e:
                print(f"{lane:<20} {sid} ? {e}")
        return 0
    if cmd == "pool-report":
        hours = float(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[2] == "--hours" else 2.0
        pool_report(hours)
        return 0
    print(__doc__)
    return 2


def pool_report(hours: float) -> None:
    """Busy and overlap per pool over the last `hours`, from linux-run.sh's log."""
    since = time.time() - hours * 3600
    runs: dict[str, list[tuple[float, float, str]]] = {}
    for line in RUNS.read_text().splitlines() if RUNS.exists() else []:
        f = line.split()
        if len(f) < 4:
            continue
        start, end, lane, pool = float(f[0]), float(f[1]), f[2], f[3]
        if end >= since:
            runs.setdefault(pool, []).append((max(start, since), end, lane))
    window = hours * 3600
    print(f"{'pool':<14} {'runs':>5} {'busy%':>6} {'overlap%':>9} {'maxpar':>6} {'avg s':>6}  lanes")
    for pool in sorted(runs):
        rs = runs[pool]
        edges = sorted([(s, 1) for s, _, _ in rs] + [(e, -1) for _, e, _ in rs])
        busy = over = 0.0
        depth = peak = 0
        prev = edges[0][0]
        for t, d in edges:
            if depth >= 1:
                busy += t - prev
            if depth >= 2:
                over += t - prev
            depth += d
            peak = max(peak, depth)
            prev = t
        avg = sum(e - s for s, e, _ in rs) / len(rs)
        lanes = ",".join(sorted({l for _, _, l in rs}))
        print(f"{pool:<14} {len(rs):>5} {100 * busy / window:>5.0f}% {100 * over / window:>8.0f}% "
              f"{peak:>6} {avg:>6.0f}  {lanes}")


if __name__ == "__main__":
    raise SystemExit(main())
