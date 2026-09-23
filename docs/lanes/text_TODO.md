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
