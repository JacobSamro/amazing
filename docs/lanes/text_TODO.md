# Text lane decision map

## TEXT-1: font matching and named-family fallback

Fixed base: `d2a610ede8f254b8da1be9121d59dbd7cf598e78`.

Decision: select faces in CSS family-list order. Within a family, match width,
style, then weight before testing glyph coverage. If the selected face lacks a
cluster, advance through the remaining named families before installed fallback.
Private-use characters only use explicitly named families. The Windows driver
registers a bounded set of pinned CSSTest faces when `AMAZING_WPT_ROOT` is set.
Use synthetic oblique when the selected face has no matching italic face.

Evidence: `crates/amazing-text-provider/evidence/text1-fonts-manifest.json`
pins 15 CSS Fonts cases at WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Untouched baseline was
5 pass, 10 mismatch, twice. Final candidate was 11 pass, 4 mismatch, twice,
with identical classifications and actual PPM hashes across runs: six gains,
zero losses. Four gains (`font-family-name-018`, `-019`, `-022`, and
`test-synthetic-italic`) match the Chrome 154 / Safari 26.6 consensus recorded
in the CSS-8 triage. `font-family-name-000` and `-024` also pass locally, but
both reference browsers fail those pinned cases. The remaining mismatches are
`font-variant-ligatures-11`, the PUA primary-font case, and
`first-available-font-002`/`-003`.

On the same pinned revision, text-flow stayed 14/16, text-shaping stayed
12/12, and vertical-ruby stayed 11/20 from untouched baseline to two candidate
runs. There were no classification losses, and both candidate runs had
identical actual PPM hashes for every case. One already-mismatching
vertical-ruby case changed pixels between baseline and candidate.

Follow-up: layout must pass parsed `font-feature-settings` and
`font-variant-ligatures` into `TextStyle.features`. The provider already passes
OpenType feature tags to HarfRust; the missing policy handoff is queued with
the primary integrator.

## TEXT-2: web-font decoding and face matching

Fixed base: `582a6af0962dd3b7e6c94427c14746915aea14ab` (`origin/dev`),
with the local TEXT-1 commit replayed as `085d0df35f06059dc93abce907f2d25a5a69786f`.

Decision: keep font decoding and CSS face matching in `amazing-text-provider`.
Use bounded wuff WOFF 1 and WOFF 2 decode through the existing DEFLATE and
Brotli adapters, then validate sfnt table order, bounds, overlap, padding,
individual table checksums, the `head` magic, and parser limits. Do not reject
an otherwise valid font solely because `head.checkSumAdjustment` makes the
whole-file checksum stale: most pinned CSSTest fonts have this property. A
strict whole-file check caused four TEXT-1 losses before it was removed.

The provider's `FontFaceDescriptor` and `FontFaceLoader` API takes ordered
`local()` and URL sources, filters `format()` and `tech()`, applies Unicode and
weight/stretch/style descriptor ranges, and records `font-display`. The
network owner supplies CORS-mode URL bytes under the provider's 8 MiB cap.
The CSS owner must parse descriptors; the driver must register faces and
relayout on generation change. These cross-lane seams are queued in
`.agents/text/questions.md`. Until they land, the page still uses its older
partial `register_web_font` path, and WPT cannot exercise the new matching API.

Evidence: `crates/amazing-text-provider/evidence/text2-gate-summary.json` and
the linked per-selection reports pin WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. The eight-case
`@font-face` selection was 0 pass, 2 mismatch, 6 page-script crashes before
and after; only two cases reached a rendered comparison, and their actual
image hashes were unchanged. The six crashes were `TypeError: value is not
callable` in page script; all six fixtures use `document.fonts.ready.then(...)`
and that DOM/driver seam is routed into DOM-17. The retained TEXT-1, text-flow,
text-shaping, and vertical-ruby selections held at 11/15, 14/16, 12/12, and
11/20 respectively,
with zero classification or actual-image-hash losses. Two candidate runs had
identical classifications and actual image hashes. Provider nextest passed
35/35 plus the ignored pinned WPT decoder case 1/1; workspace nextest passed
1728/1728 (4 skipped) and workspace doctests passed 2/2. All builds and tests
ran on win-001 against the text lane target dir under the shared build slot
for render and workspace gates.

Next: CSS descriptor parsing, CORS font fetch, and driver registration/relayout
must land before an end-to-end `@font-face` WPT movement can be expected.

Rebase update: TEXT-2 alone was replayed onto `origin/dev`
`285c16ee01956b1a04d7f247eb977f43417badb0`, whose tree already contains
TEXT-1. The provider and workspace gates were repeated on that head in the
text lane's refreshed Windows tree: provider nextest 35/35, pinned decoder
1/1, workspace nextest 1753/1753 (4 skipped), and workspace doctests 2/2.

## TEXT-3: synthesis, feature shaping, fallback, and pending loads

Fixed implementation base: `e2d7ad6ef21b013f0576a6065408a036caf8c3f4`.
Keep synthetic weight and style in the selected face's raster request. A normal
face is sheared by 14 degrees for italic or oblique requests only when synthesis
is enabled; a slanted face is never sheared again. Swash outline emboldening
uses a size-relative stroke offset. Installed fallback selects the closest face
within each generic alias after exhausting the requested named families.
Private-use characters remain restricted to explicitly listed named families.

`TextStyle.features` already reaches HarfRust. A synthetic GSUB font now proves
on and off behavior for `liga`, `clig`, `dlig`, `hlig`, and `calt` at the provider
boundary. Layout still owns the computed CSS property handoff into this vector.
The read-only `pending_font_loads()` reports zero while font registration is
synchronous; the driver can use it for DOM-17 and later connect async loading.
Net has supplied `FontRequest::new` and `NetworkService::start_font` for the
caller-owned URL loader, which remains outside the text provider.

Evidence: `crates/amazing-text-provider/evidence/text3-gate-summary.json`
links the Windows reports at WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. The frozen 15-case font
selection stayed at 11 pass and 4 mismatch from the freshly measured before
run to the candidate, with zero classification losses. Synthetic italic and
the already mismatching ligatures case changed actual pixels. Retained text
flow stayed 14/16, shaping 12/12, and vertical/ruby 11/20 against the TEXT-2
reports. The refreshed Windows tree had no intervening changes to either text
crate; its base was `6bec0a06626673d690ff0442c6c2706dbc85b2e4`.
Provider nextest passed 48/48 plus pinned decoder 1/1; workspace nextest
passed 1781/1781 with four skipped tests, and workspace doctests passed 2/2.

Remaining font selection mismatches: the ligatures case requires the layout
computed-feature handoff, and the PUA primary-font case needs page-path
diagnosis. The two first-available-font cases use a web font without U+0020;
the provider's metric selection currently chooses the first face without
checking space coverage, and page `@font-face` registration is still pending.

## TEXT-4: first available font for metrics

Implementation base: local TEXT-3 commit `34fde0d`. For `TextSystem::metrics`,
select the first matching face with actual U+0020 coverage, including the
`@font-face` unicode-range restriction and the font's character map. Ordinary
shaping still keeps uncoded whitespace with its surrounding run. Synthetic
font tests cover both missing cmap space and a font with a space glyph hidden
by unicode-range. The page-path `@font-face` loader remains caller-owned.

Evidence: `crates/amazing-text-provider/evidence/text4-gate-summary.json`
compares the TEXT-3 final reports with the TEXT-4 win-001 render reports at the
same pinned WPT revision. Font selection remains 11/15; retained text flow
14/16, shaping 12/12, and vertical/ruby 11/20. Every case kept its
classification and actual image hash. The two first-available-font cases still
mismatch because page `@font-face` registration is pending; the provider
behavior is covered by tests for missing cmap space and excluded unicode-range.
On win-002, provider nextest passed 50/50 (one skipped), workspace nextest
passed with 1,782 tests enumerated, and workspace doctests passed 2/2.

## TEXT-5: vertical orientation in provider shaping

Implementation base: local TEXT-4 commit `9714fa2`. In vertical-rl/lr,
upright clusters select the OpenType `vert` substitution and sideways clusters
disable it. Both disable `vrt2` as an automatic orientation choice; mixed
orientation follows Unicode Vertical Orientation for the first visible
character of each cluster. Existing upright bidi handling forces left-to-right
ordering. Synthetic-font tests cover feature substitution, mixed Latin/CJK
orientation, an ignorable prefix, and Hebrew bidi in both vertical modes.

The frozen seven-case vertical selection stayed at 2/7 between TEXT-4 and
TEXT-5 on win-001. Retained font selection stayed 11/15, flow 14/16, shaping
12/12, and vertical/ruby 11/20. All 70 cases retained their classifications
and actual image hashes. The vertical fixture that requires
`CSSHWOrientationTest.otf` still needs caller-owned `@font-face` registration,
so the synthetic provider tests are the direct evidence for `vert` behavior.
Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Evidence: `crates/amazing-text-provider/evidence/text5-*-after.json` and
`text5-vertical-before.json`. Windows unit gate results are in the TEXT-5
report.

## TEXT-6: web-font loading and stylesheet provenance

Implementation base: `a4ab541`. The page loader now passes each sheet's URL to
font registration, including inline and linked sheets and nested `@import`.
Linked stylesheets keep the final fetched URL after redirects; inline and render
extra author sheets use the document URL. The CSS `Stylesheet::base_url` carries
this provenance for CSS-13. HTTP(S) font URLs resolve against that sheet URL
and load through `FontRequest` and `NetworkService`, while `data:` and WPT
`file:` sources use the page resource loader. Registration honors source order,
`local()`, supported `format()`/`tech()`, unicode ranges, weight/stretch/style
ranges, display periods, size and metric overrides, and face feature settings.
The provider exposes pending-load periods and applies size and feature metadata
to shaping and metrics. Synchronous page loading finishes before first layout.

Evidence: `crates/amazing-text-provider/evidence/text6-gate-summary.json` links
before and after reports for six frozen selections at WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Font selection improved
11/15 to 12/15 (`first-available-font-003`), while font-face stayed 2 pass /
6 crash, vertical 2/7, flow 14/16, shaping 12/12, and vertical/ruby 11/20.
All 78 cases retained prior passes. The `text-combine-upright-compression-001`
actual image changed within its existing mismatch classification. On win-002,
selected nextest passed 389/389 (4 skipped), workspace nextest 1856/1856
(5 skipped), and workspace doctests 2/2.

Remaining clusters: `first-available-font-002`, PUA primary-font selection, and
ligature variant mismatch need diagnosis; font-face reftest crashes await the
DOM-17 `document.fonts.ready` work. Installed `local()` lookup currently covers
known platform faces and the pinned WPT orientation fixture.

## TEXT-7: exposed font-face cases and vertical orientation

Implementation base: `5177362`, which includes DOM-17. That binding removes
all six previous `document.fonts.ready.then(...)` page-script crashes: the
font-face selection reaches rendering at 2 pass / 6 mismatch. TEXT-7 normalizes
reversed descriptor ranges and fractional stretch endpoints, raising it to
3 pass / 5 mismatch. Selected variable face axes now reach shaping and
rasterization, with explicit variation settings taking precedence. The five
remaining font-face mismatches need exact computed stretch percentage and
oblique angle from layout for Fonts 4 section 5.2 face ranking.

In mixed vertical text, UAX50 `Tr` codepoints are upright and receive `vert`.
Sideways modes ignore `text-orientation: upright` during bidi analysis. These
provider fixes raise the seven-case vertical selection from 2/7 to 5/7.
Remaining failures are the Ahem vertical-lr inline placement and the complex
`direction-upright` bidi/layout case. The retained vertical/ruby selection
holds at 11/20 but has one gain and one **pending loss**:
`text-combine-upright-line-breaking-rules-001` loses because
`amazing-layout/src/inline.rs::split_atomic_units` inserts an allowed break
before the combined U+FFFC object despite the provider's no-break boundary.
LAYOUT-14 item 0 owns that fix; TEXT-7 must merge after it. Other ruby,
text-combine, sideways flex, and orthogonal baseline mismatches are routed to
layout. Evidence: `crates/amazing-text-provider/evidence/text7-gate-summary.json`
at pinned WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

## TEXT-8: exact font matching contract

`TextStyle` now exposes `stretch_percent: f32` and
`oblique_angle_degrees: Option<f32>`. A zero width value temporarily uses the
legacy integer `stretch`; `None` uses the CSS default 14-degree oblique angle.
LAYOUT-14 item 5 will fill both exact computed values in
`box_tree.rs::text_style`. The provider now preserves fractional face descriptor
ranges, ranks width then style (including oblique angle) then weight using CSS
Fonts 4 section 5.2, and applies selected exact width/angle to variable font
axes. Explicit `font-variation-settings` still overrides matched axes.

On the TEXT-7 base `9eda828`, the pinned font-face selection improved 3/8 to
4/8 (`font-descriptor-range-reversed-002`), while the 15-case font selection
stayed 13/15, vertical and vertical/ruby stayed 5/7 and 11/20, and retained
flow and shaping stayed 14/16 and 12/12. Across 78 cases no classifications
were lost relative to the retained baselines. The vertical/ruby
`tcy` line-break loss introduced in TEXT-7 remains pending LAYOUT-14 item 0,
so the combined series must merge only after that fix. Evidence:
`crates/amazing-text-provider/evidence/text8-gate-summary.json` at WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

## TEXT-9: contingent object line breaks

The provider supplements ICU's line opportunities at U+FFFC (the unresolved
UAX #14 CB class), allowing breaks before and after atomic inline placeholders,
including adjacent objects. Higher-priority OP, CL/CP/EX/IS/SY, QU, GL, WJ,
ZWJ, space and mandatory-break rules still suppress their respective
boundaries. The provider also filters any ICU opportunity that would violate
those neighbor rules. `white-space: nowrap` remains authoritative. This is the
provider seam needed by LAYOUT-14 item 0; its separate `split_atomic_units`
rule must still stop inserting a break after `「`.

## TEXT-10: first-available-font relative metrics contract

`TextSystem::font_relative_metrics(&TextStyle) -> FontRelativeMetrics` exposes
`ch_advance`, `ic_advance`, `x_height`, `cap_height`, and `line_height_normal`
in used pixels for LAYOUT-14 item 1. The provider selects the first face that
contains U+0020, including registered web fonts, then measures U+0030 and
U+6C34 in that same face with selected variation axes and text orientation.
Missing glyphs use CSS Values 4 defaults: `ch` is 0.5em (1em for upright
vertical text), `ic` is 1em, `ex` is 0.5em, and `cap` uses the selected font's
ascent when cap height is unavailable. The default text system provides
stable synthetic measures and `normal` line height from its metrics.

## TEXT-11: contingent object breaks in the fallback

The provider and `FallbackTextSystem` now share the UAX #14 LB20 boundary
policy for U+FFFC. The fallback supplies opportunities around adjacent atomic
inlines while preserving the higher-priority punctuation and nowrap rules.
On the combined TEXT-7–10 plus LAYOUT-14 item 0 head, this resolves both
`inline_replaced_tests` regressions. Linux workspace: 2,004 passed, five
skipped; workspace doctests: two passed. The Windows workspace and doctest
gates also passed. Commit: `7f378e2` on `2b11484`.

## TEXT-12: remaining pinned font and writing-mode failure audit

On `origin/dev` `1d53f79`, the four remaining eight-case font-face mismatches
request fractional `font-stretch` values and oblique angles from
`css/css-fonts/matching/font-matching.css`. Layout's
`box_tree.rs::text_style` still passes `stretch_percent: 0.0` and
`oblique_angle_degrees: None`, so the provider's exact Fonts 4 ranking cannot
be exercised by these pages yet. The remaining `first-available-font-002`
failure uses `ch`; layout does not yet call the provider's
`font_relative_metrics` contract. Both are routed to LAYOUT-14 items 5 and 1.

The other font-selection mismatch compares Arial and Times rendering of nine
PUA codepoints. The installed `arial.ttf` and `times.ttf` on win-001 both lack
all nine cmap entries, and the WPT mismatch-reference image is identical to
the test image (zero differing pixels). The provider already prohibits generic
and installed fallback for PUA; with those installed faces, CSS Fonts 4
requires the same missing-glyph result on both sides. A controlled font
fixture is needed for that case to discriminate implementations.

The remaining vertical-lr Ahem placement, nested upright bidi, sideways flex,
orthogonal baseline, text-combine and ruby pairing/overhang cases exercise
layout inline geometry or ruby structure. Exact cases and paths are recorded
in `.agents/text/questions.md` for the layout lane. The retained seven-case
vertical selection is 5/7 on the TEXT-12 build.

## TEXT-12: unquoted data font URL fidelity

The 40-case pinned font and writing-mode selection improved from 33 pass,
5 mismatch, 1 timeout, 1 crash to 34 pass, 5 mismatch, 0 timeout, 1 crash.
`font-feature-settings-descriptor-binary.html` now passes: the font-face
parser preserves plus signs and numeric tokens in an unquoted base64 data URL,
and page font warnings use bounded source labels so redirected stderr cannot
block the render runner. No previous pass was lost. The six retained selections
have unchanged classifications (4/8 font-face, 13/15 fonts, 5/7 vertical,
11/20 vertical/ruby, 14/16 flow, 12/12 shaping). Evidence is in
`crates/amazing-text-provider/evidence/text12-*-after.json` at WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

The five remaining expanded mismatches are routed to CSS and layout for
`@supports font-kerning`, used kerning features, and letter-spacing with
ligature precedence. The remaining crash is the DOM/driver
`getElementById("content")` script-order issue. No remaining failure in this
selection has a text-owned cause established by the audit.

## TEXT-13: font-size-adjust metric resolution and current-dev re-audit

`FontSizeAdjustMetric` (`ExHeight`, `CapHeight`, `ChWidth`, `IcWidth`,
`IcHeight`) and `TextSystem::adjusted_font_size(&TextStyle, metric, aspect)` are
the text-owned half of CSS Fonts 4 §2.6 `font-size-adjust`. The default trait
method returns `style.size` unchanged, so no caller had to change. The provider
resolves the requested metric against the first available font through the same
`font_relative_metrics` path as `ch`/`ic`, then returns
`style.size * aspect / font_metric_ratio`; a zero aspect yields a zero used size
and a missing or non-positive font metric falls back to `style.size`. Layout
adopts it by computing each element's used size and passing it as
`TextStyle.size`; that handoff is routed, not yet wired. Commit `929c301a`.

The same ticket re-measured the pinned selections on current dev `0d6b60b0`,
after LAYOUT-14 exact stretch/oblique values, CSS font-kerning and layout letter
spacing/feature mapping landed. The retained 15-case font selection moved
**13 pass / 2 mismatch → 14 pass / 1 mismatch**
(`first-available-font-002` gained once layout adopted `font_relative_metrics`);
the only remaining failure is the environment-owned
`font-unicode-PUA-primary-font` (0 differing pixels, installed Arial/Times
fixture). The expanded 40-case selection moved 34 pass /
5 mismatch / 1 crash to **38 pass / 2 mismatch / 0 crash**. Gains:
`font-feature-resolution-001`, `font-feature-resolution-002`,
`font-kerning-01`, `font-kerning-04`, and the `font-variant-ligatures` crash
(resolved by DOM/driver). One loss: `font-kerning-02` (345 px), caused by
layout's `font_features::from_style` enabling both `kern` and `vkrn` for
`font-kerning:normal` in horizontal text. Retention held for font-face 4/8,
flow 14/16, shaping 12/12. Evidence:
`crates/amazing-text-provider/evidence/text13-gate-summary.json`.

Remaining expanded mismatches are layout-owned and routed:
`font-kerning-02/03` (orientation-aware kern/vkrn), the four
`css-fonts/matching/*` variable-axis cases (layout hard-codes
`TextStyle.synthesis` to the default, so `font-synthesis:none` is ignored), and
`font-size-adjust-zero-2` (layout must use `adjusted_font_size`).

The two unassigned `css-text` word-break-shaping cases
(`overflow-wrap-shaping-001`, `line-break-shaping-001`) are also layout-owned:
with the pinned `NotoNaskhArabic-regular.woff2`, `shape_line` returns glyph ids
identical to the whole-word `shape` for every split, so Arabic joining context
survives the line edge, and `line_breaks` emits the correct Emergency/Anywhere
opportunities. The residual failure is layout break selection / float
shrink-to-fit.

## TEXT-15: automatic opsz variation axis

CSS Fonts 4 section 7.2's `font-optical-sizing: auto` (the initial and default
value) sets the `opsz` variation axis to the used font size unless an explicit
`font-variation-settings` value overrides it. `font_variations::for_face` now
pushes an `opsz` axis from `TextStyle.size` before the author-face
`wght`/`wdth`/`ital`/`slnt` axes; the existing explicit-override loop still lets
an author `font-variation-settings` `opsz` win. A face with no registered
`opsz` axis silently ignores the coordinate, the same as the existing
`wght`/`wdth` behavior for non-variable faces. Unlike the width/oblique
contract in TEXT-8, this needed no `TextStyle` field and no layout handoff:
`auto` is the initial value, so the common case is a provider-only change.
`font-optical-sizing: none` remains unimplemented — no field distinguishes it
from `auto` yet, and adding one would require a layout literal update in
`amazing-layout/src/box_tree.rs::text_style` (which constructs `TextStyle` as
a full field literal, not `..Default::default()`), so it stays deferred rather
than landed as a half-wired field. Commit `a0f68c06` on `bbb0267a`.

No pinned WPT `font-optical-sizing` fixture exists in this lane's frozen
selections and the Boat sandbox still has no WPT checkout (per TEXT-14's
filed gap), so the gate is four focused synthetic unit tests against
`font_variations::for_face` directly: automatic tracking of `TextStyle.size`
with no author face, tracking across size changes, an explicit
`font-variation-settings` `opsz` overriding the automatic value with no
duplicate axis entry, and `opsz` coexisting with an author-face `wght` axis.
Provider/text crate nextest: 68/68 (1 skipped) before, 72/72 (1 skipped)
after, zero losses. Doctests green. Workspace nextest on the merged head
(`dev` fast-forwarded past TEXT-14 by 76 commits, none touching the text
crates): 2,806 passed, 10 skipped, 0 failed; workspace doctests green.

Re-audited TEXT-14's second candidate, the `font-variant-caps`/`-numeric`/
`-position`/`-east-asian` GSUB feature contract: on inspection this needs
little to no new text-owned code. Explicit OpenType feature tags already
reach HarfRust generically through the existing `TextStyle.features` vector
(the same seam TEXT-14 used for `kern`/`vkrn`), so mapping each CSS keyword
(`small-caps` to `smcp`, `all-small-caps` to `smcp`+`c2sc`, `oldstyle-nums` to
`onum`, `super`/`sub` to `sups`/`subs`, etc.) to feature tags is CSS/layout
work into that same vector, per the precedent TEXT-12 already established for
`font-kerning`. The one genuinely provider-owned piece is CSS Fonts 4 section
6's synthesis fallback: when a requested feature (chiefly `smcp` for
small-caps) is unsupported by the selected face and `font-synthesis` allows
it, the UA may synthesize the effect (glyph substitution plus scaling for
small-caps; baseline shift plus scaling for super/subscript). That is real
raster-level work comparable in size to the existing synthetic bold/oblique
path, not a quick follow-on to opsz. `amazing_text::FontSynthesis` currently
has only `weight`/`style` bools and layout constructs it with
`FontSynthesis::default()` (not a full literal), so adding `small_caps`/
`position` fields would be safe for layout's build, but landing the fields
without the synthesis behavior they gate would be a half-wired contract.
Proposed as the next ticket rather than started here.



## TEXT-14: axis-correct kerning features

CSS Fonts 4 §5.2.1 makes `kern` the horizontal-axis and `vkrn` the
vertical-axis kerning feature, but an explicitly requested HarfBuzz buffer
feature runs in either buffer direction. Layout maps `font-kerning: normal` to
kern=1 plus vkrn=1 in both orientations
(`amazing-layout/src/font_features.rs`), so a horizontal run shaped with the
wrong-axis feature and regressed `font-kerning-02`. The provider now filters
both the `@font-face` descriptor features and the explicit `TextStyle.features`
to the tag matching the actual shaping buffer direction
(`uses_vertical_metrics() && orientation == Upright` → TopToBottom). An explicit
`kern`/`vkrn` off value still disables its own axis; an on value no longer leaks
onto the other axis. Commit `0e4afdd0`.

Evidence: `crates/amazing-text-provider/evidence/text14-gate-summary.json`,
`text14-kern-before.json` and `text14-kern-after.json`, at the pinned WPT
revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, measured on the text lane
Boat sandbox under Linux/fontconfig/freetype. The five pinned kerning cases —
every kerning case in the 40-case expanded selection — moved **3 pass / 2
mismatch → 5 pass / 0 mismatch**: `font-kerning-02` (horizontal) and `-03`
(vertical upright) each gained, while `-01`, `-04` and `-05` retained their
exact actual/reference PPM SHA-256. The before mismatches were exactly 345
differing pixels in the `.high` line. The provider and text crate nextest gate
is 68/68 (one skipped); workspace nextest is 2736 passed, 10 skipped, 0 failed;
workspace doctests are 4 passed, 0 failed.

The full 15/40/48-case pixel selections cannot run on the Boat sandbox: there is
no WPT checkout and no PowerShell, so only a focused `render-wpt` subset with a
lane-local comparator was possible. The shared-runner / WPT-provisioning gap is
filed in `.agents/text/questions.md`. With the two kerning cases closed, no
remaining mismatch in the pinned text selections has an established text-owned
cause: the retained font-face, flow, vertical and vertical/ruby failures are
layout-owned per the TEXT-7..13 routes, and the PUA/emoji cases are environment
ones. The lane has asked the orchestrator for its next ticket, proposing
`font-optical-sizing: auto` (`opsz`) and a `font-variant-caps/-numeric/-position`
feature contract as provider-local candidates.

## TEXT-16: small-caps and super/subscript synthesis fallback

CSS Fonts 4 section 6: when the selected face has no `smcp`/`c2sc`/`sups`/
`subs` GSUB feature and `font-synthesis` allows it, the UA may synthesize the
effect. `amazing_text::FontSynthesis` gains `small_caps`/`position` bools
(default `true`, matching `font-synthesis`'s initial value `weight style
small-caps position`) alongside the existing `weight`/`style`. `resolve_synthesis`
leaves both false — it only compares `FontAttributes` (weight/stretch/slant),
which has no GSUB visibility — so the provider resolves them separately, per
glyph cluster, in `shape_range`.

New module `crates/amazing-text-provider/src/font_variant_synthesis.rs`:
`FaceFeatureSupport::probe` parses the selected face's GSUB feature list once
per `shape_range` call (via the `read-fonts` crate, pinned to the exact
version `harfrust` already depends on transitively, giving `TableProvider::
gsub()`/`cmap()` on the same `harfrust::FontRef`); `feature_on` checks the
already-built `Feature` list for a requested tag; `for_cluster` is a pure
decision function (needs_smcp/c2sc/sups/subs booleans, an injectable
uppercase-glyph-lookup closure, and the cluster's representative character)
that is unit-tested directly with fake closures, no real font bytes needed.

`shape_range` wires it in: an originally-lowercase letter requesting `smcp`
gets its shaped glyph id substituted to the face's cmap-mapped uppercase
glyph (skipped, not guessed, if the face has no such glyph); an
already-uppercase letter under `c2sc`, or any glyph under `sups`/`subs`,
keeps its shaped id. Superscript/subscript get a shape-time baseline offset
(+0.33em / -0.14em, added to `offset_y`) so rasterization only needs a
uniform outline scale. `ShapedRun`s now split on synthesis equality as well
as source span, so one run with mixed original case gets separate runs per
distinct needed synthesis. `rasterize_font_glyph` composes the small-caps
(0.72) and position (0.66) scales with the existing oblique skew via
`zeno::Transform::then`/`then_scale`, identical output to before when only
`style` applies (verified by the retained oblique/bold tests).

The exact scale factors and em shifts are approximations the same way the
existing bold stroke width and -14 degree oblique skew already are; CSS does
not mandate synthesis metrics.

Not implemented: `pcap`/`c2pc` (petite-caps) and `titl` (titling-caps) reach
HarfRust generically through the same `TextStyle.features` vector already
(no text-owned change needed, same as TEXT-14's kerning conclusion), but have
no synthesis fallback — CSS Fonts 4 does not mandate one for them, unlike
small-caps/position. `unicase` has no associated GSUB tag or synthesis path
and was out of the coordinator's requested scope.

Gate: 11 new unit tests in `font_variant_synthesis` (substitution, the
no-uppercase-glyph fallback, `c2sc`/`smcp` case gating, digits/punctuation
untouched, super/sub direction and scale, combinations) plus 6 in `lib.rs`
(raster transform mirroring the existing bold/oblique tests; the full
`shape()` path through a new two-glyph synthetic font with real cmap bytes,
including the small-caps-policy-disabled case). Crate nextest 72/72 ->
89/89 (1 skipped), zero losses, doctests green. Workspace nextest 2882
passed/10 skipped/0 failed, workspace doctests 4/0, on merged dev + TEXT-15.
No pinned WPT `font-variant-caps`/`-position` fixture exists in this lane's
frozen selections, and the Boat sandbox still has no WPT checkout (TEXT-14's
filed gap, unchanged), so this gates on unit tests per the fallback
instruction. Commit `628ec199`.

The CSS/layout keyword-to-feature mapping needed to actually exercise this
from real pages — computed `font-variant-caps`/`font-variant-position` into
`TextStyle.features`, and computed `font-synthesis`'s new `small-caps`/
`position` keywords into `TextStyle.synthesis` — is routed in
`.agents/css/questions.md` and `.agents/layout/questions.md`; the CSS side of
the audit found no CSS-owned gap (both properties are already parsed and
computed as plain keyword strings), and the layout side ties into the
already-open `font-synthesis` wiring request from TEXT-13.

## TEXT-17: Linux glyph rendering was a solid box for every character

Root cause: `register_system_text_faces` in `crates/amazing/src/page.rs` (and
`FontFaceLoader::load_local` in `crates/amazing/src/font_loader.rs`, for
`src: local(...)`) hardcoded Windows-only `C:\Windows\Fonts\*.ttf` paths for
the generic families (`serif`/`sans-serif`/`monospace`/`system-ui`) and named
families (`Arial`, `Times`, `Consolas`, ...). `amazing_net::fetch` on those
paths fails on Linux, so every one of those `continue`s silently: zero font
faces were ever registered, for any family, on Linux. Pages that rely on the
UA default (`body { font-family: serif }`, the common case — `docs-index.html`
in the real-pages corpus declares no `font-family` at all) then resolved to
no usable face, and the engine's no-face fallback path painted every glyph as
a full-coverage box instead of an outline (visible in
`amazing-shell-gpui/evidence/linux/shell-docs-index.png`: chrome text, drawn
by GPUI, was fine; every engine-rendered glyph was a filled rectangle).

Headless reproduction first, per instruction: `amazing render
tests/real-pages/pages/docs-index.html out.bmp 960` on the Boat sandbox,
converted to PNG and inspected — confirmed the same solid-box rendering
headless, isolating the bug to the engine's paint/font pipeline rather than
anything GPUI-specific.

Fix, in `crates/amazing/src/font_loader.rs` (text's outright file — kept the
`page.rs` diff to only the already-granted `register_system_text_faces`
function body, which now dispatches on `cfg(target_os = "windows")` and
otherwise calls into `font_loader`): a bounded scan of the standard Linux
font directories (`/usr/share/fonts`, `/usr/local/share/fonts`,
`$HOME/.fonts`, `$HOME/.local/share/fonts`; depth <= 8, <= 4096 files) with
filename-only classification into sans/serif/mono buckets (`serif` in the
name and not `sans` -> serif; `mono` in the name -> mono; else sans), scored
to prefer well-known, broadly-hinted families (DejaVu > Liberation > Noto >
Freefont/Free) and plain regular weights over bold/italic/condensed
variants, deterministic tie-break. The winning file per bucket is registered
under the generic family plus a handful of common named aliases pages
request directly (sans -> `sans-serif`/`system-ui`/`Arial`/`Helvetica`, serif
-> `serif`/`Times`/`Times New Roman`/`Georgia`, mono ->
`monospace`/`Courier`/`Courier New`/`Consolas`). `load_local` gets the same
treatment for `src: local(...)` lookups via a small alias table into the same
three buckets. No new dependency: pure `std::fs` scan, no fontconfig binding.

On the Boat sandbox (Ubuntu 24.04, `pool-layout`), this resolves to DejaVu
Sans/Serif/Sans Mono (97 font files present: DejaVu, Liberation, Lato, Noto,
URW base35, Droid, FontAwesome). Headless re-render of the same page: PNG
grew from 2,791 bytes (solid boxes) to 62,527 bytes (real antialiased glyph
outlines); visually confirmed by reading both PNGs.

Regression test: `glyphs_render_as_outlines_not_solid_boxes` in
`crates/amazing/src/driver_tests.rs` renders `<div style="font-size:120px">
OOO</div>` with no explicit `font-family` (exercising the same UA-default
path as docs-index.html) through `cmd::render::render_surface`, finds the
tight bounding box of dark pixels, and asserts foreground coverage within
that box is under 85% — a real "O" has a light interior counter, a
full-coverage box does not. Verified both directions: fails at 100.0% dark
coverage with `register_linux_system_faces` temporarily disabled, passes at
the real (much lower) coverage with the fix in place.

`scripts/linux-shell-capture.sh` failed on this pooled sandbox for an
unrelated reason (`amazing-shell-gpui/Cargo.lock` "cannot update the lock
file because --locked was passed" — a stale lockfile issue on the shared
sandbox, not Xvfb-related); per the coordinator's stated fallback, the
headless render PNG is the proof, saved at
`.agents/text/captures/text17-glyphs-headless.png` (+ `.json` metadata).

Gate: crate tests `amazing-text`/`amazing-text-provider`/`amazing` 340/340
(6 skipped), including the new regression test, and the real-pages corpus
harness (`amazing::real_pages`) green. Workspace nextest 2993/2993 (11
skipped), workspace doctests green. Commit: see `.agents/text/status.json`.
