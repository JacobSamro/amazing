# Integrator handbook

How to run the multi-agent fleet on this project. Written so a fresh integrator
can take over without re-deriving any of it — the same checkpoint discipline the
lanes are held to.

Read `AGENTS.md` first for the end goal, the lane model and the build-server
rules. This file is the operational half: what the integrator does, in what
order, and the traps that have already cost time.

## The job

Lanes write code. The integrator does everything else:

- writes briefs and keeps each lane's next ticket queued
- answers cross-lane design questions and issues path grants
- merges, and runs the authoritative gate on the merged head
- owns the shared files no lane may touch: `ARCHITECTURE.md`, `README.md`,
  `todo.md`, the root `Cargo.toml`, the driver, and the GPUI shell
- refreshes lane build trees and keeps lockfiles coherent

**The integrator is the throughput bottleneck, not the build server.** Measured:
win-001 sat at 27% CPU with zero compiles running while four lanes were blocked
waiting on integrator actions. Before adding lanes, remove integrator round
trips.

## Daily loop

```sh
scripts/tmuxctl.py digest      # fleet on one screen; run this constantly
scripts/tmuxctl.py doctor      # isolation, ownership, stalls, context floor
scripts/tmuxctl.py questions   # everything waiting on a decision
```

Then, per lane that needs it:

```sh
scripts/merge-preflight.sh <lane>            # before reading any diff
scripts/refresh-lane-trees.sh <lane>         # or --all after a merge
scripts/tmuxctl.py brief <lane> <brief.md>   # long instructions go as a file
scripts/tmuxctl.py restart <lane>            # below the 25% context floor
```

## Merging a lane

1. `scripts/merge-preflight.sh <lane>` — rebase state, ownership, grant shape,
   whether the lane actually reported.
2. Read `report.md` for **measured** numbers. Reject estimates. A low number
   measured correctly is worth more than a high one that is not.
3. Read any diff the preflight marks `REVIEW` (rewrite grants).
4. Merge with `--no-ff` and a message that records what changed, the measured
   before/after, and what was deliberately **not** claimed.
5. `scripts/refresh-lane-trees.sh --all`, then tell the lanes.
6. Run the authoritative gate yourself on the merged head — a gate on a lane
   branch proves nothing about the merge.

## Traps that have already cost time

- **Stale Cargo fingerprints.** `tar` restores archive mtimes, which can predate
  cached artifacts, so a gate links a stale rlib against byte-correct source.
  Matching source hashes do **not** prove artifact freshness. The refresh script
  extracts with `-m` and drops first-party fingerprints; never bypass it.
- **A refresh overwrites lane-owned files** on the server. Always tell lanes to
  re-sync and treat earlier Windows results as stale.
- **Lane trees go stale for crates the lane does not own.** Rebasing on the Mac
  does not fix it; the server tree is a separate copy.
- **The shell has its own workspace and lockfile.** A new first-party dependency
  edge (e.g. `amazing-js` → `amazing-regex`) breaks `--locked` shell builds until
  that lockfile is regenerated on win-001.
- **Lanes cannot add workspace members or refresh the root lockfile.** Seed the
  crate skeleton, register it, verify it builds, then tell the lane.
- **cmd.exe mangles loops and quoted paths.** Use PowerShell `-EncodedCommand`
  (UTF-16LE base64). There is no `head`, no `rsync`. macOS bash is 3.2 — no
  `mapfile`.
- **Backticks in a `send` message** get shell-interpreted and silently mangle it.

## Grants

A lane may write only its own paths. When a ticket genuinely needs more, record
the grant in `scripts/tmuxctl.py` so `doctor` and the preflight both know, and
remove it when the ticket lands.

Two shapes, not interchangeable:

- **Additive** (`LANE_GRANTS`) — an added match arm, dispatch line or entry
  point. Any deletion means the grant was exceeded; the preflight fails it.
- **Rewrite** (`LANE_GRANTS_REWRITE`) — replacing existing code *is* the ticket.
  The preflight flags it for reading instead.

Never let two lanes hold a grant on the same file at once.

## Escalation

Cross-lane needs go through a trait in the lower crate plus a `questions.md`
line, never a direct edit into another lane's crate. The integrator decides,
routes to the owning lane, or escalates.

**Escalate to the user, as numbered options, never decide alone:** new external
dependencies or pinned data, rewriting build-server history, force-pushes,
anything destructive or outward-facing, and changes to locked `ARCHITECTURE.md`
decisions.

Open at time of writing: a pinned **Public Suffix List** for `RegistrableDomain`,
and **brotli** for HTTP content coding. Both written up in
`docs/lanes/net_TODO.md`.

## Integrator work queue

Jobs lanes have correctly routed here because they touch the driver or shell:

- **Shell stylesheet provenance** — `engine_viewport.rs` has two concatenating
  `compute` call sites. CSS-3 fixed the driver half; mirror it. Blocks CSS-4
  (background shorthand + stylesheet-relative URLs).
- **Shell/driver URL migration** — legacy `Url` still backs cookie and cache
  keys, storage origins, resource resolution and navigation. Until it lands,
  shell origin decisions can differ from the now-canonical transport. Persisted
  keys need a migration plan. Table in `docs/lanes/net_TODO.md`.
- **`intrinsic_size` adapter migration** — the contract is now natural content
  dimensions only; driver and shell adapters must stop returning attribute- or
  CSS-adjusted sizes.
- **Shell `NodeKind` match arms** — DOM-2 added variants; the shell's two
  matches were left to the integrator.
- **Author-specified provenance in CSS** — `ComputedStyle` cannot distinguish an
  explicit `background:transparent` from absent styling, so control chrome
  cannot honour explicit defaults. Queue as a CSS ticket.
- **Split the oversized files** — `crates/amazing/src/main.rs` (~3.5k lines) and
  the GPUI `main.rs` (~2.8k), both near the 4,000-line cap and on every lane's
  path.

## Measurement discipline

This is the project's real asset. Protect it.

- Name the WPT/Test262 revision, the selection, and pass/fail/timeout/
  unsupported counts. Two runs, stable hashes.
- Never report a focused, hand-built selection as a broad pass rate.
- An inert pass — matching pixels for the wrong reason — is not credit. When a
  change exposes one, record the raw outcome as a mismatch and say so.
- If a documented baseline does not reproduce, **say so in `todo.md`**. Two have
  already been withdrawn: Test262's "196/196" (a 641-file sparse checkout
  against a real 53,872) and the geometry "38/23" (an unchanged renderer
  measures 32/29).
