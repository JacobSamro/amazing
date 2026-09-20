# AGENTS.md

## Work location policy

**Never build or test on the local macOS machine.** Edit locally, sync to the
Windows build server, and run every build/test there.

### Local editing tree

- `/Users/jMac/Projects/OpenSource/amazing`

### Build server

- Host: `server@win-001`
- SSH key: `~/.ssh/win001-build-key` (filename is missing an "f" — correct as-is)
- Working tree: `C:\Users\server\src\amazing`
- Default SSH shell is cmd.exe — avoid quote-wrapped paths; prefer PowerShell
  `-EncodedCommand` for anything non-trivial.

### Sync workflow

1. Edit locally under `/Users/jMac/Projects/OpenSource/amazing`.
2. Sync changed files up:
   ```
   scp -q -r -i ~/.ssh/win001-build-key <paths> 'server@win-001:C:/Users/server/src/amazing/'
   ```
   (`rsync` is NOT available on the Windows side — use `scp`.)
3. Commit on win-001 (git lives on the server):
   ```
   git -C C:\Users\server\src\amazing add -A
   git -C C:\Users\server\src\amazing commit -m "<message>"
   ```
   (server-side git identity is configured via `-c user.name/user.email` on
   first commit; reuse it)
4. Build/test on win-001:
   ```
   ssh -i ~/.ssh/win001-build-key -o BatchMode=yes server@win-001 "cd /d C:\Users\server\src\amazing && cargo test --workspace --locked"
   ```

### Build server environment (already provisioned)

- cargo/rustc 1.97.1, git 2.55, VS 2022 Build Tools
- VS 2022 CMake at `Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe`;
  its absolute path is stored in the user-level `CMAKE` variable for bundled
  libopus builds.
- vcpkg `2026.07.29` at `C:\Users\server\src\vcpkg` (commit
  `9e593bb18ea69cc5095e012465dcd675a822ed0d`) with
  `libvpx:x64-windows-static-md` 1.16.0#2. User-level `VPX_LIB_DIR`,
  `VPX_INCLUDE_DIR`, `VPX_VERSION`, `VPX_STATIC`, `PKG_CONFIG`, and
  `PKG_CONFIG_PATH` point at that installation.
- Amazing itself is std-only; no extra deps required to build.
- Env vars set at user level (left over from other projects): `LIBCLANG_PATH`,
  `PYTHON3`.

### Screenshot-based UI proof

For GPUI/browser-shell visual checks, capture the actual Amazing window on
`win-001`, copy the PNG to the Mac, and inspect that file. Do not infer UI
state from build success or a full-desktop screenshot.

1. Keep the RDP user session active and identify the `amazing-shell.exe`
   process, executable path, PID, session id, and build timestamp.
2. Run `scripts\capture-gpui-shell.ps1` in that interactive RDP session. An
   ordinary SSH process cannot see the RDP window station, so invoke the script
   through a temporary interactive-only scheduled task running as `server`.
   Always unregister the temporary task in `finally`.
3. Write captures below `target\diagnostics\` (ignored), then copy them back:
   ```
   scp -q -i ~/.ssh/win001-build-key \
     server@win-001:C:/Users/server/src/amazing/target/diagnostics/<capture>.png \
     /private/tmp/<capture>.png
   ```
4. Inspect the copied PNG with the image-viewing tool. `PrintWindow` is the
   preferred capture path because it records the target window even when
   Terminal or another app obscures it.

Treat a screenshot as visual regression evidence only when its executable,
page/fixture, viewport, and capture path are recorded. It complements but does
not replace pinned WPT measurements. Remove temporary scheduled tasks and
throwaway capture helpers after the proof is collected.

## End goal

Amazing is a complete, independent browser: a Blink-class engine and a
Chromium-class product, written from scratch in Rust. Not a wrapper, not a fork,
not an embed. Phases 0–9 are done; what remains is the bulk of the web platform.

The route there, in dependency order. Each stage keeps the existing evidence
discipline — a pinned WPT or Test262 selection, measured counts, a runnable
artifact, and a stop at the boundary for review.

| Stage | Goal | Gate |
|-------|------|------|
| 0 | Repo made safe for parallel work: server reconciled, per-lane worktrees, testharness.js runner, oversized `main.rs` split | fleet can build independently |
| 1 | Run real pages: JS engine v2 (bytecode + GC + regex + modules), DOM/WebIDL bindings and the spec event loop, layout completeness (floats, tables, inline-block), replaced content, Fetch + security policy | Test262 %, WPT `dom/`, `fetch/`, geometry corpora |
| 2 | Performance architecture: incremental style/layout/paint invalidation, property trees, GPU compositor, animations, workers, storage | frame-time traces, Speedometer-style DOM benchmarks |
| 3 | Chromium-style process model: IPC layer, browser/renderer/network/GPU processes, sandboxing, site isolation, session history, bfcache | fuzzing + security review per release |
| 4 | The product: omnibox, tabs, find-in-page, editing/IME, permissions and trust UI, accessibility tree, internal pages, DevTools (CDP), extensions, macOS/Linux ports | screenshot proofs, AT testing, `wpt run` unmodified |
| 5 | Long tail: WebGL/WebGPU, WebAssembly, WebAudio, WebCodecs, MSE/EME, PWAs, MathML, PDF | per-feature WPT |

Stage 1's JS engine and DOM bindings are the long pole; everything downstream
waits on them. Plan in years and let the measured gates say where the project
actually is.

Five locked decisions must be revisited as project-level events before Stage 2:
single-threaded execution, the tree-walking interpreter, software-only raster,
the no-sandbox/no-multi-process non-goal, and DevTools/extensions being out of
scope. Record each in `docs/adr/` rather than editing the locked list silently.

## Multi-agent orchestration

Parallel work is split into **lanes**, one per crate boundary, each driven by a
Codex agent in its own tmux session. `scripts/tmuxctl.py` is the only supported
way to drive and monitor them.

### Lanes

| Lane | Owns | Gate |
|------|------|------|
| `js-lang` | `amazing-js`, `amazing-regex` | Test262 % per directory |
| `dom-bindings` | `amazing-dom`, `amazing-bindings` | WPT `dom/` testharness |
| `layout` | `amazing-layout` | geometry + flex/grid diagnostics |
| `css` | `amazing-css` | cascade corpus |
| `net` | `amazing-net`, `amazing-net-tls`, `amazing-fetch` | WPT `fetch/`, `cookies/` |
| `replaced-content` | `amazing-paint`, `amazing-media`, `amazing-image` | paint diagnostic |

`amazing-webrtc*` is frozen until `dom-bindings` and `replaced-content` deliver
HTMLMediaElement. `amazing-html` and a fuzzing lane are added once merge cadence
holds. The integrator owns everything no lane lists.

### How a lane works

One ticket, one PR, then stop — the same rule as the per-phase gates.

1. The orchestrator writes a brief and delivers it with `tmuxctl.py brief`.
2. The agent works **only** inside its worktree and its owned paths.
3. It iterates with `cargo test -p <its crate>` on win-001, never a full
   workspace build (the box sustains ~3 concurrent builds across six lanes).
4. It records progress in `status.json` and `log.md` as it goes.
5. Before opening a PR: rebase on `origin/dev`, full workspace green on
   win-001, its gate run twice with stable hashes, measured before/after numbers
   in the PR body, and its decision map under `docs/lanes/` updated.
6. It writes `report.md` and stops. It does not start the next ticket.

### Escalation

A lane that needs something outside its paths does **not** edit another lane's
files. It defines the trait it needs in the lower crate and appends a `-`
bulleted line to `questions.md`. The orchestrator collects these with
`tmuxctl.py questions` and either decides, routes the request to the owning
lane's decision map, or escalates to the user as a numbered list of options.

Anything irreversible or outward-facing — rewriting build-server history,
pushing a new remote branch, force-pushing, deleting work — is escalated to the
user as options, never done on an agent's own initiative.

### Why not raw tmux

`tmux capture-pane` on a Codex TUI returns the startup banner, hook warnings,
the usage-limit notice, an animated braille spinner, and a redraw of the prompt
just sent — hundreds of wasted tokens per look, with replies wrapped mid-token
across rows. Worse, `send-keys … Enter` is unreliable straight after a literal
send: the TUI may still be consuming the paste, so the Enter lands in the
composer instead of submitting. Re-pressing Enter blindly then submits stray
partial messages.

`tmuxctl.py` solves both: it filters panes down to transcript events, and it
presses Enter once then *polls for acceptance* before pressing again.

### Reporting is files, not screens

Agents report by writing files; the orchestrator reads those. Screen scraping is
for liveness only, never for results. Each lane owns `.agents/<lane>/`:

| File | Written by | Purpose |
|------|-----------|---------|
| `brief.md` | orchestrator | the ticket, its exit criteria, and lane rules |
| `status.json` | agent | heartbeat — `lane`, `state`, `ticket`, `step`, `updated` (ISO-8601 UTC), `blockers[]` |
| `log.md` | agent | append-only activity log, newest last |
| `questions.md` | agent | `-` bulleted cross-lane requests and decisions needed |
| `report.md` | agent | final ticket report, or a checkpoint before restart |

`.agents/` is gitignored — it is orchestration scratch, not project history.
Anything durable belongs in the lane's decision map under `docs/lanes/`.

### Commands

```sh
scripts/tmuxctl.py adopt --lanes js-lang,layout,css   # bind sessions to lanes
scripts/tmuxctl.py assign layout ../amazing-wt/layout # bind a lane to its worktree
scripts/tmuxctl.py digest            # fleet dashboard — the main monitoring view
scripts/tmuxctl.py doctor            # isolation / ownership / stall / context checks
scripts/tmuxctl.py brief layout f.md # deliver a long brief as a file + pointer
scripts/tmuxctl.py send layout "..." # one-line prompt, submission confirmed
scripts/tmuxctl.py wait layout --timeout 900
scripts/tmuxctl.py log layout        # tail the lane's log.md
scripts/tmuxctl.py questions         # everything the fleet is waiting on
scripts/tmuxctl.py reply layout      # cleaned transcript (liveness/debug only)
scripts/tmuxctl.py handoff layout    # checkpoint a low-context agent
scripts/tmuxctl.py raw layout        # unfiltered pane — escape hatch
```

Deliver multi-line instructions with `brief`, never `send`: a newline inside a
TUI composer submits the prompt early and corrupts it.

`wait` blocks by design and will outlive a 120s foreground tool timeout — run it
as a background command, or just poll `digest`, which is cheap. Note also that a
narrow tmux pane truncates the TUI status bar, so `CTX` reads `?` for that lane;
widen the pane if the context budget matters for it.

### Orchestration rules

- **One lane, one worktree.** Agents never share a checkout. `doctor` reports a
  lane with no worktree as an ISOLATION finding, and ownership checking only
  runs once a lane is isolated.
- **Lane ownership is enforced.** `LANE_PATHS` in `tmuxctl.py` lists the paths
  each lane may write. `ARCHITECTURE.md`, `README.md`, `todo.md`, and the root
  `Cargo.toml` belong to the integrator alone. A lane needing a change
  elsewhere defines a trait in the lower crate and files a request in
  `questions.md` — it does not edit another lane's files.
- **Context exhaustion is routine, not failure.** Below 25% remaining,
  `doctor` flags the lane; `handoff` makes the agent write `report.md` so a
  fresh agent resumes without re-deriving the work.
- **Stalls are detected, not waited on.** A lane whose tmux pane is busy while
  `status.json` has gone 20+ minutes stale is reported by `doctor`.
- **win-001 is a single 16-core box.** It sustains ~3 concurrent builds. Agents
  iterate with `cargo test -p <their crate>` and run `--workspace` only before
  opening a PR.
- **One ticket, one PR, then stop.** This mirrors the existing per-phase rule:
  report measured results and wait, never auto-proceed.

## Project notes

- Amazing: from-scratch std-only browser engine. Zero external crates in engine
  cores (`amazing-html/css/dom/layout/js/net/paint`). Read ARCHITECTURE.md for
  locked decisions before writing code.
- Phased plan with hard exit criteria — never auto-proceed to the next phase;
  report per-phase results and wait for confirmation.
- Conformance suites: WPT tracked from Phase 3 onward, Test262 from Phase 6;
  always report measured numbers, never estimates.


Keep files under 4000 Lines per file. For code health

Generated lockfile exception: `crates/amazing-shell-gpui/Cargo.lock` may exceed
4,000 lines because the isolated GPUI platform graph is machine-generated and
must remain in one Cargo-readable lockfile for `--locked` reproducibility. This
exception does not apply to authored source, tests, scripts, or documentation.
