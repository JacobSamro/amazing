# Bounded public bootstrap and exact-ref render gate

Fixed base: `1710e4d39a460d0208f7ed2ffda33c4995458fa0`.
No engine source, corpus selection, expected value, or comparison threshold changes.

The Boat bootstrap no longer writes a corpora symlink in lazy HOME. It bounds
remote setup mkdir (60 seconds), environment copy/load (30 each), native source
mkdir/link (30 each), native source sync (600), native tools directory (30), and
native nextest copy (120), with five-second kill grace. SSH connection and
keepalive timeouts and rsync idle timeout are explicit. Every native stage emits
its actual exit, raw native log path/hash/bytes and a short diagnostic tail;
compact receipt publication to HOME is separately bounded to 20 seconds. Stage
or publication failure exits 3 before user Cargo. Existing raw evidence is not
recovered or modified. User build/test commands keep their own actual exit.

Public usage, from any lane worktree (source comes from Git, including sparse trees):

```sh
bash scripts/linux-render-wpt.sh layout <owned-WPT-directory> ... \
  --ref <candidate-full-SHA> --baseline-ref <fixed-base-full-SHA> \
  --workspace --release --compact --width 800 --height 600 --jobs 4
```

One invocation runs workspace nextest plus doctests, builds the headless renderer,
and runs every supplied pinned static reftest directory on the baseline and
candidate through supported linux-run. Both sides use the same directory/options
selection. Every render compares against the actual baseline map; a missing
required baseline is an error. Existing baseline mismatches remain classified;
only passing classification losses fail the comparator. A failed native phase
still allows doctests and the remaining corpus diagnostics to run. Build/WPT
setup failures prevent renderer execution. No historical accepted-index is
required when `--baseline-ref` is supplied. The existing `--baseline auto|PATH`
mode remains supported and now requires the requested baseline to exist.

Only `gate.summary.json` and available `*.compact.json` maps are fetched. Raw
logs, pixels and full reports remain on Boat. No clusters/globs/archive pulls.
Phase summaries include actual command and output-copy exits, diagnostics,
raw paths/hashes, nextest/docs counts and listed directory comparisons. The
output tag, before/after role and exact source separate runs (including the same
engine ref on both sides); reuse of an output directory fails.
This gate does not claim absent external decoded42/geometry4 captures. Genuine
native layout fixtures remain included by workspace nextest.

Local metadata verification: eight public composition/bootstrap fixtures plus
nine existing gate-tooling fixtures; shell syntax and whitespace checks. Includes
actual shell assembly, finite failing stage receipt, both-phase continuation,
actual baseline argument, missing-baseline rejection and compact-only paths.
No local engine build or Boat qualification has been run for this ticket.

Actual qualification completed on the unique supported alias
`public-render-proof-20261005`, box `bx_zxyagf6x`, owner session 72843, terminal 0.
Both sides intentionally used published engine `1710e4d3`; qualified tooling was
`7db5aeed` (preserved on its source branch). Each side passed workspace 4,458,
17 skips, and four doctests. All five workspace/docs/setup/build/render phase
exits were zero. Both bootstrap sequences completed all seven stages with zero
actual exits and separately preserved compact receipts. The representative
static `css/css-backgrounds` limit-eight maps were identical: four passes and
four existing mismatches, zero losses or gains. This qualifies public tooling,
not a Layout feature or full browser result. No clean rerun was needed.

Only compact summaries, maps and bootstrap receipts (31,163 bytes total) were
fetched. Full logs, reports and pixels remain on Boat. Per-case classifications,
phase and output-copy exits, raw host paths/hashes and compact receipt hashes
are in `public-bootstrap-render-qualification.json`. Root received stop-safe
handoff before local packaging. The READY squash keeps qualified script bytes.
