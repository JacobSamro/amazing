# CSS text flow and bidi design

Phase #4b answers how Amazing selects and paints horizontal lines without
losing DOM offsets or splitting typographic character units.

## Prototype decision

The throwaway terminal model exercised collapsed whitespace, mixed LTR/RTL,
manual soft hyphens, and emergency Devanagari wrapping. Its scalar-indexed
emergency case split `क्षि` into four lines. Supplying grapheme clusters as
the canonical items kept each conjunct intact and let every other operation
refer to the same ranges. The prototype shell was then deleted.

The durable model is:

- transformed paragraph bytes plus transformed-to-original source mappings;
- contiguous shaping-style spans and an explicit paragraph base direction;
- ICU legal/mandatory opportunities plus separately classified emergency and
  discretionary-hyphen opportunities;
- HarfRust glyph clusters with unsafe-to-break and unsafe-to-concat flags;
- line requests expressed as logical cluster-aligned ranges; and
- visual runs and visual cluster ranges returned in paint order.

## Module seam

`amazing-text` remains the small std-only interface. CSS/layout choose policy,
perform source-preserving whitespace transformation, and select a line using
provider advances. `amazing-text-provider` executes ICU segmentation,
unicode-bidi analysis, font fallback, HarfRust shaping/line-edge reshaping,
and rasterization. Provider dependency types remain private.

CSS `embed`, `bidi-override`, `isolate`, `isolate-override`, and `plaintext`
are retained as paired nested inline-box ranges and represented as internal
Unicode formatting controls only during bidi analysis. They never enter source
mappings or paint. `dir` participates as a
low-specificity presentational hint, so author CSS can override it before the
computed direction reaches text layout.

`<br>` is retained as a paragraph separator and replaced/atomic content as a
U+FFFC anchor. During bidi analysis, a nested scope that crosses a paragraph
separator is closed before the UAX #9 `B` character and reopened after it;
paired controls never become unmatched across the reset. Final UAX #9 ordering
moves the actual replaced fragment, rebuilds inline background/padding
fragments at visual coordinates, and keeps vertical/horizontal advances in the
same pass. A discretionary hyphen is shaped and included in line fitting
before commit; its edge follows the resolved bidi run, including an RTL
embedding inside an LTR paragraph.

ICU tailoring is selected at each style span while grapheme segmentation is
always performed once over the full paragraph, so a style boundary cannot
manufacture a break inside a combining sequence. Harf unsafe flags classify
edges that require reshaping. A selected soft line is reshaped with full
pre/post context deliberately: CSS Text requires joining forms to behave as if
the soft wrap were absent. Inline padding/background geometry is then reapplied
to the returned visual run. Every glyph fragment retains all intersecting DOM
source mappings, including both caret positions around a cross-node ligature or
collapsed space.

Untrusted work is capped before output allocation: paragraph bytes, scopes,
scope segments across paragraph resets, source mappings, grapheme clusters,
itemized spans, and the product of clusters × faces × requested families each
have independent limits. The scope × bidi-class-`B` separator intersection
count is preflighted with checked arithmetic before analysis. Span lookup,
unsafe-edge classification, atomic-unit
splitting, replaced-fragment placement, neighboring-style selection, and
layout unsafe-offset lookup use monotonic cursors, precomputed indexes, or
binary search rather than cross-product scans.

## Evidence

- WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`
- Viewport: 800×600 at device scale 1
- Baseline: 0 pass / 16 mismatch
- Final: 14 pass / 2 mismatch
- Blink/WebKit consensus: 0/14 → 14/14
- Stable runs: `run-20260824T171622013Z-10452` and
  `run-20260824T171649948Z-7956`
- Projection SHA-256:
  `5585ae3cc21d9284ac54a6222bad5dedbac0cccc83683cc9ecd4bd3f4293d332`

The two remaining WPTs use the same Arabic shaping assertion and are pixel
identical in glyph output. Their mismatch is the pre-existing absolute
static-position of a blue box around a floated reference; Blink passes and
WebKit fails both at this exact revision. They remain in the diagnostic rather
than being relabeled as text failures or removed from the gate.

Post-change regressions are measured, not inferred: Phase #4a shaping is
12/12 (`run-20260824T170639613Z-10276`), block collapse is 9/9
(`run-20260824T170721180Z-16236`), and Position remains 9/12
(`run-20260824T170754663Z-10516`). The Rust 1.89 workspace and the standalone
locked GPUI check both pass.
