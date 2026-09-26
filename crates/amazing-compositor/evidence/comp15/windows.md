# COMP-15 native gate — 2026-09-26T09:37:49Z

Source `661ebb6262ef85057b3e1f392fd57635b3b8ea11`, fixed base `f5f97c39c3c2b6b9c45af7dd518a46ee7ff71156`.
Host win-001, lane compositor, `scripts/win-run.sh --ref` fresh archive, shared
build slot. All commands exited successfully; no build/test ran on macOS.

| Gate | Passed | Skipped |
| --- | ---: | ---: |
| amazing-compositor nextest | 75 | 0 |
| amazing-compositor doctests | 0 | 0 |
| workspace nextest | 2495 | 9 |
| workspace doctests | 2 | 0 |

The compositor adds 13 native tests to the previous 62. They cover filters,
all blend modes on transparent backdrops, nested/isolation groups, shapes and
paths, mask modes, cross-tile/offscreen blur, source-free effect sampling and
reserved blur halos, backdrop scope/mirror edges, snapshot rejection and
round-trip, bounded layer squashing, and atomic owner validation.

`cargo run -p amazing-compositor --example effects_trace --locked` uses a
512×512 viewport and fallback text. Results are in `win001-layer-parity.csv`.
IA reflow: 1 layer; IA effects-scroller: 4 layers; article: 1 layer. Every page
has 262144 exact pixels, 0 layer-rounding pixels and 0 losses. The fixtures
represent the IA pages after their specified mutation, plus the article.

These pages prove retained behavior against today's full painter. They do
**not** prove new CSS effect parity: current painter effect metadata carries
only opacity and its reference rendering defers filter/blend support. The
four new HTML effect fixture pages are staged, not claimed passing. CSS
registration/classification, painter effects/raw metadata/reference spaces,
IA scheduling and IPC V2 transport adoption remain with their owning lanes.
See `../../EFFECTS.md` and `docs/lanes/compositor_TODO.md`. COMP-15 stays open.

GPUI screenshot: pending interactive RDP session, nonblocking.
Local scratch transcript: `.agents/compositor/comp15-win001.log`.
