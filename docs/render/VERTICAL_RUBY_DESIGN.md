# Vertical writing and ruby design

Phase #4c asks how Amazing can add vertical metrics, glyph orientation,
logical-axis line layout, text-combine, and ruby without forking the horizontal
text pipeline or leaking provider/GPUI types into engine cores.

## Measured baseline

- WPT revision: c2a2fce69e5c2509d9dc28440ec8b42532e5c293
- Viewport: 800×600 at device scale 1
- Behavior-stratified gate: 20 static HTML reftests
- Baseline run: run-20260824T205327644Z-11456
- Amazing baseline: 9 pass / 11 mismatch / 0 unsupported / 0 crash
- Amazing final: 11 pass / 9 mismatch / 0 unsupported / 0 crash
- Exact-revision Blink: 11/20
- Exact-revision WebKit: 5/20
- Blink/WebKit consensus: 4/20
- Amazing consensus movement: 1/4 → 2/4
- Capability-credited Amazing movement: 0/20 → 8/20
- Stable final runs: run-20260824T215012613Z-7956 and
  run-20260824T215045287Z-2216
- Final projection SHA-256:
  b8dea2a3b184ac5f0a0b327cbe9f0da4438e3de72ce43c41cb29f98f6bd4c9bd

Raw equal pixels are not capability credit. The baseline passes include
test/reference pairs that flatten to the same horizontal or non-ruby structure
when their distinguishing feature is absent. The 20-case diagnostic remains
intact even though Blink and WebKit diverge on nine cases and both fail seven
at this revision.

The baseline was rebuilt from fixed point
32eeebd00dcf689ada968206c98f86019e4303e9 in an isolated win-001 snapshot.
The first exploratory run was discarded because a prior inventory had removed
the manifest's fonts sparse path; every recorded baseline/final run includes
CSSHWOrientationTest.otf and the declared WPT font tree.

## Prototype decision

The deleted terminal prototype exercised horizontal-tb, vertical-rl,
vertical-lr, sideways-rl, sideways-lr, mixed/upright/sideways cluster
orientation, text-combine, and ruby placement. One logical-axis model handled
every state:

- writing mode resolves an inline progression vector and a block/column
  progression vector;
- vertical upright runs use top-to-bottom provider shaping, canonical or
  synthesized vertical advances, and per-glyph vertical origins;
- sideways runs keep horizontal shaping and carry one clockwise or
  counter-clockwise placement transform;
- text-orientation:mixed resolves pose per grapheme/glyph cluster using the
  Unicode Vertical_Orientation property;
- text-combine-upright becomes one atomic 1em inline item containing a fitted
  horizontal run; and
- ruby is paired base/annotation structure placed on the logical line-over
  side, never a second unowned text stream.

Physical x/y conversion happens only when line fragments are placed. Paint and
the GPUI presentation adapter consume the resulting owned geometry; neither
reconstructs logical axes or measures text.

## Implemented slice

- `writing-mode`, `text-orientation`, and `text-combine-upright` have inherited
  computed-value metadata and typed provider policy.
- Sideways-lr/rl use horizontal shaping with explicit clockwise/counterclockwise
  glyph pose; mixed vertical text itemizes by Unicode Vertical_Orientation.
- HarfRust top-to-bottom shaping and Swash canonical/synthesized vertical
  metrics/origins remain behind the owned contract; raster paint rotates and
  scales mask/color glyph images from final fragment transforms.
- Vertical inline layout wraps against content height, advances columns on the
  logical block axis, and maps Flex row/column directions through writing mode
  and direction.
- Text-combine creates a 1em atomic inline cell with fitted horizontal glyphs.
- Ruby retains base/annotation roles from HTML tags or computed display,
  supports autohide, empty bases, generated ruby-text roles, spanning
  annotation containers, horizontal line-over ascent, and vertical line-over
  placement.

The final raw improvements close upright/sideways orientation,
text-combine line-breaking/RTL placement, ruby-base/ruby-text display, autohide, and empty
base behavior. Remaining mismatches stay explicit: mixed fallback coverage,
upright directionality, orthogonal inline-block baselines/sizing, TCY
whitespace/compression, nested/anonymous ruby pairing, and vertical overhang.

## Module seams

amazing-text owns provider-neutral writing mode, cluster pose, vertical
metrics/origins, and shaped advances. amazing-text-provider maps that contract
onto HarfRust/Swash/unicode-vo. Layout owns CSS policy, logical axes,
text-combine atomic fitting, ruby pairing, line construction, and physical
fragment geometry. Paint consumes glyph ids plus final pose/geometry. GPUI
composes the resulting page surface inside the browsing frame.
