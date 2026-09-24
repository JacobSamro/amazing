# LAYOUT-16 pinned render evidence

The 449-case selection is `layout16-full-manifest.json`, pinned to WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` at 800 × 600.
`before.json` and `after-first.json` are the checkpoint sweep on fixed base
`93bb98ba`. The later source was preserved as WIP `57706d03`, merged with
local `dev`, and repaired ruby column wrapping in `4c038410`. The final paint
contract came from replaced-content's `9f9e694` through local `dev`
`d70aec65`, merged into lane head `ff3a7569`.

`after-paint-focus.json` records the six-case diagnostic on that merged head:
`border-collapse-empty-row` changes from mismatch (1,700 pixels) to exact
pass. The other five ruby cases remain mismatches.

The final fixed-dev comparison uses `current-dev-combined.json` versus
`after-paint-full.json` on the same 449-case manifest and WPT revision. Local
`dev` `d70aec65` passed **273/449**; the merged candidate passed **276/449**:
three gains, **zero pass losses**. The gains are
`row-auto-repeat-001.html`, `column-grid-lanes-oof-align-content-002.html`,
and `border-collapse-empty-row.html`. The collapsed-table case changed from
850 differing pixels on current dev to exact pass.

The initial current-dev run (`current-dev-before.json`) classified 17 cases as
unsupported because the lane's sparse WPT checkout temporarily omitted
`css/css-tables` and `css/css-images`. After restoring those paths, the same
dev executable ran only those 17 cases with
`current-dev-completion-manifest.json`; `current-dev-completion.json` records
15 passes and two mismatches. `current-dev-combined.json` substitutes those
17 results into the initial 449-case report. Both raw reports are preserved.

The win-002 gate on this source passed 285 layout nextest cases, 2,362
workspace nextest cases, and two workspace doctests. Its raw log and the
LAYOUT-17 timing diagnostic are under `../layout17/`.

The merged-head win-001 renderer used for that focus has SHA-256
`F59802E4A02DB9DD3F80208B8C475962AE9C2B1BAE87FBC847B1543BD1F2172B`.
