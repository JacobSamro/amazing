# Live computed CSSOM driver gate

Whole pinned `css/cssom` HTML/XHTML testharness selection at WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`: 191 pages, 5 seconds, 8 jobs.
Selection SHA-256: `538d9b86963e6ed84d587783ca8ae2074120b0f1d64cbd5f7c2d37b70f7f3885`.

| Measure | Before | After |
|---|---:|---:|
| Workspace nextest passed | 3709 | 3714 |
| Skipped | 17 | 17 |
| Doctests passed | 4 | 4 |
| Passing pages | 6 | 20 |
| Passing named subtests | 586 | 1169 |
| Failed / timeout / error / unsupported pages | 134 / 16 / 33 / 2 | 119 / 23 / 27 / 2 |
| Lost passing pages / named subtests | — | 0 / 0 |

Before source: `961bdcc1b94d96ead412181e0efdbbff936e684b`.
After executable source: `4564e6e3c7be86a9444809f4e2993d2359b8a51a`.
Both sides include identical CSS42–45, live-frame helper and general
ArrayIterator dependency `57d219d23a3279971a12e0a6716825f17df5864a`.

Binary SHA-256 before: `4dd9333557ce0d06aac81efc03dd19f97131ef43954353ff230bc4d7d7c631d8`.
After: `9ab0b5c1c6d58502f13f32e618225d5bfb9f43cdb60f5ba5bcfaf934dd56c0ba`.
Full occurrence-aware reports are `before.json` and `after.json`.

Run the bulk recipe via main `scripts/linux-run.sh integrator-cssom --host
boat --ref <explicit-sha> -- 'bash crates/amazing/evidence/cssom-driver/gate.sh
<label> ["$OUT/cssom-before/results.json"]'`. The recipe builds and tests on
native `/tmp`, runs workspace nextest with four test threads, doctests and
the whole corpus. Comparison rechecks previously passing cases lost to
deadlines at 60 seconds; this gate needed none. It fails on any lost passing
page or occurrence-aware named subtest.

Remaining boundaries: mutable stylesheet/declaration APIs, CSS48 computed
pseudos without boxes, Layout45 actual generated/inline/iframe geometry and
insets, retained animation/resource state, and child Window realm installer
glue. These are explicit follow-on areas; current unsupported geometry uses
computed values. Functional pseudos absent from the lower selector enum are
not fabricated.
