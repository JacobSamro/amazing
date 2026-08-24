# Standards-grade text pipeline research

Ticket #4 answers how Amazing can replace character-at-a-time measurement and
rasterization with a standards-oriented text system without importing provider
types or unsafe code into the engine cores. Research and measurements were
performed on 2026-08-24. The implementation is intentionally deferred to the
next prototype ticket.

## Decision

Create a small, std-only `amazing-text` contract shared by layout and paint,
and place vetted font, Unicode, shaping, and raster dependencies behind an
isolated `amazing-text-provider` adapter. The driver owns one provider for the
page and passes it sequentially through style/layout and paint. Provider types
never appear in DOM, computed style, layout fragments, display items, or
persisted state.

The first provider will pin this pure-Rust stack exactly in `Cargo.lock`:

| Responsibility | Selected provider | Why |
|---|---|---|
| OpenType shaping | HarfRust 0.13.2 | Safe Rust implementation maintained in the HarfBuzz organization and aligned with HarfBuzz behavior used by Blink and WebKit |
| Font inspection and glyph rasterization | Swash 0.2.10 | Covers TrueType/CFF outlines, variations, OpenType/AAT shaping data, hinting, and color glyph rendering |
| Bidirectional analysis | unicode-bidi 0.3.18 | Safe Rust implementation of Unicode Bidirectional Algorithm (UAX #9) |
| Vertical glyph orientation | unicode-vo 0.1.0 | Safe Rust table lookup for Unicode Vertical_Orientation revision 17; provider types remain behind Amazing's owned contract |
| Grapheme, word, and line boundaries | icu_segmenter 2.3.0 | Current Unicode segmentation with explicit CSS `line-break` and `word-break` tailoring |
| WOFF/WOFF2 decoding | wuff 0.2.8 + brotli-decompressor 5.0.3 + flate2 1.1.9 | Pure Rust conversion to sfnt bytes; direct decoder pins let Amazing reject wuff's trusted output hint before allocation |

The versions compile at Amazing's declared Rust 1.89 MSRV. `unicode-vo` declares
no newer Rust requirement and forbids unsafe code. Dependency-internal unsafe
is allowed only inside the reviewed provider graph; both first-party crates
forbid unsafe code.

Cosmic Text is useful reference code but is not the engine interface: it owns
font fallback and wrapping policies that CSS must control. Allsorts is active
and can decode/shapes fonts, but does not target HarfBuzz parity as directly as
HarfRust. Native HarfBuzz bindings add a C build and unsafe FFI while the
maintained Rust binding does not offer a current parity advantage. Skrifa is a
strong alternative if Swash raster or metrics evidence fails during the
prototype; using both initially would duplicate font parsing without closing a
measured gate.

## Current seam is the failure

The existing implementation has no paragraph-level text object:

- `amazing-layout/src/font.rs` parses a subset of sfnt/TrueType tables and maps
  one Unicode scalar to one glyph. It has no GSUB, GPOS, CFF, kerning,
  variations, web-font decoding, or family fallback.
- `amazing-layout/src/inline.rs` collapses text into space-delimited words,
  sums scalar advances, and greedily wraps those words. It cannot preserve
  grapheme clusters or legal Unicode line-break opportunities.
- `WordFrag` stores text, x, width, and size, but not a font face, glyph id,
  cluster range, direction, baseline, or glyph offset.
- `amazing-paint/src/text.rs` repeats character-to-glyph lookup independently
  after layout. Measurement and painting can therefore disagree by design.
- Computed font, bidi, spacing, white-space, and writing-mode properties exist,
  but most are ignored below style. GPUI selects one Segoe UI face while the
  fixed renderer uses fallback rectangles, so results are embedding-dependent.

Adding kerning to `FontSource::measure` would deepen the wrong seam. Joining,
ligatures, fallback, caret mapping, bidi, and line breaking all require cluster
and paragraph context that a per-character API has already discarded.

## Reference-engine evidence

Blink's `HarfBuzzShaper` segments runs by script, direction, and fallback face,
shapes only at valid break/element boundaries, and returns glyph ids, clusters,
advances, offsets, safe-to-break flags, fallback fonts, and direction in its
`ShapeResult`. Its line breaker consumes that structured output rather than
measuring strings a character at a time.

WebKit's `ComplexTextControllerHarfBuzz` integrates HarfBuzz with ICU and its
font cascade. It retains HarfBuzz clusters and positions, applies CSS feature
precedence, handles vertical features and variations, and constructs fallback
runs. WebKit's text break iterators use ICU for line and word boundaries.

The transferable design is the high-level data contract, not either engine's
class hierarchy: itemize a paragraph, preserve shaping context across eligible
inline style boundaries, return positioned glyph clusters, and let CSS layout
choose lines only at legal/safe boundaries.

Primary references:

- [CSS Text boundary shaping](https://drafts.csswg.org/css-text-4/#boundary-shaping)
- [CSS Fonts font matching](https://drafts.csswg.org/css-fonts-4/#font-matching-algorithm)
- [Unicode Bidirectional Algorithm](https://unicode.org/reports/tr9/)
- [Unicode Line Breaking Algorithm](https://unicode.org/reports/tr14/)
- [Blink HarfBuzzShaper](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/platform/fonts/shaping/harfbuzz_shaper.h)
- [Blink LineBreaker](https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/core/layout/inline/line_breaker.h)
- [WebKit ComplexTextControllerHarfBuzz](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/platform/graphics/harfbuzz/ComplexTextControllerHarfBuzz.cpp)
- [WebKit TextBreakIterator](https://github.com/WebKit/WebKit/blob/main/Source/WTF/wtf/text/TextBreakIterator.h)
- [HarfRust](https://github.com/harfbuzz/harfrust)
- [Swash](https://github.com/dfrg/swash)
- [ICU4X segmenter](https://github.com/unicode-org/icu4x/tree/main/components/segmenter)
- [wuff](https://github.com/nicoburns/wuff)

## Owned interface

The contract uses owned, provider-neutral values:

- `FontKey` and `GlyphId` are opaque numeric ids scoped to one provider.
- `FontQuery` carries the ordered CSS family list plus weight, stretch, style,
  synthesis policy, language, feature tags, variation axes, and size.
- `ParagraphInput` contains one UTF-8 buffer, DOM source ranges, shaping-style
  spans, paint-only spans, atomic inline anchors, and explicit hard boundaries.
- `TextAnalysis` returns paragraph direction, bidi levels/runs, script runs,
  grapheme boundaries, and CSS-tailored line-break opportunities.
- `ShapeRequest` names a target byte range, its surrounding context, direction,
  script, language, candidate fonts, features, variations, and writing mode.
- `ShapedRun` contains its logical range, visual order, chosen `FontKey`, font
  metrics, and positioned glyphs. Every glyph retains its source cluster byte
  range and flags such as unsafe-to-break.
- `GlyphImage` returns bounded alpha/RGBA pixels plus bearing and dimensions;
  layout never inspects pixels and paint never recomputes advances.

The API shapes ranges with paragraph context. It must not offer a convenience
`measure(&str)` method because that would recreate the lossy path.

```text
DOM + ComputedStyle
        |
        v
std-only inline itemizer ---- paint-only spans
        |                     shaping spans + hard boundaries
        v
amazing-text contract <---- amazing-text-provider
        |                     HarfRust / Unicode / fonts / raster
        v
legal breaks + shaped clusters
        |
        +----> inline layout selects line boundaries and reshapes edges
        |                                      |
        v                                      v
positioned glyph fragments ------------> paint raster cache
```

Inline boundaries are semantic inputs. Color and text-decoration changes do
not break Arabic joining or ligatures. A change of face/size/weight creates a
new glyph run but keeps pre/post shaping context when the CSS boundary permits
joining. Non-zero inline padding/border, bidi isolation, forced breaks, and
atomic inline boxes create hard shaping boundaries. This distinction is
required by the focused WPT family and must live in itemization, not in the
provider.

Layout first filters ICU opportunities through `white-space`, `line-break`,
`word-break`, `overflow-wrap`, and hyphenation policy. It then chooses a line
that ends at a legal HarfBuzz-safe cluster boundary and reshapes the line edge
when boundary-sensitive forms require it. Bidi reordering operates on runs and
clusters; glyph arrays are never reversed scalar-by-scalar. Paint consumes the
same glyph ids and advances that established layout geometry.

## Font loading and fallback

The provider owns immutable font bytes and maps them to `FontKey`; CSS owns the
ordered family request and synthesis rules. The embedding layer supplies system
font candidates and generic-family mappings. `@font-face` descriptors and
source order remain CSS behavior. Network/session code fetches the selected
source, `wuff` decodes WOFF/WOFF2 to bounded sfnt bytes, and the provider
validates and registers the result. A font load changes font-set generation and
triggers style/layout/paint invalidation for affected faces.

Untrusted font handling needs explicit maximum compressed size, decompressed
size, table count, glyph count, bitmap dimensions, variation axes, and cache
memory. Decode/parse/raster errors return missing-face or missing-glyph results;
they never panic the browser. Fallback is resolved per cluster so combining
marks and emoji sequences are not split merely because one scalar is absent.

## Measured baseline

The broad diagnostic is revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, viewport 800×600, scale 1,
using the current `RectInkProvider`. The deterministic inventory scanned 5,958
documents and found 696 eligible HTML static reftests. A 12-case SHA-256-ranked
sample per category measured:

| Category | Amazing pass | Mismatch | Blink/WebKit consensus | Amazing in consensus |
|---|---:|---:|---:|---:|
| CSS Fonts | 4 | 8 | 9 | 2 |
| CSS Text | 2 | 10 | 8 | 1 |
| Writing Modes | 4 | 8 | 10 | 3 |
| Ruby | 9 | 3 | 2 | 2 |
| Total | 19 | 29 | 29 | 8 |

The broad raw count is diagnostic, not a text-capability rate: many tests pass
because both sides collapse to the same unsupported rendering.

The implementation gate is the 12-case behavior-stratified CSS Text shaping
manifest. It uses WPT's Noto Arabic, N'Ko, and Mongolian web fonts and covers
neutral/styled boundaries, hard boundaries, isolation, ligatures, and mark
positioning. Amazing measured 3 pass / 9 mismatch. Blink and WebKit both pass
eight selected cases; Amazing's raw result is 3/8, but all three are false
positives because no declared web font or shaped glyph cluster was used. The
capability-credited baseline is therefore **0/8**.

The parameter profile `text-shaping-profile.json` fixes manifest, source,
output, executable, 800×600 viewport, scale 1, and 20-second timeout while
letting explicit command parameters override any value. Normalized inventory,
pixel hashes, and exact-revision oracle evidence live beside the profile in
`tests/wpt-render`.

## Prototype sequence and risks

Ticket #4a should land one vertical slice:

1. Add the std-only contract and a deterministic fake provider for unit tests.
2. Add the isolated pinned provider and prove Rust 1.89 compatibility, safe
   first-party adapters, bounded malformed-font behavior, and deterministic
   repeat rasterization.
3. Parse the minimum `@font-face` descriptors and resolve local WPT font URLs
   through the existing page resource/session boundary.
4. Flatten inline content into paragraph/style spans, shape with context, store
   glyph clusters in layout fragments, and make paint consume those glyphs.
5. Rerun the focused gate twice and retain hashes; raw and consensus counts must
   improve without regressing block-collapse, position, or the workspace.

Subsequent text slices will add CSS-tailored wrapping/hyphenation and full bidi,
then vertical metrics/writing modes and ruby. The first slice must preserve
fields for those features so it does not hard-code horizontal LTR assumptions.

The principal risks are malformed-font resource consumption, provider version
drift, mismatched font ids between layout and paint, shaping across the wrong
inline boundary, and cached glyphs surviving a font-set generation change.
Exact pins, owned ids, bounded caches, generation keys, focused WPT cases, and
repeat hashes are the acceptance controls.

## Ticket #4a outcome

The prototype and production slice validated the selected seam unchanged.
`amazing-text` is std-only; `amazing-text-provider` pins the researched stack
and both first-party crates forbid unsafe. WPT web fonts now load through
bounded page resources, shaping preserves context across DOM/style spans,
fallback itemizes graphemes across the whole paragraph, and layout/paint share
font/glyph ids, vertical/horizontal advances, features, variations, synthesis
state, and inline padding/background geometry. The implementation bounds each
fetch/decode, the aggregate face set, and both raster-cache bytes and entries.

The focused gate moved from 3/12 raw and 0/8 capability-credited consensus to
12/12 and 8/8 exact-revision Blink/WebKit consensus. Two final 800×600, scale-1
runs were pixel-hash identical. The broader sample moved from 19/48 raw and
8/29 consensus to 12/48 and 7/29 because real web/system glyph output exposed
former unsupported equal-pixel passes. Normalized evidence is retained in
`tests/wpt-render/text-shaping-results-ticket4a.json` and
`tests/wpt-render/text-results-ticket4a.json`.

## Ticket #4b outcome

The throwaway line model established clusters—not scalars, words, or DOM text
nodes—as the common coordinate system for CSS transformation, original-source
mapping, legal/emergency breaks, bidi visual order, and discretionary glyphs.
Production retained the existing `TextSystem` seam and added an explicit
`shape_line` operation over an owned, provider-neutral paragraph contract.

Layout now carries typed `white-space`, `word-break`, `line-break`,
`overflow-wrap`, `hyphens`, `unicode-bidi`, and containing-block direction.
ICU4X supplies CSS-tailored UAX #14 opportunities and whole-grapheme emergency
breaks; unicode-bidi supplies paragraph levels and line visual runs. HarfRust
reshapes each selected logical line range with full hard-boundary context and
reports unsafe-to-break/concat flags. Transformed ranges retain original DOM
source mappings, tabs are layout controls, and selected U+00AD opportunities
append a separately shaped hyphen without mutating source text.

The pinned 16-case CSS Text/Writing Modes family improved from 0/16 to 14/16.
All 14 cases where exact-revision Blink and WebKit agree now pass. Two Arabic
edge-shaping cases remain mismatches because Amazing places their absolute
blue test box at the wrong static position around a float; the glyph output
matches the explicit reference, and Blink/WebKit disagree on both cases. Two
final runs are pixel-identical with normalized projection SHA-256
`5585ae3cc21d9284ac54a6222bad5dedbac0cccc83683cc9ecd4bd3f4293d332`.
Manifest, profile, oracle, and measured evidence live in `tests/wpt-render` as
`text-flow-*` artifacts.

Regression evidence after the final text-flow changes is Phase #4a 12/12,
block collapse 9/9, Position 9/12, full Rust 1.89 workspace pass, and standalone
locked GPUI check pass; exact run ids are recorded in the ticket #4b result.
