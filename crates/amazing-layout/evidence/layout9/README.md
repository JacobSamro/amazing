# LAYOUT-9 outer SVG geometry evidence

The frozen selection in `geometry-selection.json` maps twelve static states to
the pinned WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`:

- `svg/coordinate-systems/outer-svg-intrinsic-size-001.html`, SHA256
  `d47d180231b6bdb7314a5b62a59e555bdbda94adce78e88178fb156bae2123b8`;
  seven of its scripted states are reproduced as static HTML.
- `css/css-sizing/svg-intrinsic-size-002.html`, SHA256
  `5a8e31473126191ca20e2bdde1407cd409adadf69e00ce2eb7c10bfba33e81ea`;
  all five SVG dimension states are reproduced with the same 600px container.

`geometry.py` creates one HTML document per state and invokes the `amazing
layout` command on win-001. It reads the SVG root's replaced-fragment content
rectangle from the layout dump, compares width and height with the WPT expected
values to 0.05 CSS px, and records executable and selection hashes. It does
not use rendered pixels or the SVG paint implementation. The dynamic WPT case
is frozen into separate static documents so this gate can run before DOM script
mutation support lands.

Baseline: 0/12 on both repeats, executable SHA256
`8c6dc33954f199cb7088f9ad010d4979af51912c12414050381cd4d9769ce238`,
selection SHA256
`1cd090b30156ca3029f553e464bd80886d68ed27e159d1f53aeb831046109ba8`.
Final: 12/12 on both repeats, executable SHA256
`823aedf95016aac3ee98fc5ffaedcc0e245385070d96e1f8388180bab9e8e57f`.

Paint adoption seam: `ReplacedKind::Svg` fragments expose the SVG root in
`StyledReplacedFrag.node` and its CSS pixel viewport in `content_box`. A
block/flex/grid item's `LayoutBox.svg_root` marks the same DOM root, with
its viewport in `LayoutBox.content_box`. The renderer receives that root,
viewport width/height and the caller's device-pixel ratio. SVG descendants
remain in the DOM and do not generate CSS boxes. See the routed request in
`.agents/layout/questions.md` for `display_list.rs::control_commands`.

## Retained and workspace gates

`reports/` contains 44 complete pinned WPT render reports: baseline/final,
two repeats, broad plus ten retained selections. `summarize.py` requires the
per-case classifications and actual/reference/diff hashes to match between
repeats, then writes `retained-summary.json`. Every retained selection has
**zero gains, zero losses and zero changed image hashes** between baseline and
candidate. Broad is 83/200 on both sides; geometry is 42/61. The renderer
executables are the baseline/final hashes above. The unchanged static image
adapter has SHA256
`484f3d713e2d3734ed6719ac18f2534a1c0b772f2cc4b6513fed20371a227587`.

`workspace-gate.json` and `workspace-gate.log` record the win-001 build-slot
run: 1632/1632 nextest passes, 3 skipped, and 2/2 workspace doctests. The
46-file `final-sources.json` manifest matched the Windows tree before and
after the gate and again after the retained sweep. No build or test ran on
the Mac.
