# Upstream WPT render gate

`smoke.json` pins an exact commit from
`https://github.com/web-platform-tests/wpt.git`. The source is sparse-checked
out under ignored `target/wpt`; upstream test and reference files are never
copied into or modified by this repository.

Run on `win-001` from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1
```

The runner accepts alternate manifest, source, output, executable, viewport,
and timeout parameters. It reads each test's upstream `rel=match` or
`rel=mismatch` link, renders both sides through `amazing render-wpt`, applies
exact or upstream `meta name=fuzzy` pixel limits, and writes PPM
actual/reference/diff images plus a JSON report. A manifest's `expected` field
validates the harness smoke outcome; it cannot change the upstream assertion.
`.xht`/`.xhtml` tests and references are classified `unsupported`: WPT serves
them as XML, and Amazing does not yet have an XHTML parser mode.

`-SkipFetch` requires `WptRoot` already to be at the manifest revision.
`-SkipBuild` uses the selected existing executable.

`baseline.json` is the normalized, committed evidence from the ticket #1
repeatability gate. It records results and hashes only; `smoke.json` remains
the executable selection and expected harness outcome.

Ticket #2 adds a deterministic cascade/value inventory:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\inventory-render-wpt.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath target\wpt-cascade-baseline.json -SkipFetch
```

The committed `cascade-inventory.json`, `cascade-manifest.json`,
`cascade-results.json`, and `cascade-oracles.json` preserve the exact selection,
measured baseline, and exact-revision Blink/WebKit comparison.
Inventory provisions the pinned sparse checkout unless `-SkipFetch` is passed.

Ticket #2a uses `computed-values-manifest.json`, the fixed 24-case
Cascade+Variables projection of the baseline. `computed-values-results.json`
records its repeat-stable 4/24 → 16/24 result; the Variables half moved from
0/12 to 12/12. `cascade-results-ticket2a.json` records the full-corpus
30/72 → 43/72 measurement without overwriting the original baseline.

Ticket #3 corrects those historical totals in
`cascade-results-xhtml-safe.json`, then adds an HTML-only geometry inventory:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\inventory-render-wpt.ps1 `
  -CategoryCsv CSS2,css-box,css-display,css-inline,css-position,css-sizing `
  -InventoryPath target\wpt-geometry-inventory.json `
  -ManifestPath target\wpt-geometry-manifest.json
```

The committed `geometry-*` files preserve its 61-case broad diagnostic. The
nine-case `block-collapse-manifest.json` is the ticket gate;
`block-collapse-results.json` records 6/9 → 9/9 repeat-stable evidence and
`block-collapse-oracles.json` records exact-revision Blink/WebKit outcomes.

Ticket #3a uses `position-manifest.json`, the unchanged 12-case Position
projection of the geometry manifest. `position-results.json` records the
repeat-stable 5/12 → 9/12 result; `position-oracles.json` records the 3/5 → 5/5
Blink/WebKit-consensus result, and `geometry-results-positioned.json` records
the broad 35/61 → 38/61 movement.

Ticket #4 adds a broad text diagnostic plus a font-controlled shaping gate.
The reusable JSON profile keeps run parameters explicit while command-line
arguments remain authoritative:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ProfilePath tests\wpt-render\text-shaping-profile.json
```

`text-inventory.json`, `text-manifest.json`, `text-results.json`, and
`text-oracles.json` retain the 48-case Fonts/Text/Writing Modes/Ruby diagnostic.
`text-shaping-*` retains the 12-case behavior-stratified gate, raw pixel hashes,
and exact-revision Blink/WebKit results. Explicit runner parameters override
matching profile values, so the same profile can be reused with a different
viewport, executable, source tree, output directory, or timeout.

Ticket #4a records its before/after evidence in
`text-shaping-results-ticket4a.json`: 3/12 raw (0/8 capability-credited
Blink/WebKit consensus) became 12/12 and 8/8 consensus, with two identical
pixel-hash projections. `text-results-ticket4a.json` records the broad sample's
19/48 → 12/48 raw and 8/29 → 7/29 consensus movement; those decreases are
classified as former inert equal-pixel results becoming real mismatches after
web/system glyphs replaced heuristic rectangles.

Ticket #4b uses the 16-case `text-flow-manifest.json` and reusable dynamic
profile:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ProfilePath tests\wpt-render\text-flow-profile.json
```

`text-flow-oracles.json` records exact-revision Blink/WebKit outcomes.
`text-flow-results-ticket4b.json` records the 0/16 → 14/16 movement, the
0/14 → 14/14 consensus movement, two repeat-stable run ids, every final pixel
hash, and the normalized projection SHA-256. The remaining two mismatches are
triaged as static-position/float geometry rather than text shaping.

Ticket #4c uses the parameterized `vertical-ruby-profile.json` and 20-case
behavior-stratified Writing Modes/Ruby manifest. The corrected font-controlled
fixed point is 9/20 raw, 1/4 Blink/WebKit consensus, and 0/20
capability-credited; the final repeat-stable result is 11/20 raw, 2/4
consensus, and 8/20 capability-credited. `vertical-ruby-oracles.json` pins the
exact-revision browser runs and `vertical-ruby-results-ticket4c.json` retains
every final pixel hash plus the regression-gate run ids.

Ticket #5 uses the dynamic `flex-grid-profile.json` and a 24-case clean
Level-1 Flex/Grid gate. Inventory path/content filters exclude references that
depend on deferred float, table, inline-block, gradient, canvas, subgrid, or
Grid-Lanes behavior. The isolated `c8c371d` fixed point is 10/24; the final
repeat-stable result is 24/24, and every case passes both exact-revision Blink
and WebKit. `flex-grid-results-ticket5.json` records every final pixel hash,
the normalized projection, the 35/200 → 47/200 broad diagnostic, and prior
phase regression gates.

Ticket #6 uses `paint-compositing-profile.json` and a 24-case paint/compositing
gate covering the supported stacking, opacity, rounded clipping/background,
zero-blur shadow, and 2D transform slice. The isolated `c42df4f` fixed point is
12/24; the final repeat-stable result is 24/24 with a 23/23 exact-revision
Blink/WebKit consensus subset. `paint-compositing-results-ticket6.json` stores
the final hashes, bounded broad diagnostic, and regression gates; advanced 3D,
blend/filter, blurred-shadow, scrollbar, float, and inline-stacking
dependencies remain explicit rather than being relabeled as passes.
