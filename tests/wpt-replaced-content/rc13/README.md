# RC-13 frozen SVG recovery evidence

Fixed source base: `cb3064b` (CSS-10 included). WPT revision:
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Selection:
[`../rc11-svg-manifest.json`](../rc11-svg-manifest.json), all 35 cases.
The renderer driver is integrator-assist's frozen `7e48a0e`
`crates/amazing-image/examples/render_reftest.rs`, built on win-001 with its
adapter-only fetch/net dev dependencies in the server scratch manifest. The
production `amazing-image` manifest and shell lock omit those adapter edges.

`pre-rc12-report.json` is IA's `859bb53` predecessor measurement. The two
`final-recovery` reports were rendered before the crate split; the two
`after-move` reports were rendered after it. Each report's matching
`-nonwhite.json` counts nonwhite pixels in the actual and reference PPMs.
`comparison.json` records every transition and verifies stable actual,
reference and diff SHA-256 values across all four recovery runs.

| Phase | Pass | Mismatch | Nonblank passes | Retained pass losses |
| --- | ---: | ---: | ---: | ---: |
| Pre-RC-12 (`859bb53`) | 22 | 13 | 22 | — |
| Recovered 1 and 2 | 26 | 9 | 23 | 0 |
| After move 1 and 2 | 26 | 9 | 23 | 0 |

All four recovery runs have identical classifications and per-case hashes.
Three passing cases are blank matches: `ellipse-auto-rx-textpath`,
`pathlength-005`, and `non-scaling-stroke-007`; they receive no painting credit.
The one newly passing `pathlength-005` is among these blank matches.

The selection was run with `scripts/run-render-wpt.ps1` using
`-ManifestPath tests\wpt-replaced-content\rc11-svg-manifest.json`,
`-WptRoot target\wpt`, `-SkipFetch`, `-SkipBuild`, the frozen adapter executable,
`-FontSet 'Fallback font; exact IA adapter 7e48a0e'`, and an 800×600 viewport
at DPR 1. The Windows runs and PPM artifacts were scratch; the reports and
pixel counts here are the durable evidence.
