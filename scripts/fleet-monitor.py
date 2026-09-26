#!/usr/bin/env python3
"""Emit one line per NEW fleet event: a lane finishing, blocking, or asking.

Designed for the orchestrator's Monitor tool, where every stdout line becomes a
notification. So it must be selective but must not be silent on failure: a lane
that stalls or exhausts its context looks exactly like a quiet healthy lane
otherwise, and silence would be indistinguishable from progress.

Events, each emitted once:

    DONE     <lane> <ticket> -- <step>     status.json says done, ready to merge
    BLOCKED  <lane> <blocker>              lane cannot proceed
    ASK      <lane> <question>             new unresolved question
    STALL    <lane> busy but status is Nm old
    ERROR    <lane> idle after a provider error (model at capacity, rate limit)
    CONTEXT  <lane> N% left -- restart
    MODEL    <lane> running X, expected gpt-6-sol medium
    SERVER   win-001 unreachable over ssh

State is kept in .agents/.monitor-seen so a restarted monitor does not replay
history. Resolved questions, per the RESOLVED convention, disappear on their
own.

Usage: scripts/fleet-monitor.py [poll_seconds]
"""

from __future__ import annotations

import hashlib
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tmuxctl  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
SEEN = REPO / ".agents" / ".monitor-seen"
POLL = int(sys.argv[1]) if len(sys.argv) > 1 else 45
_last_disk_check = 0.0


def load_seen() -> set[str]:
    if SEEN.exists():
        return set(SEEN.read_text().split())
    return set()


def remember(seen: set[str], key: str) -> None:
    seen.add(key)
    SEEN.parent.mkdir(parents=True, exist_ok=True)
    with SEEN.open("a") as fh:
        fh.write(key + "\n")


def digest(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()[:16]


def stale_minutes(age: str) -> int:
    """Parse tmuxctl's relative age back into minutes."""
    if not age or age == "-":
        return 0
    try:
        n = int(age[:-1])
    except ValueError:
        return 0
    return {"m": n, "h": n * 60, "d": n * 1440}.get(age[-1], 0)


# Case-insensitive: a Codex update changed the status bar from "gpt-6-sol" to
# "GPT-6-Sol", which made every lane look like it had drifted.
MODEL_RE = re.compile(r"(gpt-[0-9a-z.\-]+) (minimal|low|medium|high|xhigh)", re.I)


def server_down() -> str | None:
    """Detect the build-server outage class before lanes pile up retries.

    Checks the local multiplexed master first, which costs no network round
    trip. Only if it is gone does it try one real connection, and the ssh
    config's ControlMaster makes that connection the new master.
    """
    if subprocess.run(["ssh", "-O", "check", "win-001"],
                      capture_output=True, text=True).returncode == 0:
        return None
    probe = subprocess.run(
        ["ssh", "-o", "ConnectTimeout=10", "win-001", "echo ok"],
        capture_output=True, text=True, timeout=30)
    if "ok" in probe.stdout:
        return None
    return (probe.stderr.strip().splitlines() or ["no response"])[-1][:120]


def scan(seen: set[str]) -> list[str]:
    events: list[str] = []
    reg = tmuxctl.load_registry()

    for lane in reg:
        try:
            session = tmuxctl.resolve(lane)
            raw = tmuxctl.capture_live(session)
        except SystemExit:
            continue
        st = tmuxctl.read_status(lane)
        state = str(st.get("state", "")).lower()
        ticket = st.get("ticket", "-")
        step = str(st.get("step", ""))[:110]

        # Finished work, ready to merge. Keyed on the step so a lane finishing a
        # second ticket reports again rather than being swallowed as a repeat.
        if state in ("done", "checkpointed"):
            key = digest(f"done:{lane}:{ticket}:{step}")
            if key not in seen:
                remember(seen, key)
                events.append(f"DONE     {lane} {ticket} -- {step}")

        for b in st.get("blockers", []) or []:
            key = digest(f"blocked:{lane}:{b}")
            if key not in seen:
                remember(seen, key)
                events.append(f"BLOCKED  {lane} {str(b)[:150]}")

        for q in tmuxctl.open_questions(lane):
            key = digest(f"ask:{lane}:{q}")
            if key not in seen:
                remember(seen, key)
                events.append(f"ASK      {lane} {q[:180]}")

        # Failure paths: a stalled or context-exhausted lane must not look quiet.
        if tmuxctl.state_of(raw) == "busy":
            # Measure from the file's mtime, not the agent-written "updated"
            # field: OpenCode agents sometimes write placeholder timestamps.
            status_file = tmuxctl.AGENTS_DIR / lane / "status.json"
            if status_file.exists():
                mins = int((time.time() - status_file.stat().st_mtime) // 60)
            else:
                mins = stale_minutes(tmuxctl.age_of(st.get("updated", "-")))
            # A restarted agent inherits its predecessor's status.json. Until
            # it writes its own, measure staleness from the newest brief, or a
            # fresh agent working normally looks stalled for days.
            brief = tmuxctl.AGENTS_DIR / lane / "brief.md"
            if brief.exists():
                mins = min(mins, int((time.time() - brief.stat().st_mtime) // 60))
            if mins >= tmuxctl.STALL_MINUTES:
                key = digest(f"stall:{lane}:{ticket}:{mins // 10}")
                if key not in seen:
                    remember(seen, key)
                    events.append(f"STALL    {lane} busy but status is {mins}m old")

        # A provider error ends the agent's turn and leaves it idle at the
        # prompt, which is indistinguishable from "waiting" in every other
        # signal. text sat 40 minutes after "model is at capacity" before
        # anyone noticed. An idle lane whose recent output ends in such an
        # error needs a nudge to resume.
        if tmuxctl.state_of(raw) == "idle" and state not in ("done", "checkpointed"):
            tail = "\n".join(raw.splitlines()[-15:]).lower()
            for marker in ("at capacity", "rate limit", "stream error",
                           "stream disconnected", "internal server error"):
                if marker in tail:
                    key = digest(f"err:{lane}:{marker}:{int(time.time() // 900)}")
                    if key not in seen:
                        remember(seen, key)
                        events.append(f"ERROR    {lane} idle after provider error '{marker}' -- resend to resume")
                    break

        if tmuxctl.OC_FOOTER in raw:
            # OpenCode lane: the mode line names the model.
            if tmuxctl.AGENT_MODEL_LABEL not in raw:
                key = digest(f"model:{lane}:oc")
                if key not in seen:
                    remember(seen, key)
                    events.append(f"MODEL    {lane} not running {tmuxctl.AGENT_MODEL_LABEL}")
            m = None
        else:
            m = MODEL_RE.search(raw)
        if m:
            running = f"{m.group(1)} {m.group(2)}".lower()
            expected = f"{tmuxctl.AGENT_MODEL} {tmuxctl.AGENT_EFFORT}"
            if running != expected:
                key = digest(f"model:{lane}:{running}")
                if key not in seen:
                    remember(seen, key)
                    events.append(f"MODEL    {lane} running {running}, expected {expected}")

        pct = tmuxctl.context_pct(raw)
        if pct.endswith("%"):
            left = int(pct[:-1])
            if left <= tmuxctl.CONTEXT_FLOOR:
                key = digest(f"ctx:{lane}:{left // 5}")
                if key not in seen:
                    remember(seen, key)
                    events.append(f"CONTEXT  {lane} {pct} left -- checkpoint and restart")

    # Build-host disk. Two venus hosts filled up on 2026-09-24 and killed lane
    # builds and Test262 runs mid-flight; a warning at 15 GB leaves room to
    # prune before anything breaks. Checked every ~10 minutes, not every poll.
    global _last_disk_check
    if time.time() - _last_disk_check > 600:
        _last_disk_check = time.time()
        for host in ("venus-001", "venus-003"):
            try:
                out = subprocess.run(["ssh", "-o", "ConnectTimeout=10", host,
                                      "df -BG --output=avail / | tail -1"],
                                     capture_output=True, text=True, timeout=30).stdout
                free = int(out.strip().rstrip("G") or 0)
            except (subprocess.TimeoutExpired, ValueError):
                continue
            if free < 15:
                key = digest(f"disk:{host}:{int(time.time() // 1800)}")
                if key not in seen:
                    remember(seen, key)
                    events.append(f"SERVER   {host} root disk low: {free} GB free -- prune ~/amazing/target and ~/amazing/out")

    try:
        down = server_down()
    except subprocess.TimeoutExpired:
        down = "probe timed out"
    if down:
        key = digest(f"server:{int(time.time() // 300)}")
        if key not in seen:
            remember(seen, key)
            events.append(f"SERVER   win-001 unreachable over ssh: {down}")

    return events


def main() -> int:
    seen = load_seen()
    # Prime on first run so an armed monitor does not replay the whole backlog.
    if not SEEN.exists():
        scan(seen)
        print("MONITOR  armed; existing state primed, reporting new events only",
              flush=True)
    while True:
        try:
            for line in scan(seen):
                print(line, flush=True)
        except Exception as exc:  # never let one bad poll kill the watch
            print(f"MONITOR-ERROR {type(exc).__name__}: {exc}", flush=True)
        time.sleep(POLL)


if __name__ == "__main__":
    sys.exit(main())
