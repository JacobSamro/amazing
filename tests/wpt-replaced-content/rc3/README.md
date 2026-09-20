# RC-3: control chrome and a supplied-glyph contract

Baseline: origin/dev `0cc0ea0` (plus the lane's documentation-only RC-2 evidence
commit). Final: RC-3 owned paint changes. Both executable sets were rebuilt on
win-001 after the integrator's artifact-invalidating refresh; baseline sources
were restored with current mtimes, then final paint sources re-synced.
`CARGO_INCREMENTAL=0`; all Cargo commands used `--locked`. No Mac build/test.
Executable hashes are in `artifacts.json`; source/lock hashes and validation
counts are in `validation.json`. Matching source hashes supplement, not replace,
the artifact invalidation protocol.

## Selection and results

WPT pin: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
`../select-controls.ps1` inventories static `.html` tests with quoted upstream
rel=match links and direct control tags, excluding script-bearing files and
templates, in html/semantics/forms, html/rendering/widgets, css/css-ui and
css/css-forms. There are 63 distinct eligible candidates. Sort by SHA-256 of
relative path; in text-input, checkbox, radio, button, select, textarea order,
take the first four not already selected. This yields 24 distinct cases before
measurement. Category candidate counts overlap. No upstream HTML was edited.

| Gate | Before (both runs) | After (both runs) |
|---|---:|---:|
| Controls | 21 match / 2 mismatch / 1 unsupported | 21 match / 2 mismatch / 1 unsupported |
| Retained CSS Images PNG | 12/12 | 12/12 |
| Focused paint | 24/24 | 24/24 |

Zero timeouts/crashes. Six repeated pairs have identical classifications,
difference counts and actual/reference/diff pixel hashes. See the full reports
and `repeatability.json`; `../analyze-rc3.py` summarizes saved evidence only.
Twenty matching control cases change both actual and reference pixels; the
radio-no-centering match changes neither. Two select mismatch actuals change
while their references do not. No new control conformance passes are claimed:
references often contain the same controls and omitted labels/state, and
appearance keyword distinctions remain unsupported. Matches alone cannot prove
correct native appearance, selected text or bidi behavior.

The mismatches are customizable-select/select-base-appearance-writing-modes
(9,311 differing pixels after) and customizable-select/select-explicit-size
(20,453). These require base-select behavior; vertical writing and label payload
are also outside this implemented slice. These are observed requirements, not
a measured allocation of each differing pixel to a particular missing feature.

The zero-size checkbox case remains unsupported by the harness: its
`/css/reference/blank.html` exists in the pinned checkout, but
`scripts/run-render-wpt.ps1:267` joins root-relative hrefs to the test directory.
The integrator-owned harness issue is routed in questions.md. Reports retain
the runner's misleading "absent" detail verbatim; no case is dropped or rewritten.
The saved final control runs include css/reference in the sparse checkout.

## Reproduction on Windows

Use the pinned WPT checkout with the four selection roots plus css/reference,
css/css-images, css/support, fonts and the focused manifest's existing roots.
Keep the root and isolated shell target directories separate. Build each phase:

```powershell
$env:CARGO_INCREMENTAL='0'
$env:CARGO_TARGET_DIR='C:\Users\server\target\replaced-content'
cargo build -p amazing --bin amazing --locked
# Save the executable as target/rc3-driver-<phase>.exe.
$env:CARGO_TARGET_DIR='C:\Users\server\target\replaced-content-image'
cargo build --manifest-path crates/amazing-image/Cargo.toml --example render_reftest --locked
# Save examples/render_reftest.exe as target/rc3-images-<phase>.exe.
powershell -File tests/wpt-replaced-content/run-controls-gates.ps1 -Phase before -RepositoryRoot $pwd
# Repeat with -Phase after using final artifacts.
powershell -File tests/wpt-replaced-content/collect-rc3.ps1 -RepositoryRoot $pwd
```

The reports record viewport 800x600, scale 1, and font providers. The image adapter
uses the existing fallback font; root driver uses the existing Amazing text
provider and pinned font tree. This is headless evidence, not a GPUI screenshot.

## Delivered boundary

The display-list builder alone enumerates `styled_replaced_fragments()` and
flattens control geometry/style into commands. Rasterization uses supplied
rectangles with inherited clips and compositing. All six kinds have default
chrome; check/dot, disabled colors and positioned glyph lines can be supplied
through `ControlProvider` and `paint_document_with_text_resources_and_controls`.
Glyph lines use absolute layout coordinates before control transforms; paint
clips them to the content rectangle and shares the existing glyph raster path.
No DOM state reads, label selection, shaping or measurement are added.

Existing entrypoints supply unchecked/enabled/empty-label defaults. DOM state
and layout shaping/placement are routed to their owners; production checked,
disabled, value and selected-label behavior is not claimed until wiring lands.
Only controls exposed by layout's inline replaced view are handled. Native
appearance is a bounded platform-neutral design, not platform widget fidelity.

Visible author backgrounds/borders and appearance:none are honored. Computed
initial transparent/none cannot distinguish explicit author defaults; per the
integrator's decision, ambiguous defaults are treated as absent styling. CSS
provenance is routed separately. No paint color-parser evolution, shell edits,
root graph edits, or lockfile edits are included.

Paint: 86 passing tests, including 12 new control tests. Full root workspace:
1,029 passed, zero failed, three ignored across 52 targets. The isolated image
example builds with --locked using the integrator's corrected shell lockfile.
