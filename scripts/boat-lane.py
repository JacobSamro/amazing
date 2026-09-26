#!/usr/bin/env python3
"""Give a lane its own Boat sandbox and print how to reach it.

Each lane gets one persistent sandbox deployed from the `amazing-builder`
template (Rust 1.97.1, libvpx 1.16, nextest, PyYAML, the pinned Test262
checkout, and a warm target directory). The lane -> sandbox id map lives in
.agents/.boat-lanes.json. A stopped sandbox is resumed; a missing one is
created. scripts/boat-idle-reaper.py stops sandboxes idle for 30 minutes, so
lanes never pay for time they do not use.

Lanes share pooled sandboxes (2026-09-26). One sandbox per lane kept 24
sandboxes billed while agents were thinking, not building; related lanes now
share one, each in its own ~/amazing/wt/<lane> tree with its own
CARGO_TARGET_DIR (see target_of).
`pool-report` measures each pool's busy time and overlap from
.agents/.boat-runs.log (written by linux-run.sh) so hot pools can be split or
moved to a larger type.

Usage:
  boat-lane.py ensure <lane> [--type default]   print "<user@host> <port> <id> <pool> <target>"
  boat-lane.py heal <lane>                      restart it if its lazy filesystem died; print as ensure
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
    "pool-js-a": ["js-runtime"],
    "pool-js-vm": ["js-vm"],  # split 2026-09-26: 60% busy, 23% overlap with js-runtime
    "pool-js-b": ["js-lang"],
    "pool-js-date": ["js-date"],  # split 2026-09-26: 57% busy, 24% overlap with js-lang
    "pool-js-lib": ["js-binary", "js-stdlib", "js-intl"],
    "pool-dom": ["dom-bindings", "html"],
    "pool-layout": ["layout", "layout-flexgrid", "css", "text"],
    "pool-paint": ["replaced-content", "compositor"],
    "pool-sys": ["net", "ipc", "storage"],
    "pool-new": ["wasm", "a11y", "devtools"],
    # integrator-assist left pool-dom (disk full) then pool-new (75% busy, 28% overlap)
    # on 2026-09-26; it builds the whole `amazing` crate, so it gets its own sandbox.
    "pool-ia": ["integrator-assist"],
    "pool-desktop": ["shell"],
}
POOL_OF = {lane: pool for pool, lanes in POOLS.items() for lane in lanes}

# Machine size per pool (default: 4 vCPU/8 GB; large: 8 vCPU/16 GB at 2x rate).
# The JS pools and layout compile the biggest crates and run Test262 or the
# layout corpora, so they get large (2026-09-26). A running sandbox of the
# wrong size switches on its next resume (the idle reaper stops it first).
POOL_TYPE = {
    "pool-js-a": "large", "pool-js-vm": "large", "pool-js-b": "large",
    "pool-js-date": "large", "pool-js-lib": "large", "pool-layout": "large",
    "pool-dom": "large",  # 76% busy, 41% overlap across three lanes (2026-09-26)
}


# Large is only worth its 2x rate on bare metal. A large resume that lands on
# Hetzner (8 slower cores) is dropped back to default for a while, which has
# landed on bare metal every time (2026-09-26). The reaper reads the same rule.
DOWNSIZED = MAIN / ".agents" / ".boat-downsized.json"
DOWNSIZE_HOURS = 6


def downsized() -> dict:
    try:
        d = json.loads(DOWNSIZED.read_text())
    except (OSError, ValueError):
        return {}
    return {k: v for k, v in d.items() if v > time.time()}


def pool_type(pool: str) -> str:
    return "default" if pool in downsized() else POOL_TYPE.get(pool, "default")


def type_of(lane: str) -> str:
    return pool_type(pool_of(lane))


def pool_of(lane: str) -> str:
    return POOL_OF.get(lane, lane)


def target_of(lane: str) -> str:
    """Each lane needs its own CARGO_TARGET_DIR. Cargo hashes workspace members
    relative to the workspace root, so two lanes' trees on one sandbox produce
    the same artifact names; with one shared target a lane linked another
    lane's stale rlib (compositor, 2026-09-26). The pool's first lane keeps the
    template's warm ~/amazing/target; the others build their own once."""
    pool = POOL_OF.get(lane)
    if pool is None or POOLS[pool][0] == lane:
        return "amazing/target"
    return f"amazing/targets/{lane}"
STOPPED = {"archived", "stopped"}


# Only sandboxes named "amazing*" (the template's own name) belong to this fleet.
# Everything else on the account is the user's and is never stopped, resized or
# deleted from here (2026-09-26).
MUTATING = {"stop", "resume", "delete", "exec"}


def boat(*args: str, timeout: int = 180) -> dict:
    if args and args[0] in MUTATING and len(args) > 1 and args[1].startswith("bx_"):
        name = _name_of(args[1])
        if not name.startswith("amazing"):
            raise SystemExit(f"boat-lane: refusing `boat {args[0]}` on {args[1]} ({name!r}): not an amazing sandbox")
    return _boat(*args, timeout=timeout)


_NAMES: dict[str, str] = {}


def _name_of(sid: str) -> str:
    if sid not in _NAMES:
        d = _boat("info", sid)
        _NAMES[sid] = d.get("sandbox", d).get("name", "") or ""
    return _NAMES[sid]


def _boat(*args: str, timeout: int = 180) -> dict:
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


def endpoint(d: dict) -> str | None:
    """host:port for ssh. Bare-metal sandboxes publish sshEndpoint; Hetzner ones
    publish none but answer on their IPv4 address, port 22 (2026-09-26)."""
    if d.get("sshEndpoint"):
        return d["sshEndpoint"]
    ip = d.get("ip") or ""
    if ip and ":" not in ip:
        return f"{ip}:22"
    return None


def wait_ready(sid: str, limit: int = 300) -> dict:
    deadline = time.time() + limit
    while time.time() < deadline:
        d = info(sid)
        if d.get("state") in READY and endpoint(d):
            d["sshEndpoint"] = endpoint(d)
            if d.get("machineProvider") != "baremetal":
                print(f"boat-lane: WARNING {sid} runs on {d.get('machineProvider')} "
                      "(2-4x slower than bare metal)", file=sys.stderr)
            return d
        time.sleep(3)
    raise SystemExit(f"boat-lane: {sid} not ready after {limit}s")


KEY = os.path.expanduser("~/.ssh/ascii_box_ed25519")


def healthy(d: dict) -> bool:
    """A resumed sandbox serves /home/user from ascii-lazyfs (FUSE). Under load
    right after a resume that daemon can die, leaving every path failing with
    "Transport endpoint is not connected" until the sandbox restarts."""
    host, port = d["sshEndpoint"].rsplit(":", 1)
    try:
        r = subprocess.run(
            ["ssh", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=no",
             "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR",
             "-o", "ConnectTimeout=20", "-i", KEY, "-p", port, f"user@{host}",
             "stat -c %s ~/.amazing-env >/dev/null && mkdir -p ~/amazing/wt && touch ~/amazing/wt/.probe"],
            capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        return False
    return r.returncode == 0


def heal(lane: str) -> dict:
    """Restart the lane's sandbox when its lazy filesystem has died."""
    # The reaper may have stopped it between ensure and the run's lease;
    # resume it first rather than waiting for a ready state that never comes.
    ensure(lane, type_of(lane))
    sid = load_map().get(pool_of(lane))
    d = wait_ready(sid)
    if healthy(d):
        return d
    print(f"boat-lane: {sid} filesystem broken (lazyfs); restarting it", file=sys.stderr)
    boat("stop", sid, timeout=240)
    boat("resume", sid, "--no-auto-stop", "--type", type_of(lane), timeout=300)
    return wait_ready(sid)


def ensure(lane: str, size: str) -> dict:
    # Parallel shard runs create sandboxes at once; serialize map updates.
    MAP.parent.mkdir(parents=True, exist_ok=True)
    with open(MAP.with_suffix(".lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        sid = _ensure_locked(pool_of(lane), size)
    d = wait_ready(sid)
    if d.get("type") == "large" and d.get("machineProvider") == "hetzner" and not busy(d):
        pool = pool_of(lane)
        print(f"boat-lane: {sid} came up large on Hetzner; using default for "
              f"{DOWNSIZE_HOURS} h", file=sys.stderr)
        marks = downsized()
        marks[pool] = time.time() + DOWNSIZE_HOURS * 3600
        DOWNSIZED.write_text(json.dumps(marks))
        boat("stop", sid, timeout=240)
        with open(MAP.with_suffix(".lock"), "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            _ensure_locked(pool, "default")
        d = wait_ready(sid)
    return d


def busy(d: dict) -> bool:
    """True if another run holds a lease on this sandbox (don't restart it)."""
    host, port = d["sshEndpoint"].rsplit(":", 1)
    try:
        r = subprocess.run(
            ["ssh", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
             "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=15", "-i", str(KEY), "-p", port,
             f"user@{host}", "find ~/.amazing-busy -type f -mmin -60 | wc -l"],
            capture_output=True, text=True, timeout=60)
        return int(r.stdout.strip() or 0) > 0
    except (subprocess.TimeoutExpired, ValueError):
        return True


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
    elif d.get("state") in ("stopping", "snapshotting"):
        # Stopped under us (the reaper); let the stop finish, then resume.
        for _ in range(60):
            time.sleep(5)
            d = info(sid)
            if d.get("state") not in ("stopping", "snapshotting"):
                break
    if d is not None and d.get("state") in STOPPED:
        extra = ("--type", size) if d.get("type") and d.get("type") != size else ()
        boat("resume", sid, "--no-auto-stop", *extra, timeout=300)
        print(f"boat-lane: resumed {sid} for {lane}"
              + (f" as {size} (was {d.get('type')})" if extra else ""), file=sys.stderr)
    return sid


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    cmd = sys.argv[1]
    if cmd == "ensure":
        lane = sys.argv[2]
        size = sys.argv[4] if len(sys.argv) > 4 and sys.argv[3] == "--type" else type_of(lane)
        d = ensure(lane, size)
        host, port = d["sshEndpoint"].rsplit(":", 1)
        print(f"user@{host} {port} {d['id']} {pool_of(lane)} {target_of(lane)}")
        return 0
    if cmd == "heal":
        d = heal(sys.argv[2])
        host, port = d["sshEndpoint"].rsplit(":", 1)
        print(f"user@{host} {port} {d['id']} {pool_of(sys.argv[2])} {target_of(sys.argv[2])}")
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
