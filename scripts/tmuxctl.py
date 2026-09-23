#!/usr/bin/env python3
"""Drive the Codex lane agents running in tmux without flooding the caller's context.

Why this exists
---------------
Reading a Codex TUI pane directly is expensive and unreliable: every capture
carries the startup banner, the hook warnings, the usage-limit notice, the
animated braille spinner, and a redraw of the prompt you just sent. A single
`tmux capture-pane` costs hundreds of useless tokens and still wraps replies
mid-token, so the useful line arrives split across rows.

This utility fixes that two ways:

1. **Screen scraping is filtered and structural.** `reply`/`tail` parse the
   pane into transcript events (`*` agent message, `|_` tool output) and strip
   the banner/spinner/composer noise, so a reply costs a few lines instead of a
   screenful.
2. **Real reporting goes through files, not the screen.** Agents write
   `.agents/<lane>/status.json` and `.agents/<lane>/report.md`; `status` reads
   those. Screen scraping is then only needed for liveness, never for results.

Long briefs are delivered the same way: `brief` writes the text to a file and
sends the agent a one-line pointer, which avoids pasting multi-line prompts
into a TUI composer (newlines submit early and corrupt the prompt).

Usage
-----
    scripts/tmuxctl.py adopt --lanes js-lang,layout,css
    scripts/tmuxctl.py list
    scripts/tmuxctl.py status
    scripts/tmuxctl.py brief layout path/to/brief.md
    scripts/tmuxctl.py send layout "run cargo test -p amazing-layout"
    scripts/tmuxctl.py wait layout --timeout 900
    scripts/tmuxctl.py reply layout
    scripts/tmuxctl.py report layout

Every command prints compact text. `raw` is the deliberate escape hatch when
something is wrong and the unfiltered pane is genuinely needed.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def _main_checkout(start: Path) -> Path:
    """The primary checkout, even when this script runs from a lane worktree.

    Coordination state -- the agent registry, lane status files, build slots,
    refresh holds -- lives only in the primary checkout's .agents/. A worktree
    has its own empty .agents/, so resolving it relative to this file silently
    gave each lane a private copy: an empty registry and, worse, a private set
    of build slots, which meant the slot limit was not global at all.
    """
    try:
        common = subprocess.run(
            ["git", "-C", str(start), "rev-parse", "--path-format=absolute",
             "--git-common-dir"], capture_output=True, text=True, check=True
        ).stdout.strip()
        return Path(common).parent
    except (subprocess.CalledProcessError, FileNotFoundError):
        return start


MAIN = _main_checkout(REPO)
AGENTS_DIR = MAIN / ".agents"
REGISTRY = AGENTS_DIR / "agents.json"

DEFAULT_SESSION_PREFIX = "termirust-amazing-"

# Lane ownership. An agent may only write inside its own paths; anything else is
# a collision waiting to happen, so `doctor` reports it as a violation. Shared
# files (ARCHITECTURE.md, README.md, todo.md, root Cargo.toml) belong to the
# integrator alone and appear in no lane.
LANE_PATHS = {
    # amazing-js is split between two lanes. js-lang owns the front half
    # (lexing, parsing, AST) and amazing-regex; js-runtime owns the execution
    # half. ast.rs is the contract: js-lang owns it and only adds to it.
    "js-lang": [
        "crates/amazing-js/src/lexer.rs",
        "crates/amazing-js/src/parser.rs",
        "crates/amazing-js/src/ast.rs",
        "crates/amazing-js/src/tokens.rs",
        "crates/amazing-regex/",
    ],
    "js-runtime": [
        "crates/amazing-js/src/interp.rs",
        "crates/amazing-js/src/interp_builtins.rs",
        "crates/amazing-js/src/interp_tests.rs",
        "crates/amazing-js/src/value.rs",
        "crates/amazing-js/src/env.rs",
        "crates/amazing-js/src/vm/",
        "crates/amazing-js/src/gc/",
    ],
    "dom-bindings": ["crates/amazing-dom/", "crates/amazing-bindings/"],
    "layout": ["crates/amazing-layout/"],
    "css": ["crates/amazing-css/"],
    "net": [
        "crates/amazing-net/",
        "crates/amazing-net-tls/",
        "crates/amazing-net-brotli/",
        "crates/amazing-fetch/",
    ],
    "replaced-content": [
        "crates/amazing-paint/",
        "crates/amazing-media/",
        "crates/amazing-image/",
    ],
    "html": ["crates/amazing-html/"],
    # Drains the integrator's routed queue: driver-side work the engine lanes
    # correctly refused to do. Shell files are granted individually below;
    # the exclusive viewport grant is limited to natural intrinsic dimensions.
    "integrator-assist": ["crates/amazing/"],
}
# Narrow, temporary grants recorded here so `doctor` does not cry wolf about an
# exception the orchestrator deliberately made. Remove one when its ticket lands.
LANE_GRANTS = {
    # DOM-1 needs a subcommand entry point; the integrator still owns main.rs
    # beyond the single dispatch line, and is splitting it separately.
    "dom-bindings": ["crates/amazing/src/cmd/", "crates/amazing/src/main.rs"],
    # RC-1's shell grant is RETIRED: that lane confirmed it needs no remaining
    # shell or viewport access, and engine_viewport.rs transferred exclusively
    # to integrator-assist. Two lanes never share a shell file.
    "replaced-content": [],
}
# DOM-2 adds NodeKind variants, which breaks exhaustive matches in 9 files
# across 5 crates. Granted for ADDED MATCH ARMS ONLY, verified at merge; the
# shell's two matches stay with the integrator because another lane holds that
# grant and two lanes must not edit the shell at once.
# JS-2 adds the amazing-js -> amazing-regex dependency edge, which only cargo
# can record. Granted for that single edge, verified by diff at merge.
LANE_GRANTS["js-lang"] = [
    "Cargo.lock",
    "scripts/test262.py",
    # JS-3 needs a real Test262 host binary; it lives with the front half
    # the lane owns, not in the runtime half js-runtime took.
    "crates/amazing-js/examples/test262_host.rs",
]
LANE_GRANTS["integrator-assist"] = [
    "crates/amazing-shell-gpui/src/session_state.rs",
    "crates/amazing-shell-gpui/src/dom_runtime.rs",
    # IA-1 routed harness fixes: WPT-root resources and Git-free pinned
    # revision validation. Read the rewrite diff before integration.
    "scripts/run-render-wpt.ps1",
    # Transferred from replaced-content once that lane confirmed it needed no
    # remaining shell access. Scope is the natural-intrinsic-dimensions
    # migration, not the shell at large.
    "crates/amazing-shell-gpui/src/engine_viewport.rs",
]
# CSS-3 rewires the driver's stylesheet collection to preserve per-sheet
# provenance. Granted for the collection and compute call sites only.
LANE_GRANTS["css"] = [
    "crates/amazing/src/main.rs",
    # CSS-3 also needs an ADDITIVE &[Stylesheet] entry point on layout,
    # whose public API currently takes CSS as &str so provenance cannot
    # flow through. Additive signature only; no layout behaviour changes.
    "crates/amazing-layout/src/lib.rs",
    "crates/amazing/tests/css3_stylesheets.rs",
]
LANE_GRANTS["dom-bindings"] += [
    "crates/amazing-html/src/tree.rs",
    "crates/amazing-layout/src/box_tree.rs",
    "crates/amazing-layout/src/lib.rs",
    "crates/amazing-css/src/selectors.rs",
    "crates/amazing/src/gui.rs",
    "crates/amazing/tests/phase2_style.rs",
]


# Grants come in two shapes and they are not interchangeable. Most are
# ADDITIVE -- an added match arm, an added dispatch line, an added entry point --
# where any deletion means the grant was exceeded. A few are REWRITE grants,
# where replacing existing code IS the ticket: CSS-3 replaces the driver's
# stylesheet concatenation with per-sheet collection, so deletions are expected
# and the diff needs reading rather than rejecting.
LANE_GRANTS_REWRITE = {
    "css": ["crates/amazing/src/main.rs"],
    "js-lang": ["scripts/test262.py", "Cargo.lock"],
    "integrator-assist": [
        "scripts/run-render-wpt.ps1",
        # Natural-intrinsic-dimensions migration only; read the diff at merge.
        "crates/amazing-shell-gpui/src/engine_viewport.rs",
    ],
}


def grant_is_rewrite(lane: str, path: str) -> bool:
    return any(path.startswith(p) for p in LANE_GRANTS_REWRITE.get(lane, []))


def owned_paths(lane: str) -> list[str]:
    """Paths this lane may write: its crates, its scratch, its docs, its evidence."""
    return (
        LANE_PATHS.get(lane, [])
        + LANE_GRANTS.get(lane, [])
        + [
            f".agents/{lane}/",
            f"docs/lanes/{lane}",
            f"tests/wpt-{lane}/",
        ]
    )


# Every lane runs the same model. A bare `codex --yolo` inherits whatever the
# local default happens to be, so a restart could silently move a lane onto a
# different model or effort -- two lanes had already drifted this way.
AGENT_MODEL = "gpt-6-sol"
AGENT_EFFORT = "medium"
AGENT_LAUNCH = f'codex --yolo -m {AGENT_MODEL} -c model_reasoning_effort="{AGENT_EFFORT}"'

# Context below this fraction means the agent must checkpoint and be restarted
# before it loses the thread mid-ticket.
CONTEXT_FLOOR = 25
# A busy agent that has not updated status.json in this long is probably stuck.
STALL_MINUTES = 20

# Codex composer placeholder; present exactly when the input box is empty.
IDLE_MARKER = "Ask Codex to do anything"
# Shown by the TUI while a turn is running.
BUSY_MARKERS = ("Esc to interrupt", "esc to interrupt", "Working", "Thinking")

SPINNER = re.compile(r"[⠀-⣿]")
# Banner box art, but NOT U+2514 -- the TUI uses that elbow to mark tool output.
BOXDRAW = re.compile(r"[─-┓┕-╿]")
# The composer caret. Everything from here down is live input, not transcript.
COMPOSER = "›"

# Startup banner, plugin warnings and status bar. None of it is ever useful.
NOISE = tuple(
    re.compile(p)
    for p in (
        r"^\s*Tip:",
        r"clamping SessionEnd hook",
        r"\.codex/plugins",
        r"hooks/hooks\.json",
        r"usage limit resets",
        r"Run /usage",
        r"chatgpt\.com/codex",
        r"app-landing-page",
        r"^\s*app' or visit",
        r"^\s*model:\s",
        r"^\s*directory:\s",
        r"^\s*permissions:\s",
        r"Context \d+%",
        r"(?i)gpt-[\w.\-]+ (medium|high|low|minimal|xhigh)",
        r"You have \d+ usage limit",
        r"skills context budget",
        r"Disable unused skills",
        r"weekly \d+",
        re.escape(IDLE_MARKER),
    )
)

CONTEXT_RE = re.compile(r"Context (\d+)%")
# The TUI marks agent messages with a bullet and tool output with an elbow.
EVENT_RE = re.compile(r"^\s*[•●▶]\s+(.*)$")
TOOL_RE = re.compile(r"^\s*└\s+(.*)$")
DONE_RE = re.compile(r"^\s*done\s+\d+:\d+\s*(AM|PM)?\s*$", re.IGNORECASE)


# ---------------------------------------------------------------- tmux plumbing


def tmux(args: list[str], check: bool = False) -> str:
    result = subprocess.run(
        ["tmux", *args], capture_output=True, text=True, check=check
    )
    return result.stdout


def capture(pane: str, scrollback: int = 200) -> str:
    """Capture a pane. scrollback=0 means the visible screen only.

    Live state (busy/idle, context %, composer contents) MUST be read from the
    visible screen. Scrollback holds older redraw frames, and a frame captured
    while the agent was working still contains "Esc to interrupt" -- so reading
    state from scrollback reports an idle agent as permanently busy.
    """
    args = ["capture-pane", "-p", "-t", pane]
    if scrollback > 0:
        args[2:2] = ["-S", f"-{scrollback}"]
    return tmux(args)


def capture_live(pane: str) -> str:
    """The visible screen: the only valid source for busy/idle/context state."""
    return capture(pane, 0)


def pane_cwd(session: str) -> str:
    """The pane's current directory, read live.

    Never trust the cwd cached at adopt time: an agent relaunched into its
    worktree keeps the same session name but changes directory, and an
    isolation check against a stale value would silently pass.
    """
    return tmux(
        ["display-message", "-p", "-t", session, "#{pane_current_path}"]
    ).strip()


def discover(prefix: str = DEFAULT_SESSION_PREFIX) -> list[dict]:
    """Find candidate agent panes, newest session last."""
    fmt = "#{session_name}\t#{pane_id}\t#{pane_pid}\t#{pane_current_path}"
    found = []
    for line in tmux(["list-panes", "-a", "-F", fmt]).splitlines():
        parts = line.split("\t")
        if len(parts) != 4:
            continue
        session, pane_id, pane_pid, cwd = parts
        if not session.startswith(prefix):
            continue
        # Only panes actually running a codex CLI underneath.
        children = subprocess.run(
            ["pgrep", "-P", pane_pid], capture_output=True, text=True
        ).stdout.split()
        is_codex = any(
            "codex" in subprocess.run(
                ["ps", "-o", "command=", "-p", pid], capture_output=True, text=True
            ).stdout
            for pid in children
        )
        if is_codex:
            found.append({"session": session, "pane": pane_id, "cwd": cwd})
    return sorted(found, key=lambda a: a["session"])


# ---------------------------------------------------------------- registry


def load_registry() -> dict:
    if REGISTRY.exists():
        return json.loads(REGISTRY.read_text())
    return {}


def save_registry(reg: dict) -> None:
    AGENTS_DIR.mkdir(parents=True, exist_ok=True)
    REGISTRY.write_text(json.dumps(reg, indent=2) + "\n")


def resolve(target: str) -> str:
    """Map a lane name, session name, or numeric suffix to a tmux target."""
    reg = load_registry()
    if target in reg:
        return reg[target]["session"]
    if target.startswith(DEFAULT_SESSION_PREFIX):
        return target
    candidate = DEFAULT_SESSION_PREFIX + target
    sessions = {a["session"] for a in discover()}
    if candidate in sessions:
        return candidate
    sys.exit(f"tmuxctl: unknown agent '{target}' (try: tmuxctl.py list)")


def lanes_or_all(targets: list[str]) -> list[str]:
    return targets if targets else list(load_registry().keys())


# ---------------------------------------------------------------- pane parsing


def clean_lines(raw: str) -> list[str]:
    """Strip spinner/banner noise and cut the live composer off the bottom.

    The composer sits at the end of every capture and redraws the prompt that
    was just sent, so leaving it in would echo each instruction back as if the
    agent had said it.
    """
    out = []
    skipping = False
    for line in raw.splitlines():
        line = SPINNER.sub("", line)
        if line.lstrip().startswith(COMPOSER):
            skipping = True  # composer block: the prompt echoed back at us
            continue
        line = BOXDRAW.sub("", line).rstrip()
        if not line.strip():
            continue
        if skipping:
            # The composer block runs until the next real transcript marker.
            if EVENT_RE.match(line) or TOOL_RE.match(line):
                skipping = False
            elif line.startswith((" ", "\t")):
                continue
            else:
                skipping = False
        if any(p.search(line) for p in NOISE):
            continue
        out.append(line)
    return out


def state_of(raw: str) -> str:
    if any(m in raw for m in BUSY_MARKERS):
        return "busy"
    if IDLE_MARKER in raw:
        return "idle"
    return "unknown"


def context_pct(raw: str) -> str:
    matches = CONTEXT_RE.findall(raw)
    return f"{matches[-1]}%" if matches else "?"


def composer_has_text(raw: str) -> bool:
    """True when the input box holds an unsubmitted prompt.

    Codex renders *submitted* user messages with the same caret as the live
    composer, so the first caret line on screen is usually old transcript. The
    composer is always the last one.
    """
    body = None
    for line in raw.splitlines():
        stripped = SPINNER.sub("", line).strip()
        if stripped.startswith(COMPOSER):
            body = stripped.lstrip(COMPOSER).strip()
    if body is None:
        return False
    return bool(body) and IDLE_MARKER not in body


def events(raw: str, include_tools: bool = True) -> list[str]:
    """Parse the pane into transcript events, rejoining the TUI's hard wraps."""
    blocks: list[list[str]] = []
    kind: list[str] = []
    for line in clean_lines(raw):
        if DONE_RE.match(line):
            continue
        m = EVENT_RE.match(line)
        if m:
            blocks.append([m.group(1).strip()])
            kind.append("msg")
            continue
        t = TOOL_RE.match(line)
        if t:
            blocks.append([t.group(1).strip()])
            kind.append("tool")
            continue
        if blocks and line.startswith((" ", "\t")):
            blocks[-1].append(line.strip())

    # A pane's scrollback holds many redraw frames of the same conversation, so
    # the same event appears repeatedly. Collapse consecutive duplicates.
    rendered: list[str] = []
    for block, k in zip(blocks, kind):
        if k == "tool" and not include_tools:
            continue
        text = join_wrapped(block)
        if not text:
            continue
        line = ("  |_ " if k == "tool" else "* ") + text
        if rendered and rendered[-1] == line:
            continue
        rendered.append(line)
    return dedupe_frames(rendered)


def dedupe_frames(lines: list[str]) -> list[str]:
    """Drop an earlier run of lines that a later run repeats verbatim.

    The TUI redraws the whole screen, so `capture-pane` scrollback contains the
    same conversation several times over. Keep only the newest copy.
    """
    n = len(lines)
    for size in range(n // 2, 0, -1):
        if lines[-size:] == lines[-2 * size : -size]:
            return dedupe_frames(lines[:-size])
    return lines


def join_wrapped(parts: list[str]) -> str:
    """Rejoin TUI-wrapped rows.

    The TUI breaks on width, not on words, so a path can split mid-token. Join
    without a space when both sides look like one token was severed; otherwise
    join with a space.
    """
    if not parts:
        return ""
    text = parts[0]
    for nxt in parts[1:]:
        if not nxt:
            continue
        severed = text.endswith(("/", "-", "_", ".", "\\")) or (
            text and text[-1].isalnum() and nxt[0].isalnum() and len(text) > 30
        )
        text += nxt if severed else " " + nxt
    return text


# ---------------------------------------------------------------- lane state files


def lane_dir(lane: str) -> Path:
    d = AGENTS_DIR / lane
    d.mkdir(parents=True, exist_ok=True)
    return d


def read_status(lane: str) -> dict:
    path = AGENTS_DIR / lane / "status.json"
    if not path.exists():
        return {}
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError:
        return {"state": "corrupt-status-json"}


def age_of(stamp: str) -> str:
    """Render an ISO-8601 status timestamp as a short relative age."""
    if not stamp or stamp == "-":
        return "-"
    try:
        from datetime import datetime, timezone

        when = datetime.fromisoformat(stamp.replace("Z", "+00:00"))
        if when.tzinfo is None:
            when = when.replace(tzinfo=timezone.utc)
        secs = int((datetime.now(timezone.utc) - when).total_seconds())
    except ValueError:
        return "?"
    if secs < 0:
        return "0m"
    if secs < 3600:
        return f"{secs // 60}m"
    if secs < 86400:
        return f"{secs // 3600}h"
    return f"{secs // 86400}d"


def open_questions(lane: str) -> list[str]:
    """Bulleted questions a lane is still waiting on.

    A question stays in the file as a record once answered, so anything marked
    RESOLVED is dropped -- otherwise settled items pile up in the dashboard and
    drown the ones that still need a decision.
    """
    out = []
    for line in tail_file(lane, "questions.md", 100):
        if not line.lstrip().startswith(("-", "*")):
            continue
        body = line.lstrip("-* ").strip()
        if body.upper().startswith(("RESOLVED", "ANSWERED", "DONE")):
            continue
        out.append(body)
    return out


def tail_file(lane: str, name: str, n: int) -> list[str]:
    path = AGENTS_DIR / lane / name
    if not path.exists():
        return []
    lines = [l.rstrip() for l in path.read_text().splitlines() if l.strip()]
    return lines[-n:]


# ---------------------------------------------------------------- commands


def cmd_adopt(args) -> int:
    found = discover(args.prefix)
    lanes = [l.strip() for l in args.lanes.split(",") if l.strip()]
    if len(lanes) > len(found):
        sys.exit(f"tmuxctl: {len(lanes)} lanes requested but {len(found)} agents found")
    reg = {}
    for lane, agent in zip(lanes, found):
        reg[lane] = {"session": agent["session"], "pane": agent["pane"], "cwd": agent["cwd"]}
        lane_dir(lane)
    save_registry(reg)
    for lane, meta in reg.items():
        print(f"{lane:<18} {meta['session']}")
    return 0


def cmd_assign(args) -> int:
    reg = load_registry()
    if args.target not in reg:
        sys.exit(f"tmuxctl: unknown lane '{args.target}'")
    reg[args.target]["worktree"] = os.path.abspath(args.worktree)
    save_registry(reg)
    print(f"{args.target}: worktree -> {reg[args.target]['worktree']}")
    return 0


def cmd_list(args) -> int:
    reg = load_registry()
    if reg:
        for lane, meta in reg.items():
            raw = capture_live(meta["session"])
            print(f"{lane:<18} {meta['session']:<26} {state_of(raw):<8} ctx={context_pct(raw)}")
    else:
        for agent in discover(args.prefix):
            raw = capture_live(agent["session"])
            print(f"{'(unassigned)':<18} {agent['session']:<26} {state_of(raw):<8} ctx={context_pct(raw)}")
    return 0


def cmd_status(args) -> int:
    rows = []
    for lane in lanes_or_all(args.targets):
        session = resolve(lane)
        raw = capture_live(session)
        st = read_status(lane)
        rows.append(
            {
                "lane": lane,
                "tmux": state_of(raw),
                "ctx": context_pct(raw),
                "ticket": st.get("ticket", "-"),
                "step": st.get("step", "-"),
                "updated": st.get("updated", "-"),
                "blockers": st.get("blockers", []),
            }
        )
    if args.json:
        print(json.dumps(rows, indent=2))
        return 0
    for r in rows:
        blockers = f"  BLOCKED: {'; '.join(r['blockers'])}" if r["blockers"] else ""
        print(f"{r['lane']:<18} {r['tmux']:<8} ctx={r['ctx']:<5} {r['ticket']:<16} {r['step']}{blockers}")
    return 0


def submit(session: str, text: str, attempts: int = 2, window: float = 10.0) -> bool:
    """Type a prompt and confirm the TUI accepted it.

    Enter right after a literal send is unreliable -- the TUI may still be
    consuming the paste, so the keystroke lands in the composer instead of
    submitting. But blindly re-sending Enter is worse: once the first one lands,
    every extra Enter submits another (possibly partial) message. So press once,
    then poll for acceptance, and only press again if the prompt is provably
    still sitting in the composer.
    """
    tmux(["send-keys", "-t", session, "-l", text])
    time.sleep(0.5)
    for _ in range(attempts):
        tmux(["send-keys", "-t", session, "Enter"])
        deadline = time.time() + window
        while time.time() < deadline:
            time.sleep(0.5)
            raw = capture_live(session)
            # Accepted once the composer is empty or the turn is already running.
            if not composer_has_text(raw) or state_of(raw) == "busy":
                return True
    return False


def cmd_send(args) -> int:
    session = resolve(args.target)
    text = " ".join(args.message).replace("\n", " ").strip()
    if not text:
        sys.exit("tmuxctl: empty message")
    ok = submit(session, text)
    print(f"{args.target}: {'sent' if ok else 'SUBMIT FAILED (prompt left in composer)'}")
    return 0 if ok else 1


def cmd_brief(args) -> int:
    """Deliver a long brief as a file plus a one-line pointer."""
    session = resolve(args.target)
    src = Path(args.file)
    if not src.exists():
        sys.exit(f"tmuxctl: no such brief file: {src}")
    dest = lane_dir(args.target) / "brief.md"
    dest.write_text(src.read_text())
    # Absolute, because the agent's cwd is its worktree, not this repo. A
    # relative path would resolve inside the worktree and silently miss.
    pointer = (
        f"Read {dest} and follow it exactly. "
        f"Report progress by writing {dest.parent}/status.json as described there."
    )
    ok = submit(session, pointer)
    print(f"{args.target}: brief -> {dest} ({'sent' if ok else 'SUBMIT FAILED'})")
    return 0 if ok else 1


def cmd_broadcast(args) -> int:
    text = " ".join(args.message).replace("\n", " ").strip()
    rc = 0
    for lane in lanes_or_all(args.lanes.split(",") if args.lanes else []):
        ok = submit(resolve(lane), text)
        print(f"{lane}: {'sent' if ok else 'SUBMIT FAILED'}")
        rc |= 0 if ok else 1
    return rc


def cmd_wait(args) -> int:
    """Block until targets go idle. Prints one line per agent, nothing else."""
    targets = lanes_or_all(args.targets)
    deadline = time.time() + args.timeout
    pending = {t: resolve(t) for t in targets}
    done = {}
    while pending and time.time() < deadline:
        for lane, session in list(pending.items()):
            raw = capture_live(session)
            if state_of(raw) == "idle" and not composer_has_text(raw):
                done[lane] = context_pct(raw)
                del pending[lane]
        if pending:
            time.sleep(args.interval)
    for lane, ctx in done.items():
        print(f"{lane}: idle ctx={ctx}")
    for lane in pending:
        print(f"{lane}: STILL BUSY after {args.timeout}s")
    return 1 if pending else 0


def cmd_reply(args) -> int:
    raw = capture(resolve(args.target), args.scrollback)
    evs = events(raw, include_tools=not args.no_tools)
    for line in evs[-args.n :]:
        print(line)
    if not evs:
        print("(no transcript events parsed; use `raw` to inspect)")
    return 0


def cmd_tail(args) -> int:
    for line in clean_lines(capture(resolve(args.target), args.scrollback))[-args.n :]:
        print(line)
    return 0


def cmd_raw(args) -> int:
    print(capture(resolve(args.target), args.scrollback))
    return 0


def cmd_interrupt(args) -> int:
    for lane in lanes_or_all(args.targets):
        tmux(["send-keys", "-t", resolve(lane), "Escape"])
        print(f"{lane}: interrupt sent")
    return 0


def cmd_digest(args) -> int:
    """The fleet dashboard: everything worth knowing about all lanes, one screen.

    This is the command to run while orchestrating. It reads agent-written files
    rather than scraping panes, so it costs the same few lines no matter how much
    the agents have actually done.
    """
    reg = load_registry()
    print(f"{'LANE':<18} {'TMUX':<7} {'CTX':<5} {'TICKET':<16} {'AGE':<5} STEP")
    problems: list[str] = []
    for lane in reg:
        session = resolve(lane)
        raw = capture_live(session)
        tmux_state = state_of(raw)
        ctx = context_pct(raw)
        st = read_status(lane)
        age = age_of(st.get("updated", "-"))
        print(
            f"{lane:<18} {tmux_state:<7} {ctx:<5} {st.get('ticket', '-'):<16} "
            f"{age:<5} {st.get('step', '-')}"
        )
        for b in st.get("blockers", []):
            problems.append(f"BLOCKED  {lane}: {b}")

    for lane in reg:
        for q in open_questions(lane):
            problems.append(f"ASK      {lane}: {q}")

    if problems:
        print("\n-- needs attention --")
        for p in problems:
            print(p)
    return 0


def cmd_doctor(args) -> int:
    """Safety preflight. Catches the three things that actually break a fleet.

    1. An agent working in the shared checkout instead of its own worktree.
    2. An agent writing outside the paths its lane owns.
    3. An agent that has stalled, or is about to run out of context.
    """
    reg = load_registry()
    findings: list[str] = []
    for lane, meta in reg.items():
        worktree = meta.get("worktree")
        cwd = pane_cwd(resolve(lane))

        # 1. Isolation.
        if not worktree:
            findings.append(f"ISOLATION {lane}: no worktree assigned (still shared checkout)")
        elif os.path.realpath(cwd) != os.path.realpath(worktree):
            findings.append(f"ISOLATION {lane}: cwd={cwd} but worktree={worktree}")

        # 2. Path ownership -- only meaningful once the lane has its own tree.
        # In a shared checkout every lane sees every other lane's edits plus any
        # pre-existing dirt, so the check would report the same noise six times.
        if worktree:
            # -uall expands untracked directories; git otherwise collapses
            # them to "docs/lanes/" and a lane's own file looks like a stray.
            changed = subprocess.run(
                ["git", "-C", worktree, "status", "--porcelain", "-uall"],
                capture_output=True,
                text=True,
            ).stdout.splitlines()
            allowed = owned_paths(lane)
            strays = [
                p
                for p in (e[3:].strip().strip('"') for e in changed)
                if p and not p.startswith(".agents/") and not any(p.startswith(a) for a in allowed)
            ]
            for path in strays[:5]:
                findings.append(f"OWNERSHIP {lane}: touched {path} (outside its lane)")
            if len(strays) > 5:
                findings.append(f"OWNERSHIP {lane}: ... and {len(strays) - 5} more stray paths")

        # 3. Liveness.
        raw = capture_live(resolve(lane))
        pct = context_pct(raw)
        if pct.endswith("%") and int(pct[:-1]) <= CONTEXT_FLOOR:
            findings.append(
                f"CONTEXT   {lane}: {pct} left -- checkpoint and restart (see `handoff`)"
            )
        st = read_status(lane)
        if state_of(raw) == "busy":
            age = age_of(st.get("updated", "-"))
            if age.endswith(("h", "d")) or (age.endswith("m") and int(age[:-1]) >= STALL_MINUTES):
                findings.append(f"STALL     {lane}: busy but status.json is {age} old")

    if not findings:
        print("doctor: all lanes isolated, in-lane, live")
        return 0
    for f in findings:
        print(f)
    return 1


def cmd_log(args) -> int:
    lines = tail_file(args.target, "log.md", args.n)
    if not lines:
        print(f"(no .agents/{args.target}/log.md yet)")
        return 1
    for line in lines:
        print(line)
    return 0


def cmd_questions(args) -> int:
    """Collect every open cross-lane request and decision the fleet is waiting on."""
    found = False
    lanes = lanes_or_all(args.targets)
    # Report-only integrators have no tmux registry entry. Their questions must
    # still reach the daily dashboard; do not register a fictitious session.
    if not args.targets:
        lanes = list(dict.fromkeys(lanes + [
            path.parent.name for path in sorted(AGENTS_DIR.glob("*/questions.md"))
            if (path.parent / "status.json").is_file()
        ]))
    for lane in lanes:
        items = open_questions(lane)
        if items:
            found = True
            print(f"[{lane}]")
            for q in items:
                print(f"  {q}")
    if not found:
        print("no open questions")
    return 0


def cmd_handoff(args) -> int:
    """Tell an agent to checkpoint itself so a fresh agent can take over.

    Context exhaustion is the normal end of an agent's life, not an error. The
    replacement reads report.md and continues without re-deriving the work.
    """
    msg = (
        f"CHECKPOINT NOW. You are low on context. Stop the current step and write "
        f".agents/{args.target}/report.md containing: (1) the ticket and its exit criteria, "
        f"(2) what you changed, file by file, (3) measured gate numbers before/after, "
        f"(4) exactly what the next agent must do first, (5) anything half-finished or risky. "
        f"Then update .agents/{args.target}/status.json with state=\"checkpointed\". "
        f"Do not start new work."
    )
    ok = submit(resolve(args.target), msg)
    print(f"{args.target}: handoff {'requested' if ok else 'SUBMIT FAILED'}")
    return 0 if ok else 1


def cmd_restart(args) -> int:
    """Replace a lane's agent with a fresh one in the same worktree.

    Context exhaustion is the normal end of an agent's life. Restarting is
    fiddly by hand: Ctrl-C during a running turn only interrupts the turn, so
    the agent must be brought to its prompt first, and the pane must be back at
    a shell before the relaunch is typed or the command lands in the TUI
    composer instead. This does both, verifying each step.
    """
    lane = args.target
    session = resolve(lane)
    reg = load_registry()
    worktree = reg.get(lane, {}).get("worktree")
    if not worktree:
        sys.exit(f"tmuxctl: {lane} has no worktree assigned; use `assign` first")

    # Bring it to a prompt: Escape cancels a running turn, Ctrl-C then exits.
    tmux(["send-keys", "-t", session, "Escape"])
    time.sleep(2)
    for _ in range(4):
        tmux(["send-keys", "-t", session, "C-u"])
        tmux(["send-keys", "-t", session, "C-c"])
        tmux(["send-keys", "-t", session, "C-c"])
        time.sleep(3)
        shell = tmux(
            ["display-message", "-p", "-t", session, "#{pane_current_command}"]
        ).strip()
        if shell not in ("node", "codex"):
            break
    else:
        print(f"{lane}: agent would not exit; left running")
        return 1

    # A session launched directly with Codex has no parent shell: exiting the
    # agent removes the session itself. Recreate it before typing the relaunch.
    exists = subprocess.run(
        ["tmux", "has-session", "-t", session], capture_output=True
    ).returncode == 0
    if not exists:
        tmux(["new-session", "-d", "-s", session, "-c", worktree], check=True)
        print(f"{lane}: recreated exited session", flush=True)

    tmux(["send-keys", "-t", session, "-l", f"cd {worktree} && {AGENT_LAUNCH}"])
    tmux(["send-keys", "-t", session, "Enter"])
    deadline = time.time() + 45
    while time.time() < deadline:
        time.sleep(3)
        raw = capture_live(session)
        if IDLE_MARKER in raw:
            cwd = pane_cwd(session)
            ok = os.path.realpath(cwd) == os.path.realpath(worktree)
            print(
                f"{lane}: restarted ctx={context_pct(raw)} cwd={'ok' if ok else cwd}"
            )
            return 0 if ok else 1
    print(f"{lane}: relaunched but did not reach a prompt in time")
    return 1


def cmd_report(args) -> int:
    path = AGENTS_DIR / args.target / "report.md"
    if not path.exists():
        print(f"(no report at {path.relative_to(REPO)})")
        return 1
    print(path.read_text())
    return 0


# ---------------------------------------------------------------- entry point


def main() -> int:
    p = argparse.ArgumentParser(prog="tmuxctl.py", description=__doc__.split("\n")[0])
    p.add_argument("--prefix", default=DEFAULT_SESSION_PREFIX, help=argparse.SUPPRESS)
    sub = p.add_subparsers(dest="cmd", required=True)

    a = sub.add_parser("adopt", help="assign discovered agents to lane names")
    a.add_argument("--lanes", required=True, help="comma-separated lane names")
    a.set_defaults(func=cmd_adopt)

    a = sub.add_parser("list", help="list agents and their tmux state")
    a.set_defaults(func=cmd_list)

    a = sub.add_parser("status", help="compact per-lane status from status.json")
    a.add_argument("targets", nargs="*")
    a.add_argument("--json", action="store_true")
    a.set_defaults(func=cmd_status)

    a = sub.add_parser("send", help="send a one-line prompt and confirm submission")
    a.add_argument("target")
    a.add_argument("message", nargs="+")
    a.set_defaults(func=cmd_send)

    a = sub.add_parser("brief", help="deliver a long brief as a file plus a pointer")
    a.add_argument("target")
    a.add_argument("file")
    a.set_defaults(func=cmd_brief)

    a = sub.add_parser("broadcast", help="send one prompt to several lanes")
    a.add_argument("message", nargs="+")
    a.add_argument("--lanes", default="")
    a.set_defaults(func=cmd_broadcast)

    a = sub.add_parser("wait", help="block until lanes are idle")
    a.add_argument("targets", nargs="*")
    a.add_argument("--timeout", type=int, default=900)
    a.add_argument("--interval", type=float, default=5.0)
    a.set_defaults(func=cmd_wait)

    a = sub.add_parser("reply", help="cleaned transcript events (default: last 12)")
    a.add_argument("target")
    a.add_argument("-n", type=int, default=12)
    a.add_argument("--no-tools", action="store_true")
    a.add_argument("--scrollback", type=int, default=200)
    a.set_defaults(func=cmd_reply)

    a = sub.add_parser("tail", help="cleaned raw lines (default: last 20)")
    a.add_argument("target")
    a.add_argument("-n", type=int, default=20)
    a.add_argument("--scrollback", type=int, default=200)
    a.set_defaults(func=cmd_tail)

    a = sub.add_parser("raw", help="unfiltered pane capture (escape hatch)")
    a.add_argument("target")
    a.add_argument("--scrollback", type=int, default=200)
    a.set_defaults(func=cmd_raw)

    a = sub.add_parser("interrupt", help="send Escape to cancel a running turn")
    a.add_argument("targets", nargs="*")
    a.set_defaults(func=cmd_interrupt)

    a = sub.add_parser("report", help="print a lane's report.md")
    a.add_argument("target")
    a.set_defaults(func=cmd_report)

    a = sub.add_parser("digest", help="fleet dashboard: all lanes on one screen")
    a.set_defaults(func=cmd_digest)

    a = sub.add_parser("doctor", help="safety preflight: isolation, ownership, stalls, context")
    a.set_defaults(func=cmd_doctor)

    a = sub.add_parser("log", help="tail a lane's log.md")
    a.add_argument("target")
    a.add_argument("-n", type=int, default=20)
    a.set_defaults(func=cmd_log)

    a = sub.add_parser("questions", help="open cross-lane requests and decisions")
    a.add_argument("targets", nargs="*")
    a.set_defaults(func=cmd_questions)

    a = sub.add_parser("handoff", help="ask a low-context agent to checkpoint for replacement")
    a.add_argument("target")
    a.set_defaults(func=cmd_handoff)

    a = sub.add_parser("restart", help="replace a lane's agent with a fresh one")
    a.add_argument("target")
    a.set_defaults(func=cmd_restart)

    a = sub.add_parser("assign", help="bind a lane to its git worktree")
    a.add_argument("target")
    a.add_argument("worktree")
    a.set_defaults(func=cmd_assign)

    args = p.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
