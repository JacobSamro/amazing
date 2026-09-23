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
