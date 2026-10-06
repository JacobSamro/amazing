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
import importlib.util
import json
import shlex
from pathlib import Path
import os
import subprocess
import time
from datetime import datetime, timezone

BOAT = os.path.expanduser("~/.ascii/bin/boat")
# Sandboxes the user runs by hand; never stop them.
KEEP = {"bx_rg6jga8q", "bx_p3hj37uf", "bx_9czp5hbk"}
LIVE_STATES = {"provisioned", "cloning", "ready", "idle", "running"}
LOAD_BUSY = 0.3
BUSY_TTL_SECONDS = 7200
RENEW_BEFORE_SECONDS = 600
PROBE = (
    "cut -d' ' -f1 /proc/loadavg; "
    "pgrep -xc 'cargo|rustc|cargo-nextest|test262_host|amazing|amazing-shell|wpt_render_compare|cc|c\\+\\+|ld|ld.lld|clang|gcc' || true; "
    # linux-run.sh leases (removed when its command exits). A run killed with
    # its ssh session can leave one behind, so they count for 60 min only;
    # long runs stay visible through the build/test process check above.
    "find ~/.amazing-busy -type f -mmin -60 2>/dev/null | wc -l"
)


def log(msg: str) -> None:
    stamp = datetime.now(timezone.utc).strftime("%H:%M:%SZ")
    print(f"{stamp} {msg}", flush=True)


def boat(*args: str, timeout: int = 60) -> dict | None:
    try:
        out = subprocess.run(
            # --json goes right after the subcommand: after `exec <id> <cmd>`
            # it would be passed to the remote command, the probe would fail,
            # and every sandbox would count as busy (never stopped, 2026-09-26).
            [BOAT, "--no-update", args[0], "--json", *args[1:]], capture_output=True, text=True, timeout=timeout
        ).stdout
    except subprocess.TimeoutExpired:
        return None
    lines = [l for l in out.splitlines() if l.startswith("{")]
    return json.loads(lines[-1]) if lines else None


_custody_spec = importlib.util.spec_from_file_location("evidence_custody", Path(__file__).with_name("evidence-custody.py"))
custody = importlib.util.module_from_spec(_custody_spec)
_custody_spec.loader.exec_module(custody)


def custody_status(sandbox_id: str, now: float) -> dict:
    return custody.decision(custody.DEFAULT_DIRECTORY, sandbox_id, now)


def archive_admission(sandbox_id: str) -> dict:
    # Ships read-only production checks; no observer output or generated helper
    # is mistaken for retained producer proof. Failure never authorizes stop.
    try:
        directory=Path(__file__).resolve().parent
        archive_code=(directory/'native-proof-archive.py').read_text()
        pack_code=(directory/'native-proof-pack.py').read_text()
        pack_loader="spec = importlib.util.spec_from_file_location('native_pack', Path(__file__).with_name('native-proof-pack.py'))\npack = importlib.util.module_from_spec(spec); spec.loader.exec_module(pack)"
        if archive_code.count(pack_loader)!=1:raise ValueError('pack helper import boundary changed')
        archive_code=archive_code.replace(pack_loader,'pack=native_pack',1)
        content_code=(directory/'native-proof-content.py').read_text()
        content_loader="spec_content = importlib.util.spec_from_file_location('native_content', Path(__file__).with_name('native-proof-content.py'))\ncontent = importlib.util.module_from_spec(spec_content); spec_content.loader.exec_module(content)"
        if archive_code.count(content_loader)!=1:raise ValueError('content helper import boundary changed')
        archive_code=archive_code.replace(content_loader,'content=native_content',1)
        guard_code=(directory/'native-stop-admission.py').read_text()
        loader="spec=importlib.util.spec_from_file_location('archive',Path(__file__).with_name('native-proof-archive.py'))\na=importlib.util.module_from_spec(spec);spec.loader.exec_module(a)"
        if guard_code.count(loader)!=1:raise ValueError('stop helper import boundary changed')
        guard_code=guard_code.replace(loader,'a=archive',1)
        code="import types,sys,json\nnative_content=types.ModuleType('native_content')\nexec("+repr(content_code)+",native_content.__dict__)\nnative_pack=types.ModuleType('native_pack')\nexec("+repr(pack_code)+",native_pack.__dict__)\narchive=types.ModuleType('archive');archive.__dict__['native_pack']=native_pack;archive.__dict__['native_content']=native_content\nexec("+repr(archive_code)+",archive.__dict__)\nguard=types.ModuleType('guard');guard.__dict__['archive']=archive\nexec("+repr(guard_code)+",guard.__dict__)\nwith archive.deadline(600):\n row=guard.admission(sys.argv[1])\nprint(json.dumps(row,separators=(',',':')))\nraise SystemExit(0 if row['admitted'] else 3)\n"
        command='timeout --kill-after=5 605 /usr/bin/python3 -I -c '+shlex.quote(code)+' '+shlex.quote(sandbox_id)
        result=boat('exec',sandbox_id,command,timeout=620)
        if not result:raise ValueError('native stop admission unavailable')
        lines=[line for line in result.get('stdout','').splitlines() if line.startswith('{')]
        row=json.loads(lines[-1]) if lines else None
        if not isinstance(row,dict) or row.get('schema')!=1 or row.get('box_id')!=sandbox_id or row.get('admitted') is not True or row.get('errors'):
            return {'admitted':False,'reason':result.get('stderr','')[-1000:],'native':row}
        if result.get('success') is not True:return {'admitted':False,'reason':'native verifier did not exit successfully','native':row}
        return row
    except (OSError,ValueError,TypeError,KeyError) as error:
        return {'admitted':False,'reason':str(error)}


def custody_extension(sandbox: dict, hold: dict, now: float) -> int:
    try:
        remaining = datetime.fromisoformat(sandbox['archiveAfter'].replace('Z', '+00:00')).timestamp()-now
    except (KeyError, TypeError, ValueError):
        return 0
    requested = hold['extension_seconds']
    return requested if remaining <= RENEW_BEFORE_SECONDS and requested > remaining else 0


LANE_MAP = Path(__file__).resolve().parent.parent / ".agents" / ".boat-lanes.json"


def wanted_types() -> dict[str, str]:
    """Sandbox id -> the size boat-lane.py's POOL_TYPE wants for it."""
    import importlib.util
    try:
        spec = importlib.util.spec_from_file_location("boat_lane", Path(__file__).with_name("boat-lane.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        lanes = json.loads(LANE_MAP.read_text())
    except Exception:
        return {}
    return {sid: mod.pool_type(key) for key, sid in lanes.items() if key.startswith("pool-")}


def managed_ids() -> set[str]:
    try:
        return set(json.loads(LANE_MAP.read_text()).values())
    except (OSError, ValueError):
        return set()


def running_sandboxes() -> list[dict]:
    listing = boat("list") or {}
    return [s for s in listing.get("sandboxes", []) if s.get("state") in LIVE_STATES]


def expires_soon(sandbox: dict, now: float) -> bool:
    value = sandbox.get("archiveAfter")
    try:
        expiry = datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
    except (AttributeError, TypeError, ValueError):
        return False
    return expiry - now <= RENEW_BEFORE_SECONDS


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


START_RESERVE_DAY = 40  # Boat allows 200 starts a day; a refused resume blocks a lane


def starts_left_this_hour() -> int | None:
    """Hourly starts left, or 0 when the daily allowance is nearly spent."""
    status = boat("billing", "status") or {}
    try:
        starts = status["limits"]["starts"]
        if int(starts["day"]["remaining"]) < START_RESERVE_DAY:
            return 0
        return int(starts["hour"]["remaining"])
    except (KeyError, TypeError, ValueError):
        return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--idle-minutes", type=float, default=30)
    # Every resume counts against Boat's 60 starts per hour. When few are left,
    # stopping early just forces a resume that may be refused, so wait longer.
    ap.add_argument("--scarce-idle-minutes", type=float, default=40)
    ap.add_argument("--poll", type=int, default=60)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--no-auto-extend", action="store_true",
                    help="Keep finite mission TTLs unchanged; continue normal idle stopping")
    ap.add_argument("--extend-custody", action="store_true",
                    help="Allow visible <=900s renewals bounded by authenticated custody deadline; --no-auto-extend still forbids renewal")
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
        wanted = wanted_types()
        for sb in live:
            sid, name = sb["id"], sb.get("name", "")
            # Only "amazing*" sandboxes boat-lane.py manages; anything else is the user's
            # (their hand-made "Box <date>" sandboxes above all), never touched.
            if (not name.startswith("amazing") or "keep" in name.lower() or sid in KEEP
                    or sid not in managed_ids()):
                continue
            hold = custody_status(sid, now)
            if hold['held']:
                idle_since.pop(sid, None)
                log(f"CUSTODY-HOLD {sid}: jobs={hold['records']} errors={hold['errors']}")
                ttl = custody_extension(sb, hold, now)
                if ttl and args.extend_custody and not args.no_auto_extend:
                    log(f"CUSTODY-EXTEND {sid}: finite {ttl}s, bounded by explicit hold deadline")
                    if not args.dry_run:
                        result = boat("extend", sid, "--ttl", str(ttl))
                        if not result or result.get('event') == 'error':
                            log(f"CUSTODY-EXTEND failed {sid}")
                continue
            busy = is_busy(sid)
            if busy is not False:
                idle_since.pop(sid, None)
                if busy is True and not args.no_auto_extend and expires_soon(sb, now):
                    log(f"EXTEND {sid}: active work, finite TTL {BUSY_TTL_SECONDS}s")
                    if not args.dry_run:
                        result = boat("extend", sid, "--ttl", str(BUSY_TTL_SECONDS))
                        if not result or result.get("event") == "error":
                            log(f"EXTEND failed {sid}; active run may expire")
                continue
            # A busy pool never idles long enough to pick up a new size, so a
            # size change is applied in the first gap between runs: stop it now
            # and boat-lane.py resumes it at the wanted size.
            want = wanted.get(sid)
            if want and sb.get("type") and sb.get("type") != want and is_busy(sid) is False and not custody_status(sid, time.time())["held"]:
                archive=archive_admission(sid)
                if not archive.get('admitted'):
                    log(f"ARCHIVE-HOLD {sid}: {archive}");continue
                if is_busy(sid) is not False or custody_status(sid,time.time())['held']:
                    idle_since.pop(sid,None);continue
                log(f"RESIZE-STOP {sid} ({sb.get('type')} -> {want} on next resume)")
                if not args.dry_run:
                    boat("stop", sid, timeout=120)
                idle_since.pop(sid, None)
                continue
            first = idle_since.setdefault(sid, now)
            idle_min = (now - first) / 60
            if idle_min >= limit:
                # Probe once more: a run may have started since the last poll.
                if is_busy(sid) is not False or custody_status(sid, time.time())["held"]:
                    idle_since.pop(sid, None)
                    continue
                archive=archive_admission(sid)
                if not archive.get('admitted'):
                    log(f"ARCHIVE-HOLD {sid}: {archive}");continue
                if is_busy(sid) is not False or custody_status(sid,time.time())['held']:
                    idle_since.pop(sid,None);continue
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
