#!/usr/bin/env python3
"""Give a lane its own Boat sandbox and print how to reach it.

Each lane gets one persistent sandbox deployed from the `amazing-builder`
template (Rust 1.97.1, libvpx 1.16, nextest, PyYAML, the pinned Test262
checkout, and a warm target directory). The lane -> sandbox id map lives in
.agents/.boat-lanes.json. A stopped sandbox is resumed; a missing one is
created. scripts/boat-idle-reaper.py stops sandboxes idle for 15 minutes, so
lanes never pay for time they do not use.

Usage:
  boat-lane.py ensure <lane> [--type default]   print "<user@host> <port> <id>"
  boat-lane.py stop <lane>
  boat-lane.py list
"""

from __future__ import annotations

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
STOPPED = {"archived", "stopped"}


def boat(*args: str, timeout: int = 180) -> dict:
    out = subprocess.run(
        [BOAT, *args, "--json"], capture_output=True, text=True, timeout=timeout
    ).stdout
    rows = [json.loads(l) for l in out.splitlines() if l.startswith("{")]
    for row in rows:
        if row.get("event") == "error" or (row.get("status", 0) or 0) >= 400:
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
            return d
        time.sleep(3)
    raise SystemExit(f"boat-lane: {sid} not ready after {limit}s")


def ensure(lane: str, size: str) -> dict:
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
    d = wait_ready(sid)
    if d.get("machineProvider") != "baremetal":
        print(f"boat-lane: WARNING {sid} for {lane} runs on {d.get('machineProvider')} "
              "(2-4x slower than bare metal)", file=sys.stderr)
    return d


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
        print(f"user@{host} {port} {d['id']}")
        return 0
    if cmd == "stop":
        sid = load_map().get(sys.argv[2])
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
    print(__doc__)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
