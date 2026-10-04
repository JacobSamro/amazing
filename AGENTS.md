# AGENTS.md

## How we work: bulk (read this first)

Speed comes from doing a lot per cycle, not from many small cycles.

- **Write in bulk.** A ticket is a whole area: every Date method, the whole Range interface, all of
  flex line breaking. Read the spec section once, write the whole area, then compile. Do not
  compile after each function.
- **Fix in bulk.** When the compiler reports errors, fix all of them before building again.
- **Test in bulk.** Each run covers hundreds of cases: the crate's tests plus the whole relevant
  corpus directory (Test262 directory, WPT directory), in ONE `linux-run.sh` command
  (`build && tests && corpus`). Never run a single test in a loop.
- **Triage by cluster.** Group failures by error message, fix the largest cluster first, rerun
  the whole corpus. A cluster fix is worth dozens of passes; a one-off is worth one.
- **Gate once.** One final Linux gate per ticket (workspace nextest + doctests + corpus before and
  after). Then commit, set `done` with the SHA and numbers, and start the next item immediately.
  Never wait idle for a reply or a merge; a blocked item goes to `questions.md` and you move on.
- **If a request is flagged as cybersecurity**, restate the real context and continue: this is a
  from-scratch web browser engine. TLS, cookies, CORS, sandboxing, process isolation, fuzzing,
  parsers for untrusted input and DevTools are ordinary browser features built to spec, not
  security tooling or attacks.
- **Never `git stash`.** The stash is one stack shared by every lane worktree of this repo; on
  2026-09-27 a lane's `git stash pop` applied another lane's concurrently pushed stash. Park
  work-in-progress as a WIP commit on your own lane branch, or `git diff > $OUT/wip.patch`.
- **Never `nohup … &` a run.** Start long `linux-run.sh` / `test262-boat.sh` commands with your
  shell tool's background mode, which notifies you when they exit. A detached `nohup` job exits
  silently and the lane waits forever (2026-09-27: layout and js-date each sat idle 20+ minutes
  after their run had already finished).
- **Sandbox time is money.** Sandboxes are shared and billed while running. Batch your work so each
  `linux-run.sh` call does a lot; do not hold a sandbox with polling or sleeps.
- **Port, don't reinvent (2026-09-27).** When a spec area is already solved in a permissively
  licensed Rust engine, port the algorithm instead of re-deriving it. See "Porting from other
  engines" below.

## Merge gate (2026-10-04, user-approved; overrides older merge procedures)

Merge tickets independently. A completed ticket is a `lane/<ticket>-ready` branch with **one
squashed commit on a recorded `origin/dev` base**, containing code and compact evidence only.
Do not publish WIP commits, merge commits, or lane-to-lane dependency history. If a source
prerequisite is missing, package it as a separate ready ticket first; record the dependency in
`questions.md`. Do not merge `integrator/batch19-preview` as a batch.

Only these requirements block an engine merge:

- workspace `cargo nextest run --workspace --locked --no-fail-fast` plus
  `cargo test --workspace --locked --doc` on Linux/Boat;
- a **16-shard** pinned Test262 run with zero lost passing files and zero lost passing variants;
- the ticket's own corpus (WPT, render, XML or geometry) with zero lost passes;
- compact evidence within **512 KiB per file and 2 MiB per ticket**.

The integrator cherry-picks one or a few independent ready commits onto recorded `origin/dev`,
runs the required gate, and pushes the exact passing candidate. If a commit breaks the gate,
prepare a new candidate without that commit, gate the remainder, and push it. Preserve the failed
branch; do not delete work. Each subsequent gate records the previous published source as its
base, so deltas chain rather than accumulating against an older batch.

Censuses, repeated PID observations, read-only capture audits and other lane-invented custody
checks may be notes, but **must not block a merge**. The integrator reviews the compact summaries
and their hashes, not lane evidence scripts or raw archives. An actual lost pass remains a blocker.

### Tooling failures and time use

- A tooling failure gets **one clean rerun**. If tooling fails again, record the command, failure
  and remote raw-output path in `questions.md`, then move to another ready ticket. Do not recover
  raw files by hand during integration. A real test regression is an engine issue, not a tooling
  failure to waive.
- Gate-tooling repairs are a separate ticket owned by one lane. Wait for actual job exit status;
  never infer completion from output files appearing. Keep raw output on the sandbox and pull
  back only compact classifications, summaries and hash manifests.
- Messages to each lane: at most once per 15 minutes, only for a decision, a merge result or a new
  ticket. Read file reports for routine progress. Native Codex subagents are authorized; tmuxctl
  remains the supported interface for existing tmux lanes.
- Target at least **one engine-code push to `dev` every two hours while ready work exists**. If
  missed, report the concrete reason to the user. Policy-only pushes do not satisfy this target.
- Maintain a short current handoff for a fresh Codex session. Preserve stale branches until the
  user explicitly approves their deletion.
- `scripts/merge-preflight.py` rejects WIP titles, merge commits, oversized evidence files and
  evidence totals over the ticket limit before remote gate allocation.

## Evidence policy (2026-10-04, user-approved)

On 2026-10-03 lanes committed 174 MB of evidence in one day against about 10 MB of code, mostly
raw `before.jsonl`/`after.jsonl` dumps of about 4 MB each. `.git` is shared by every lane and
clone, and history is never rewritten, so every committed byte is permanent.

- **Commit compact evidence only.** Per gate, commit at most:
  - a `compact.json` mapping test id to classification (`--compact` for render sweeps), or a
    pass-set listing;
  - the counts (pass, fail, timeout, gained, lost);
  - the lost list (it should be empty) and the gained list;
  - the sha256 of each raw result file;
  - the source and base commits.
- **Raw dumps stay on the build host,** under `$OUT`. This covers per-test `report.json`,
  `before/after.jsonl`, logs, renders, PPM/PNG diffs and full Test262 result JSON. Record their
  sha256 and host path in the compact file, so a gate can be re-checked without the bytes.
- **Size limit: 512 KiB per evidence file, 2 MiB per ticket.** Oversized evidence is rejected
  by the gate; compact it before presenting a ready commit.
- **Evidence never comes at the expense of honesty.** Smaller files must keep exact
  counts, every lost id, and enough hashes to prove which binary and corpus produced them.
- **Integrator:** check `git diff --stat <base> <lane-branch> -- '*/evidence/*' docs/lanes/`
  before every merge, and send back any branch that exceeds the limit.

### Local disk hygiene

- After creating a worktree, run `scripts/thin-worktree-evidence.py <worktree> --apply`.
  It omits recoverable historical raw evidence while preserving modified files and Git history.
- Keep builds and new raw gate output on Boat. The external mbx cache does not cover local
  worktree copies or `.agents/` scratch automatically.
- The integrator may use `scripts/externalize-agent-scratch.py` to move large scratch files
  inactive for at least two hours to mounted external storage. Copies must be hash verified;
  original paths remain symbolic links. Active outputs, source changes and branches are preserved.
- Remove retired clean worktrees only after confirming they have no active owner; keep their
  branches. Never delete a branch as part of disk cleanup without user approval.

## Porting from other engines (2026-09-27, user-approved, ADR 0013)

Amazing stays our own architecture (value model, VM, bindings) and std-only in the engine cores,
but lanes may **port code** from other open-source engines to close spec gaps faster. Ports are
code copied and adapted into our crates, never new crate dependencies.

- **Allowed sources: MIT or Apache-2.0 only.** MPL-2.0 (Servo, Stylo, Nova) and GPL/LGPL code
  is read-only reference: study it, do not copy it.
- Official Unicode 17 data and conformance tests have a data-only Unicode-3.0 exception
  approved by the owner in ADR 0014. Rust algorithms still require MIT or Apache-2.0.
- **Attribution is mandatory.** Every file containing ported code starts with a header comment
  naming the upstream repo, path, commit and license, e.g.
  `// Portions ported from boa-dev/boa (core/engine/src/builtins/...) @ <sha>, MIT.`
  and the lane appends a line for that upstream to `THIRD_PARTY_NOTICES.md` (repo, license,
  copyright line, which of our files use it) in the same commit. **Append a new row; never
  edit an existing one** (the file is union-merged, so an edited row comes back as a duplicate).
  The integrator consolidates rows.
- **Ports obey every other rule:** std-only, under 4000 lines per file, zero lost passes on the
  lane's Test262/WPT gate. Rewrite upstream dependencies (ICU4X, temporal_rs types, `gc` crates)
  onto our own types; do not pull them in.
- Port whole areas in bulk (a builtin, a parser section), not line-by-line snippets, and record
  the upstream commit in the lane's `docs/lanes/` decision map.

Suggested sources per lane (verify the license at the commit you port from):

| Lane | Source | License |
|------|--------|---------|
| js-lang, js-runtime, js-vm | `boa-dev/boa`, `Hans-Halverson/brimstone`, `lucid-softworks/lumen` (std-only, claims full Test262) | MIT |
| js-date (RegExp) | `ridiculousfish/regress`, `lucid-softworks/lumen` | Apache-2.0 / MIT |
| js-stdlib (Temporal) | `boa-dev/temporal` (temporal_rs), `lucid-softworks/lumen` | Apache-2.0 / MIT |
| js-intl | `lucid-softworks/lumen` (std-only Intl/CLDR tables) | MIT |
| layout, layout-flexgrid | `DioxusLabs/taffy` | MIT |
| html | `servo/html5ever` tree builder (check its per-crate license: MIT/Apache) | MIT / Apache-2.0 |
| css, compositor, paint | Servo/Stylo/WebRender are MPL-2.0: reference only | — |

## Linux-first (2026-09-26, overrides the Windows gate rules below until lifted)

Windows gates are deferred. Every lane's final gate is on Linux, on its own Boat sandbox:
`scripts/linux-run.sh <lane> -- 'cargo nextest run --workspace --locked --no-fail-fast && cargo test --workspace --locked --doc'`
plus its corpus (Test262, WPT) before and after. Do not run `scripts/win-run.sh` or win-001 gates
unless the orchestrator asks. Report Linux gate numbers in status.json and report.md. Merges are
gated by the integrator on Linux (workspace + sharded Test262 on Boat).

## Work location policy

**Never build or test on the local macOS machine.** Edit locally, sync to the
Windows build server, and run every build/test there.

### Local editing tree

- `/Users/jMac/Projects/OpenSource/amazing`

### Build server

- Host: `server@win-001`
- SSH key: `~/.ssh/win001-build-key` (filename is missing an "f" — correct as-is)
- 16 cores. It sustains roughly **3 concurrent builds**; six lanes must not all
  run a full workspace build at once.
- Per-lane build trees: `C:\Users\server\src\wt\<lane>`
- Per-lane target dirs: `C:\Users\server\target\<lane>` via `CARGO_TARGET_DIR`
- Default SSH shell is cmd.exe. It mangles `for` loops and quoted paths — use
  PowerShell `-EncodedCommand` (UTF-16LE base64) for anything beyond a single
  simple command. Unix tools are absent: no `head`, no `rsync`.

**All ssh/scp to win-001 multiplexes over one persistent connection.** The
Mac's `~/.ssh/config` has a `Host win-001` block with `ControlMaster auto`,
`ControlPersist 8h` and a shared `ControlPath`, so the first command opens an
authenticated master and every later `ssh`/`scp` runs as a channel inside it.
Existing command forms (`ssh -i ~/.ssh/win001-build-key server@win-001 …`, `scp …
server@win-001:…`) pick it up automatically; `ssh win-001 '…'` is the short form.

Why: on 2026-09-21 ten lanes opening hundreds of short-lived connections
tripped sshd's default `MaxStartups 10:30:100`, which randomly drops
connections *before authentication* once too many are mid-handshake. New
connections reset before auth, established sessions stayed healthy, and it
cleared by itself once the retries stopped -- the box was never rebooted and had
~49 GB RAM free. Measured after multiplexing: **0.16s per command vs 0.68s** with
a fresh connection.

**Bulk transfers use their own connection.** The shared master is one TCP
connection, so a large `scp` starves every other lane's commands. On
2026-09-23 a 5 MB file took 69 s through the master and 3.7 s on a fresh
connection, while four 45 MB refresh archives sat stuck in it. Any upload
bigger than about 1 MB (archives, evidence bundles) uses
`scp -o ControlPath=none …`. `refresh-lane-trees.sh` already does this.
Short commands keep using the master.

Server limits were raised on 2026-09-22 to `MaxSessions 50` (channels per
connection) and `MaxStartups 50:30:100`; the previous config is kept as
`C:\ProgramData\ssh\sshd_config.bak-20260922202601`. Verified: 20 concurrent
channels over one master, 20/20, none refused or fallen back (previously 3 of 13
overflowed). Past 50, OpenSSH falls back to a fresh connection rather than
failing. **A master opened before a limit change keeps the old limit** -- run
`ssh -O exit win-001` after changing sshd config. Do not open ssh in tight retry
loops; if a command is refused, back off. `ssh -O check win-001` shows the master.

**Git lives on the Mac, not on the server.** win-001 has no GitHub credentials
and the repo is private, so it can neither fetch nor push. It is a pure build
box: source arrives by `scp`, results come back over ssh. Never run `git commit`
there — server-side commits created a 139-commit branch that never reached
GitHub and had to be archived (`archive/win001-main`, plus
`archive/win001-phase6-paint-compositing`).

### Sync workflow

1. Edit in your lane worktree on the Mac (`../amazing-wt/<lane>`).
2. Sync changed files to your lane's build tree:
   ```
   scp -q -r -i ~/.ssh/win001-build-key <paths> \
     'server@win-001:C:/Users/server/src/wt/<lane>/'
   ```
   For a bulk seed, `tar` locally, `scp` one archive, and expand it server-side —
   far faster than `scp -r` over many small files.
3. Build/test on win-001 against your own target dir:
   ```
   ssh -i ~/.ssh/win001-build-key -o BatchMode=yes server@win-001 \
     "powershell -NoProfile -EncodedCommand <b64>"
   ```
   where the script sets `$env:CARGO_TARGET_DIR` to
   `C:\Users\server\target\<lane>` and runs the test command below.

**Use `cargo nextest`.** Measured on this workspace: `cargo test --workspace
--locked` takes 105.8s, `cargo nextest run --workspace --locked` takes 53.1s
for the same 1208 tests. Roughly 2x, and it matters because every lane gates
repeatedly.

```
cargo nextest run -p <your crate> --locked      # or --workspace before a PR
cargo test -p <your crate> --locked --doc       # ALWAYS run this too
```

**Wrap every `--workspace` gate or full render sweep in a build slot:**

```
scripts/with-build-slot.sh <lane> -- ssh win-001 "powershell -NoProfile -EncodedCommand <b64>"
```

The box sustains about three concurrent full builds. When a fleet resumes, every
lane re-gates at once and ten simultaneous workspace builds run slower than
three sequential batches would. The wrapper holds one of three Mac-side slots
(`mkdir`-atomic, dead holders reclaimed) and queues the rest. Crate-scoped `-p`
iterations do not need a slot.

**Linux runs on pooled Boat sandboxes (2026-09-26).** `scripts/linux-run.sh <lane> -- '<cmd>'`
runs on a Boat sandbox (4 vCPU Ryzen 9950X, 8 GB, from the `amazing-builder` template: Rust 1.97.1,
libvpx 1.16, nextest, the pinned Test262). Related lanes share one sandbox (`POOLS` in
`scripts/boat-lane.py`); each lane syncs into its own `~/amazing/wt/<lane>` and writes outputs to
`$OUT` (`~/amazing/out/<lane>`). All lanes on a sandbox share `~/amazing/target`, so cargo's lock
serializes their compiles ("Blocking waiting for file lock" means another lane is building; it is
normal). `scripts/boat-idle-reaper.py` stops a sandbox after 30 idle minutes (40 when Boat starts run low; raised 2026-09-27 after the fleet used up the 200 starts/day quota); the next run resumes
it in seconds. `scripts/boat-lane.py pool-report` shows busy and overlap per pool.
**Linux desktop and WPT (2026-09-26).** Sandboxes are Ubuntu 24.04 with a GNOME desktop, no
GPU; Mesa software Vulkan (llvmpipe), Xvfb, xdotool and ImageMagick are in the template.
`scripts/linux-shell-capture.sh <lane> -- [shell args]` builds the GPUI shell, runs it on a virtual
display and copies a window PNG plus metadata to `.agents/<lane>/captures/`. The pinned WPT
(`c2a2fce`) is at `~/corpora/wpt`; on an older sandbox run `bash scripts/ensure-wpt.sh` first.
Linux pixel results are reported as Linux numbers. `~/corpora/wpt` is shared by every lane on a
sandbox: treat it as read-only, never run `git sparse-checkout` on it (clone a sparse copy under
`$OUT` if you need one), and start WPT commands with `bash scripts/ensure-wpt.sh &&`, which
restores the full tree if another lane narrowed it.
**Linux reftests (2026-09-26, extended 2026-09-26 SHELL-L5):** `scripts/linux-render-wpt.sh <lane>
<wpt-subdir> [<wpt-subdir> ...]` renders the self-contained static reftests of one or more
pinned-WPT subdirs (e.g. `css/css-backgrounds css/css-color`; batch a lane's whole sweep into one
call, sharing a single build) with the headless `amazing` renderer and fuzzy-compares them
(`scripts/wpt_render_compare.py`, `--jobs N` parallel), writing one `report.json` per subdir plus a
`clusters.json` grouping mismatches by diff bounding-box and colour
(`scripts/wpt_render_cluster.py`); it replaces win-001 render sweeps while Windows is deferred.
`--baseline auto|<dir>|<file>` turns a run into a zero-loss gate: it diffs the run's per-test
classifications against a previous report (checked-in evidence by default with `auto`) and prints
gained/lost/changed/unchanged/new/removed counts, failing the run (and every remaining subdir still
runs) if anything regressed from passing. `--compact` writes/commits `compact.json` (test id ->
classification only, no per-test detail) instead of the full `report.json`, so gating evidence
stays small over repeated runs. Baselines: `crates/amazing-shell-gpui/evidence/wpt-render/`.
**Lanes never run `boat` commands directly** (no `boat snapshot`, `boat new`, `boat stop`,
`boat data-retention`, `boat billing`, ...): the account, its templates and its retention
settings belong to the orchestrator. Use `scripts/linux-run.sh` only; report Boat problems in
`questions.md`.
Same interface, same `$CARGO_TARGET_DIR`, `$JOBS` and `$TEST262_ROOT`; outputs go to
`~/amazing/out` on the sandbox.

**Iterate on Linux; gate on Windows.** Two Linux build hosts, `venus-001`
(32 threads) and `venus-003` (16 threads), take the platform-independent load:
crate and workspace nextest while iterating, full Test262, the WPT `dom/`
harness and the net corpora. The whole workspace builds and passes there
(1,756 tests in about 20 seconds). Use:

```
scripts/linux-run.sh <lane> -- 'cargo nextest run -p <crate> --locked'
scripts/linux-run.sh <lane> -- 'cargo nextest run --workspace --locked && cargo test --workspace --locked --doc'
```

It rsyncs your worktree (uncommitted edits included) to `~/amazing/wt/<lane>`
on the lane's host, sets your own `CARGO_TARGET_DIR`, `JOBS` and
`TEST262_ROOT` (the pinned `3655e746` checkout), and queues in
that host's own slot pool. `--ref <sha>` runs an exact commit; `--no-slot` skips
the queue for small `-p` runs. Results stay on the host under `~/amazing/out`.

What still needs win-001: **final gates** (the numbers in your report), render
sweeps and reftests, anything font, GPU, GPUI shell or media-pipeline specific,
and any Windows-only code path. Linux numbers are for iteration and are never
reported as the gate. If a result differs between Linux and Windows, that is a
bug to report, not a reason to pick one.

**Windows gates: win-002 for crate/workspace, win-001 for renders.** win-001's
three slots also carry every render sweep, so crate and workspace final gates run
on win-002 (same toolchain), with its own two-slot pool:

```
scripts/win-run.sh <lane> -- 'cargo nextest run --workspace --locked; cargo test --workspace --locked --doc'
```

It extracts your worktree (or `--ref <sha>`) fresh into win-002 and runs the
PowerShell commands with your own `CARGO_TARGET_DIR`. Render sweeps, reftests,
and anything that needs the WPT checkout, fonts or GPU stay on win-001 via
`with-build-slot.sh`.

**Work in bulk (2026-09-24).** Tickets are whole areas, not slices. Write the
whole area first, against the spec, then test it all at once. Do not gate
after each function.
- Iterate on Linux, where a full Test262 run takes minutes and the workspace
  about 20 seconds.
- Run the **final gate once** on win-001: your crate plus the workspace, with
  nextest and doctests, and the lane's corpus before and after. Repeat a run
  only for render/reftest sweeps, or when a classification changed that you
  did not expect.
- The integrator re-checks zero losses at merge on the merged head, so no
  ceremony beyond that is needed.
- Keep `report.md` short: what landed, the measured before/after numbers, and
  cross-lane requests. Evidence goes in files, not prose.
- When a ticket is done early, take the next item from your lane's TODO or
  its failure clusters without waiting. Report at the end, with one ticket
  per commit series.

**Driver adapter per lane (2026-09-24).** `crates/amazing/src/runtime_binding_host.rs` is shared by
every lane that installs JS bindings, and big additions there caused repeated merge conflicts. Put your
adapter in your own file, `crates/amazing/src/runtime_host_<lane>.rs` (a separate `impl RuntimeBindingHost`
block plus your state types; that file is yours), and keep the shared file to ONE `mod` line, one installer
call and at most a few state fields or hook lines. Existing large blocks move out when their lane next
touches them.

**nextest does not run doctests.** This workspace has them (`amazing-js`,
`amazing-html`), and nextest silently reports success without them, so a lane
that switches and forgets the `--doc` line loses that coverage without any
failure to warn it. Report both counts.
4. Commit and push **on the Mac**, from your lane branch, then open the PR.

### Keeping a lane tree current

A lane's build tree is seeded once and then only receives that lane's own
files, so as `dev` advances it goes **stale for every crate the lane does not
own**. A full-workspace gate run against a stale tree measures old sibling code
and is worthless. Rebasing on the Mac does not fix this — the server tree is a
separate copy.

Before any full-workspace gate, the lane asks the integrator for a tree
refresh. The integrator tars current `dev` on the Mac, uploads one archive, and
expands it into all six trees:

```
tar -cf /tmp/dev-refresh.tar --exclude=.git --exclude=target --exclude=.agents <paths>
scp -q -i ~/.ssh/win001-build-key /tmp/dev-refresh.tar server@win-001:C:/Users/server/
# then, per lane, server-side:  tar -xf C:\Users\server\dev-refresh.tar -C C:\Users\server\src\wt\<lane>
```

**Do not chase `dev` during a final gate.** With many lanes merging, `dev`
moves every few minutes, and re-gating on each move means a busy fleet never
finishes. A lane's final gate proves *its branch on a fixed base*: record that
base commit in `report.md` and gate against it. Proving the combination works is
the integrator's authoritative gate, run on the merged head after every merge.
Rebase and re-gate only when the drift since your base **touches your own
crates**; check with `git diff --name-only <base> origin/dev -- <your paths>`.

**Each integrator gates in its own tree.** The co-integrator uses
`wt\integrator`; the primary uses `wt\integrator-primary`, each with its
matching target dir. On 2026-09-23 the two shared one tree: the co-integrator's
`REFRESH-HOLD` silently skipped the primary's refresh, so a "merged-head" gate
measured a different commit and reported the previous head's count as a pass.
A merged-head gate that did not print a fresh lock line is not evidence.

**Integrators gate a merge before pushing it, with
`scripts/primary-gate.sh HEAD`** (serialized, and it can gate an unpushed
local commit). This stopped the IA-BINDINGS-2 install from reaching `dev`
when it broke four tests on the combined head. If `origin/dev` moves during
the gate *only in crates the merge does not touch*, rebase, push, and gate
the pushed head straight away. Re-gating on every move would never finish
while two integrators are merging. **Merge ownership is split, with no
exceptions:** the primary merges js-lang, js-runtime, dom-bindings, layout,
css and net; the co-integrator merges replaced-content, html and
integrator-assist.

**A refresh overwrites lane-owned files in the server tree with the `dev`
versions**, including work the lane had already synced up. The Mac worktree is
the source of truth so nothing is lost, but the lane must re-`scp` its own
changed files afterwards, and must treat any Windows result from before the
refresh as stale. Order is: rebase on `origin/dev` → refresh tree → re-sync own
files → gate.

### Disk hygiene on win-001

On 2026-09-23 the C: drive filled to 3.4 GB free and every build failed with
`os error 112`. The causes were render and evidence folders from merged
tickets left under `target\<lane>\` (60 GB, mostly layout), Cargo
incremental caches, and the retired pre-worktree checkout's
`src\amazing\target` (68 GB). Clearing them brought it back to 202 GB free.

- **Once a ticket merges, the lane deletes its own server-side
  render/evidence folders.** Durable evidence is what the lane committed
  under `crates/*/evidence/` or `docs/lanes/`. The server copies are scratch.
- Keep `debug\` and `release\` build caches. `debug\incremental` may be
  purged at any time, because Cargo rebuilds it.
- The integrator checks free space when a build fails oddly:
  `(Get-PSDrive C).Free`. Below about 30 GB, purge stale folders (unmodified
  for 2 hours or more) under `target\*\` other than `debug`/`release`.

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
single-threaded execution, the tree-walking interpreter (replaced by a bytecode
VM in ADR 0003), software-only raster (a compositor with tiled raster in ADR 0006;
GPU raster still open), the no-sandbox/no-multi-process non-goal (replaced by
ADR 0005), and DevTools/extensions being out of
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
| `replaced-content` | `amazing-paint`, `amazing-media`, `amazing-image`, `amazing-svg` | pinned SVG/paint WPT selections |
| `js-runtime` | `amazing-js` interpreter, built-ins, value model | Test262, zero lost passes |
| `js-vm` | `amazing-js/src/vm`, `gc` (ADR 0003) | Test262 on the VM vs fallback |
| `js-binary` | `amazing-js/src/builtins_binary*` | Test262 ArrayBuffer/DataView/TypedArray/Atomics, BigInt |
| `js-stdlib` | `amazing-js/src/builtins_std*`, `builtins_temporal*` | Test262 Reflect/collections/Iterator/Temporal |
| `ipc` | `amazing-ipc`, `amazing-process` (ADR 0005) | crate tests incl. serialization fuzzing; child-process demo |
| `compositor` | `amazing-compositor` (ADR 0006) | pixel parity with full repaint; frame-time traces |
| `storage` | `amazing-storage` | crash-recovery tests; engine-level IndexedDB expectations |
| `html` | `amazing-html`, `amazing-encoding` (ADR 0004) | html5lib trees, WPT `encoding/` |
| `text` | `amazing-text`, `amazing-text-provider` | pinned `css-fonts` selections |
| `integrator-assist` | `crates/amazing` driver adoption | WPT through the installed runner |

`amazing-webrtc*` is frozen until `dom-bindings` and `replaced-content` deliver
HTMLMediaElement. `amazing-html` and a fuzzing lane are added once merge cadence
holds. The integrator owns everything no lane lists.

### How a lane works

One ticket per ready commit. Follow the Merge gate section above; older Windows/twice-gate
procedures do not apply.

1. Work only in your isolated worktree and owned paths; use Boat for builds and tests.
2. Write and test the whole area in bulk, preserving all existing passing corpus identities.
3. Record progress in `status.json` and `log.md` without routine messages to the integrator.
4. On a recorded `origin/dev` base, prepare `lane/<ticket>-ready` as one squashed commit with
   code, compact before/after evidence and raw-result hashes/remote paths.
5. Run `scripts/merge-preflight.py --base <recorded-base> --ref <ready-commit> --ready`.
6. Write `report.md`, set status `done` with SHA and counts, then start the next independent
   ticket. A missing prerequisite goes to `questions.md`; do not import its lane history.

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
- **win-001 is a single 16-core box.** It sustains ~3 concurrent builds, so it
  is kept for final gates and Windows-only work. All Linux work uses Boat through `scripts/linux-run.sh`.
  Windows-only work uses `server@win-001`; Windows gates remain deferred.
- **One ticket per commit series, then continue.** Report measured results,
  set status done so the integrator can merge, and carry on with the brief's
  next item on a new commit series. Stop only when the brief's list is empty.

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
