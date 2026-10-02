# CSS-33 intrinsic size overrides

Base7c9e3652; pinned WPTc2a2fce; Boat/Linux pool-layout bx_shgjh66q.
Physical/logical intrinsic-size values now share physical cascade slots. Full
grouped shorthand and variable/default behavior preserve axis declaration order.

Crate362 tests /0docs; workspace3521 passed,17 skipped /4docs. Corrected static
parsing runner:3440/4805 ->3482/4805,42 exact gains/zero lost. Full sizing
render90/138 ->91/138, gained logical001/zero lost.

The runner previously mistook a present dynamic expected argument for omitted
expected and compared against specified text. Both measurements now use the
same corrected runner, skipping169 nonliteral cases. Baseline engine is exactly
7c9e3652; only examples/wpt_conformance.rs is overlaid from this change.
`gate.json` records the runner hash, assertion identities, and measured counts.
The seven false viewport passes from the old runner are not engine regressions:
new registered lengths correctly compute to px; static runner cannot evaluate
length_ref()'s DOM helper.

Reproduce using MAIN scripts/linux-run.sh css --host boat; native target
/tmp/amazing-target-css. Batch workspace nextest + workspace doctests +
wpt_conformance's default ten selections plus css-logical/css-images/css-sizing/
css-contain. Build amazing explicitly --bin amazing, run wpt_render_compare.py
css/css-sizing with800x600,timeout20,jobs4,--compact and before/compact.json.
Layout38 separately consumes resolved physical values and remembered-size state.
