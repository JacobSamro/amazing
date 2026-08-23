# todo.md — Amazing engine handoff

> Handoff note. Read this top-to-bottom before touching code.
> Authoritative docs: `ARCHITECTURE.md` (locked decisions), `AGENTS.md`
> (build policy: **everything builds/tests on win-001**, never macOS).

## Where the project stands

| Phase | Status |
|-------|--------|
| 0 — skeleton | ✅ done, all exit criteria met on win-001 |
| 1 — HTML parser | ✅ done on win-001 (51 workspace tests green) — see report gaps |
| 2 — CSS | ✅ done on win-001 (88 workspace tests green) — see Phase 2 section |
| 3 — Layout | ✅ done on win-001 (workspace green; curated css/CSS2 subset 17/17) |
| 3.5 — Flexbox | ✅ done on win-001 (13 flex tests; row/column/wrap/grow/shrink/justify/align/gaps/auto-margins/order) |
| 3.6 — Grid | ✅ done on win-001 (16 grid tests; track lists incl. fr/%/repeat+auto-fill, row/column/dense placement, line numbers + negative anchors, implicit tracks, alignment) |
| 4 — Paint | ✅ crate integrated, workspace green (43 paint tests); `render` demo writes BMP+PPM |
| 5 — Net | ✅ crate integrated, workspace green (61 net tests incl. loopback HTTP); `fetch` demo |
| 6 — JS | ✅ tree-walking milestone exit criteria met (142 crate tests); focused Test262 slice 196/196 supported (100%) |
| 7 — DOM/JS | ✅ live DOM exit criteria met: handlers mutate layout/style and repaint visibly end-to-end |
| 8 — GUI | ✅ Windows native shell and interactive desktop exit criteria verified on win-001 |
| 9 — Media | 🔄 images/audio + ICE/RTP/RTCP/SDP + vetted DTLS-SRTP + D3D11 milestones landed |

Local editing tree: `/Users/jMac/Projects/OpenSource/amazing`.
Server tree: `C:\Users\server\src\amazing` (git lives there).
Sync loop: edit locally → `scp -r` changed paths → build/test via ssh → commit on server.
**No rsync on Windows. No git push from macOS (cmd.exe breaks it) — commit on the server.**
CI artifact: `.github/workflows/ci.yml` runs the workspace build and test gate on
push and pull request using the Windows toolchain.

## Historical Phase 1 handoff checklist (superseded)

The detailed checklist below is retained as design history only. Phase 1 was
completed and verified; do not treat the old "in-flight" instructions as
current work items. The authoritative current status is the completed report
immediately below and the phase table above.

### 1. Historical tokenizer rewrite design

Historical note: an early draft had invalid Rust and was replaced by the
compiled tokenizer described in the completed report below.

Design that was intended (keep it):
- `Tokenizer { input: Vec<char>, pos, state, queue: VecDeque<Token>, text_buf,
  tag: Option<TagBuilder>, is_end_tag: bool, comment_buf, doctype_name,
  raw_end: String, eof_sent }` — all fields on the struct, one impl block.
- `State` enum mirrors WHATWG §13.2.5 names: Data, TagOpen, EndTagOpen,
  TagName, Before/AttrName/AfterAttrName, Before/AttrValue{Double,Single,
  Unquoted}, AfterAttrValueQuoted, SelfClosingStart, Comment*, BogusComment,
  Doctype states, RawText/RcData + their LessThan/EndTagOpen/EndTagName.
- `Token` enum already defined in the draft (Doctype/StartTag/EndTag/
  Comment/Characters/Eof) — keep its shape; `tree.rs` will match on it.
- Start vs end tags distinguished by `is_end_tag` flag set when entering
  TagName from EndTagOpen. One `TagBuilder` type for both; end tags drop attrs.
- Attribute accumulation via `PendingAttr { name, value }`;
  `finish_attr()` commits into the tag (first attribute wins on duplicates,
  per §13.2.5.33). Names lowercased ASCII-only. Values entity-decoded with
  `in_attr=true`.
- After emitting a start tag named `script`/`style` → switch to RawText;
  `textarea`/`title` → RcData; store name in `raw_end`. Appropriate end tag =
  accumulated name equals `raw_end`; mismatch dumps `</name…` back into text.
- Entity consumption is a method `fn consume_reference(&mut self, in_attr:
  bool) -> Option<String>` living IN tokenizer.rs (the draft wrongly imported
  it from entities). Numeric path full spec incl. the 32-entry C1
  Windows-1252 replacement table and FFFD for 0/surrogates/out-of-range.
- EOF inside a tag reconstructs literal text (attrs may be lost — acceptable,
  document it).

### 2. Fix `crates/amazing-html/src/entities.rs`

- Remove/bump `MAX_NAME_LEN` (8 is too small — `&thetasym;` is 10);
  simplest: delete the const, use a local window of 12 chars in the matcher.
- Legacy no-semicolon matching: char *after* the name is `rest[name.len()]`
  (names include the leading `&`) — the draft indexed wrong. In attributes,
  legacy refs followed by `=` or alphanumeric must NOT decode (spec rule).
- Keep the three table-integrity tests passing; they encode real invariants.

### 3. Write `crates/amazing-html/src/tree.rs` (~600–800 lines)

Historical planning note. This was the biggest piece of Phase 1 and is now
implemented. Spec: §13.2.6
"Tree construction". Required insertion modes:

```
Initial, BeforeHtml, BeforeHead, InHead, AfterHead,
InBody, Text,
InTable, InTableBody, InRow, InCell, InCaption, InColumnGroup,
AfterBody, AfterAfterBody
```

Must-haves (each maps to a planned test):
- implied `<html>/<head>/<body>` synthesis from bare text/comments
- void elements (area base br col embed hr img input link meta param source
  track wbr) never take children
- auto-closing `</p>` semantics: new block start closes open p (button scope)
- `li` / `dd` / `dt` sequences close their predecessors (scope walk rules)
- implicit `<tbody>` for `<table><tr><td>`
- foster parenting: stray text/nodes inside table context insert *before*
  the table (`Dom::attach_before`)
- RAWTEXT/RCDATA content lands as merged text under script/style/textarea/title
- active formatting elements list + Noah's Ark clause (cap 3 identical) +
  reconstruction + adoption agency algorithm (§13.2.6.5) — the classic gate:
  `<b>1<p>2</b>3</p>` must yield `<b>1</b><p><b>2</b>3</p>` shape
- markers pushed for caption/td/th/applet/marquee/object
- `image` start tag rewrites to `img`; `</br>` becomes `<br>`
- comments attach at current position; doctype name captured into
  `Dom::doctype`
- scope checks: generic scoping set (applet caption html table td th marquee
  object template) + list-item (+ol ul) + button variants

Use the new Dom low-level API added in Phase 1: `alloc_unattached`,
`attach_last`, `attach_before(parent, Some(sibling), idx)`, `detach`,
`append_text_data`, `name()`, `attrs_of()`, `set_attrs()`.

### 4. Wire up

- `crates/amazing-html/src/lib.rs`: replace placeholder with
  `pub fn parse(src: &str) -> Dom` driving `Tokenizer` → `TreeBuilder`;
  keep module docs listing deviations (encoding assumed UTF-8 lossy;
  CDATA treated as bogus comment; frameset ignored; select/optgroup minimal;
  PUBLIC/SYSTEM ids consumed but uncaptured; quirks flags recorded only).
- `crates/amazing/src/main.rs`: add `parse [file]` subcommand (stdin when no
  arg): read bytes → `String::from_utf8_lossy` → `amazing_html::parse` →
  print `dom.dump(NodeIdx(0))`. That IS the Phase 1 demo artifact.

### 5. Tests (exit criteria)

- Tokenizer (~10): tag stream; attr quoting styles + dedupe; comment forms
  incl. abrupt close; doctype case-insensitive + missing-name quirks;
  entities (named w/ ';', legacy without, numeric dec/hex, C1 table, bad →
  FFFD); script keeps inner `<div>` as raw text; textarea decodes entities;
  self-closing flag; EOF mid-tag literal fallback.
- Tree (~14): nesting dump shape; unclosed `<p>` autoclose; implicit tbody;
  li sequence; **adoption-agency fixture above**; mismatched `</div>`
  ignored; voids; bare-text synthesis; comments placement; foster-parented
  text before table; reconstruction fixture `<b>1<i>2</b>3</i>`;
  `</br>`→br; adjacent text merges into ONE node; doctype captured.
- Gate: `cargo test --workspace` green on win-001, then commit.

### 6. Historical phase-gate reporting requirement

What's implemented vs deferred, demo command (`amazing parse page.html`),
correctness gaps, then ask before Phase 2. This gate was completed; later
phase work is recorded in the sections that follow.

## Phase 1 — DONE (win-001, all tests green)

Shipped: rewritten `tokenizer.rs` (full §13.2.5 state machine, RAWTEXT/RCDATA,
entities incl. numeric + C1 table, EOF-in-tag literal fallback), fixed
`entities.rs` (window 12, uppercase semicolon forms), new `tree.rs`
(§13.2.6: 15 insertion modes, adoption agency + reconstruction + Noah's Ark,
foster parenting, implicit tbody, scope sets), `amazing_html::parse` +
`amazing parse [file|stdin]` demo subcommand, CRLF normalization in parse(),
README.md.

Known gaps (deferred, listed in module docs): full 2 231-entry named-entity
table; template/frameset/select modes; foreign content; foster parenting
ignores reconstructed formatting elements; PUBLIC/SYSTEM ids not captured;
quirks flag approximated. WPT tracking starts Phase 3 per plan.

## Phase 2 — DONE (win-001, all tests green)

Shipped: `amazing-css` (std-only, zero deps) with `syntax.rs` (css-syntax-3 §4
tokenizer + serializer: comments, strings+escapes, numbers/dimensions/%,
functions paired with explicit parens, hash/at-keywords), `parser.rs` (§5:
qualified rules, comma-split preludes, `!important`, spec-style error
recovery, at-rules consumed-and-dropped, inline `style=""` via
`parse_declarations`), `selectors.rs` (css-selectors-4 subset: all four
combinators right-to-left matching, attr ops incl. `i` flag, structural
pseudos with full an+b micro-syntax incl. sign-folding token recovery,
`:root/:empty/:link`, never-match interactive set, `:not/:is/:where` over
compound args, §6.4 specificity), `cascade.rs` (css-cascade-4 §6 full key:
importance → origin → inline → specificity → order; inheritance set; 40
initial values; ~60-rule UA sheet; delta-view for the demo).
Driver: `amazing style page.html [more.css…]` demo subcommand +
`crates/amazing/tests/phase2_style.rs` e2e gate.

Known gaps (deferred, in module docs): no shorthand→longhand expansion;
at-rule bodies not interpreted (`@media` etc. dropped); custom properties,
`var()`, pseudo-elements, namespaces, `:lang()`, `nth- of S` unsupported;
presentational attributes ignored; interactive pseudos never match.
*(Update, Phase 3: margin/padding/border-width/style/color and the full
`border:` shorthand now DO expand into longhands before cascading —
see cascade.rs `expand_shorthand`.)*

## Phase 3 — DONE (win-001, all tests green)

Shipped: `amazing-layout` (std-only) with `values.rs` (lengths px…rem/%,
auto, font-size ladder, line-height forms, shorthand+longhand layering for
margin/padding/border), `box_tree.rs` (§9.2 display classification,
anonymous blocks around inline runs, span flattening at own font size,
block-hoisting, display:none pruning, `<br>` markers), `font.rs` (panic-free
sfnt/TTF metrics parser: head/hhea/maxp/hmtx, cmap format 4 incl.
idRangeOffset cross-segment addressing and format 12 bonus, plus a fallback
metric font so layout works without a real font file), `block.rs`
(§10.3.3 constraint equation with auto-margin centering, auto/specified
heights, sibling max-rule collapse, first/last-child collapse
approximation vs parent margins, padding/border separation),
`inline.rs` (strut-based line boxes, whitespace collapse, greedy wrap,
`<br>` handling, half-leading baselines). Driver: `amazing layout
page.html [width] [extra.css…]` demo subcommand +
`crates/amazing/tests/phase3_layout.rs` curated WPT css/CSS2-subset gate
(17 named cases, prints PASS/FAIL report). Demo: `cargo run -p amazing --
layout examples/layout.html`.

Known gaps (deferred, in module docs): floats/abs-pos none; parent-child
margin collapse approximated inside the parent instead of propagating
through; inline-block flattened as plain inline; min/max-width, box-sizing,
text-align, images ignored; one strut per block (no per-line mixed-font
metrics beyond word widths); percentage heights stay auto; TTF glyph
OUTLINES not parsed (advance/metrics only — paint draws placeholder ink).
WPT css/CSS2 pass-rate tracking = this curated subset until full WPT
harness lands.

## Phase 3.5 (flexbox) — DONE (win-001, all tests green)

Shipped: `amazing-layout/src/flex.rs` — `display:flex`/`inline-flex`,
flex-direction ×4 (column-reverse stacks top-down as documented), wrap,
grow/shrink/basis + full `flex` shorthand grammar (`none`, `auto`, number →
basis 0%, one-value length), order sort, gaps, justify-content ×6,
align-items/self, align-content vs definite height, auto-margin absorption
on the main axis (materialized via `ComputedStyle::set` so the box-model
resolver agrees), cross-axis stretch incl. single-line growth into a
definite container cross size. Flex containers hoist like blocks in
box_tree (they are neither Block nor inline). Deviations: baseline≈start,
no shrink floors, column grow/shrink needs definite height, cross-axis auto
margins = 0. Tests: 13 unit tests in flex.rs through the real pipeline.
Demo: `cargo run -p amazing -- layout examples/layout.html`.

## Phase 3.6 (grid) — DONE (integrated, win-001 green)

Shipped: `amazing-layout/src/grid.rs` — track lists (px/em/%, auto,
min/max-content conflated as max-content, `<number>fr`,
`repeat(int | auto-fill, …)` nested), `grid-auto-columns/rows` cycling onto
implicit tracks, gaps, auto-flow row/column/dense (column = transpose),
placement via grid-row/-column + longhands with negative lines anchored to
the explicit-grid end and §8.5 sparse cursor (+dense backflow), two-phase
track sizing (definite bases → intrinsic contributions → weighted fr split
of leftover), justify/align-content ×6, items/self alignment with stretch
materialization. Integration: `Flow::Grid` variant, Display::Grid
classification, block dispatch; bring-up fixes: auto-placement wrap limit
must count COLUMNS for row flow (transposed for column flow); outer_h +
End/Center alignment no longer double-count padding/border inside
border_box. Deviations: named areas / grid-area / span-to-line-name;
row-track % resolves against width; fr rows need definite height. Tests:
16 in grid.rs through the real pipeline. Demo: gridprobe-style pages via
`cargo run -p amazing -- layout page.html`.

## Phase 4 (paint) — DONE (crate integrated, win-001 green)

Shipped: `amazing-paint` — color parser (hex/named/rgb·rgba legacy+modern
syntax, overflow clamps, hostile-input safe), Surface framebuffer
(fill_rect with NaN/degenerate guards, PPM + BMP encoders incl. correct
14-byte BMP file header + row padding), text layer (GlyphProvider trait +
rect-ink placeholder rasterizer, alpha-composited blit),
paint_document (backgrounds → border strips → line-box words → children,
viewport-clipped, currentcolor resolution). Driver: `amazing render
page.html out.bmp [width] [extra.css…]` writes BMP + PPM twin of the page.
Follow-up (post-phase): real glyph ink — `amazing-layout/font.rs` now decodes
TTF `loca`/`glyf` outlines (simple + composite glyphs, F2Dot14 transforms,
implied quadratic midpoints, depth-capped recursion, hostile-input safe) and
`amazing-paint/text.rs` rasterizes them (curve flattening, 4x4 supersampled
nonzero-winding scanline fill, bearing/advance blit); fallback rect-ink kept
for fonts without outlines. Deviations: glyph ink is placeholder bars until TTF
outlines land; borders
always solid; no radius/shadows/images.

## Phase 5 (net) — DONE (crate integrated, win-001 green)

Shipped: `amazing-net` — Url parser/resolver (http/https/file, IPv6,
percent-decode, dot-segment removal, file↔Windows path), case-insensitive
multi-value Headers, Response (+text/content_type/is_success), HTTP/1.1
GET client (chunked > Content-Length > EOF framing, 32 MiB cap,
bare-\n tolerance), fetch() dispatcher with ≤5 redirect hops and loopback
integration tests (exact request bytes, chunk extensions+trailers, redirect
chains). TLS deliberately unsupported (`https` fails fast; documented).
Driver: `amazing fetch <url|file> [-i]`. Deferred: keep-alive, non-GET,
gzip/br, cookies/caching/auth, HTTP/2.

## Phase 6 (JS) — runtime milestone landed (win-001 green)

Shipped: `amazing-js/src/interp.rs` — panic-free tree-walking execution over
the existing ECMA-262 AST. The runtime has lexical closures, `var`/`let`/
`const`, function and arrow calls, `new`, ordinary objects and prototypes,
arrays with length/index bookkeeping, member assignment, arithmetic/logical/
relational/equality operators, unary/update/delete/typeof, conditionals,
loops, switch, try/catch/finally, throw, `for-in`/`for-of`, templates,
optional chaining, and captured `console.log`. Built-ins include `Object`,
`Array`, `Map`, `Set`, `Promise`, `Date`, `Math.abs/max/min`, `Number`,
`String`, `Boolean`, `Function.call/apply/bind`, `Object.keys/values`, `Object.assign`, `Array.from/of`,
`Number.isNaN/isFinite`, `Math.floor/ceil/round/sqrt/pow/trunc/sign`,
`JSON.parse/stringify`,
`eval`, numeric parsing globals, and array `push`/`pop`/`join`/`slice`/
`concat`/`includes`/`indexOf`/`map`/`filter`/`reduce`/`forEach`, plus common
string utilities. Map and Set use
SameValueZero keys, preserve insertion order, expose `size`, and return
array-backed key/value/entry iterables for this milestone. Promise supports
the constructor executor, `resolve`/`reject`, `then`/`catch`/`finally`,
pending reaction delivery, async-function return wrapping, and a FIFO
microtask queue drained after each top-level script. Promise combinators
`all`/`race`/`allSettled`/`any` preserve input order and settlement kind.
`for-of` and destructuring also consume bounded `next()` iterator objects.
Recursive array/object binding patterns with defaults/rest now work in
declarations, parameters, and `for` declaration heads.
Assignment uses a
single evaluated reference for compound/logical updates; property descriptors,
accessors, extensibility, wrapper objects, named anonymous functions,
completion values, labeled control flow, and per-iteration `for (let …)`
environments, assignment-pattern defaults, and computed binding keys are
covered by the runtime milestone.

Driver: `amazing run [script.js]` (stdin when omitted) executes a classic
script, prints captured console lines, then its completion value. The
interpreter's focused runtime coverage is included in the `amazing-js` suite;
`cargo test --workspace` is green on win-001 (142 tests in `amazing-js`).

Conformance evidence: the official Test262 repository was shallow-sparsely
checked out on win-001 with 641 tests across the selected classic-language
directories. `python scripts/test262.py C:\\Users\\server\\src\\test262
--binary target\\debug\\amazing.exe` measured 196 passed, 0 failed, and
445 skipped; measured pass rate = **196/196 (100%)**. Skips are explicit
for modules/async/negative tests, BigInt/Symbol/private/accessor/destructuring
features, and unavailable harness hooks. This is a focused slice, not whole-suite
coverage.

Known gaps: the browser loop is cooperative and currently lacks network event
sources, and generators, symbols, modules, real Symbol identity, regex
literals, and full class semantics are deferred. Basic
`super` property and constructor calls now work.
The `Symbol.iterator` key bridge, lazy `next()` consumption, and abrupt-body
closing are implemented. The runtime is
a milestone, not Phase 6 exit-criteria completion.

## Phase 7 (DOM/JS bridge) — interactive tracer slice landed (win-001 green)

Shipped: `amazing_js::DomHost` keeps the JS core independent of arena DOM
types while exposing live node wrappers. The driver implements a bounded,
mutation-capable surface: `document.querySelector`/`querySelectorAll`,
`getElementById`/`getElementsByTagName`/`getElementsByClassName`
(tag/id/class selectors), `document.body`,
`document.createElement`, element `textContent`, `id`/`className`,
`document.createTextNode`,
`getAttribute`, `setAttribute`, `append`/`appendChild`/`remove`,
`hasAttribute`/`removeAttribute`,
`matches`/`closest`,
`innerHTML`/`innerText`, `document.title`, the `window`/`globalThis` aliases, and
`element.dataset`, `element.classList`, and `element.style` inline properties,
viewport globals (`innerWidth`, `innerHeight`, `devicePixelRatio`),
`onclick`, `addEventListener`, `removeEventListener`, and `click()` event
dispatch. Detached nodes are reparented through the arena's existing
attach/detach API; callbacks remain JS values owned by the interpreter.

Driver: `amazing domrun <page.html> <script.js>` executes a classic script,
prints captured console output and the completion value, then dumps the
mutated DOM. The end-to-end fixture `examples/phase7.html` +
`examples/phase7.js` was run on win-001: `before` became `after`, an
attribute was written, a click handler changed text and inline color, a
listener wrote a second attribute, and a new `<span>` child was appended.
Full `cargo test --workspace` remains green on win-001.

Known gaps: selector coverage follows the CSS engine's supported subset;
script execution is
classic/document-order rather than full HTML
parser scheduling; and the browser global/window surface remains a bounded
subset of the web platform.

## Phase 8 (GUI shell) — Windows native first slice landed (win-001 green)

Shipped: `crates/amazing/src/gui.rs` isolates the only `unsafe` code in a
hand-written Win32 FFI module. `amazing gui [url]` creates a native top-level
window with a tab strip, address bar, Go, Back, Forward, and new-tab controls.
Each tab owns its own history, rendered surface, and active page; the mouse
wheel scrolls the painted viewport. Navigation uses the existing
`amazing-net` fetcher, then runs the existing HTML → CSS → layout → software
paint pipeline and blits the RGBA surface through a top-down GDI bitmap.
Local files and plain `http://` URLs use the same fetch path as the headless
driver; unsupported TLS remains the documented Phase 5 exception.

The shell compiles and the complete workspace suite passes on win-001. Live
Windows App/RDP desktop runs loaded both `examples/phase8.html` and the
no-argument default `examples/sample.html` in the release executable. The
clean, bounded no-argument-launch capture is preserved at
`docs/screenshots/phase8-win001.png`. It visibly proves the Ferric top-level
window, tab/address controls, Go/Back/Forward/new-tab controls, the default
local URL, and the colored software-painted page. The
GUI click smoke test also asserts the live DOM mutation, white foreground,
red repainted background pixel, event bubbling, timers, and fetch results.
On Windows, launching `amazing.exe` with no arguments now opens the GUI, so
the release artifact is directly double-clickable; `amazing version` retains
the pipeline-status output.

Inline page scripts execute against the live DOM before the first layout
and repaint. Each tab retains its JS runtime; layout-box hit testing dispatches
mouse clicks and repaints DOM/style mutations. Limitations are blocking
navigation during fetch/paint, no native scrollbar thumb, no network event
sources, and no module/async script scheduling. Classic
external `script[src]` resources now resolve relative to the page URL/path and
load through the existing fetcher; `setTimeout`, `setInterval`,
`requestAnimationFrame`, and Promise microtasks use the cooperative GUI timer
pump. `document.addEventListener("DOMContentLoaded", …)`
is dispatched after classic scripts complete; DOM events bubble through arena
parents and expose `currentTarget`, `stopPropagation`, and `preventDefault`;
`DOMContentLoaded` and `load` dispatch on `document`/`window`, with a body
load dispatch, after classic scripts and author stylesheets are fetched.
Classic inline `on*="…"` attributes install into the same bubbling event
registry as `addEventListener`.
`window.location.href`/`assign`/`replace` requests resolve relative to the
active tab and enter its history. GUI anchor clicks perform the same default
navigation unless a handler prevents the default. Page `fetch()` is host
mediated and returns a minimal Promise-backed Response (`ok`, `status`, `url`,
`headers.get/has`, `text()`/`json()`); Promise resolution now adopts returned
Promises for normal chains.
`history.back`/`forward`/`go` and no-reload `pushState`/`replaceState` update
the active tab’s application history.
External `<link rel="stylesheet" href="…">` resources are fetched and merged
into the author cascade before layout.

Release artifact evidence: `cargo build --release` on win-001 produced
`target/release/amazing.exe` (1,701,376 bytes); its `version` command reports
the GUI subcommand and its release `render` command produced a 640×54 raster.
Phase 8 runnable demo: double-click `target\release\amazing.exe`, or run
`target\release\amazing.exe gui examples\phase8.html` to open the captured
fixture directly.

## Phase 9.1 (media) — decoded image vertical slice

Shipped: the std-only `amazing-media` crate detects and validates binary PPM
(P6, max values 1–255) and uncompressed 24-bit BMP, normalizing both into a
top-left RGBA8 `Image`. The driver resolves relative `<img src>` resources
through `amazing-net`; `PageMedia` is the concrete adapter at both the layout
intrinsic-size seam and the paint pixel seam. Inline images participate in
word order, wrapping, baseline placement, and line-height growth. Block/flex/
grid image boxes use intrinsic dimensions, and HTML `width`/`height`
attributes scale while preserving aspect ratio when only one is supplied.
Paint performs clipped nearest-neighbor scaling and source-over RGBA.

Runnable artifact on win-001:
`target\debug\amazing.exe render examples\phase9-media.html phase9_media.bmp 400`.
It produced a measured 400×355 BMP/PPM pair; the visually inspected render is
preserved at `docs/screenshots/phase9-media.png`. The integration test invokes
the real `render` command in a temporary directory and proves a relative PPM's
red/green pixels survive fetch → decode → inline layout → paint → BMP encode →
decode.

Known Phase 9.1 image gaps: PNG/JPEG/GIF/WebP, animation, and
`<picture>`/`srcset` remain separate follow-on slices rather than hidden
exceptions. Audio begins in Phase 9.2; video, WebRTC, and acceleration remain
later Phase 9 sub-projects.

Conformance baseline for this slice: curated WPT CSS2 remains 17/17 and
flexbox/grid remains 22/22; the focused Test262 slice remains 196/196
supported tests (445 unsupported/skipped). No dedicated media WPT subset is
imported yet; the Phase 9 render integration gate is 1/1.

Final win-001 gate: all 416 workspace tests/doctests passed and
`cargo build --release` produced `target/release/amazing.exe` at 1,719,296
bytes. The release demo emitted a 426,054-byte BMP and 426,015-byte PPM.

## Phase 9.2 (media) — PCM WAV audio foundation

Shipped: `amazing-media::decode_wav` parses bounded RIFF/WAVE chunks in any
order, honors odd-byte chunk padding, validates PCM format consistency, and
normalizes unsigned 8-bit or signed 16-bit mono/multichannel streams into
interleaved signed `i16` samples. `amazing audio-info <file.wav>` reports the
decoded sample rate, channels, frame count, duration, and peak. The committed
`examples/phase9-audio.wav` fixture is 0.25 seconds of mono PCM16 at 8 kHz;
the command measured 2,000 frames and peak 4,095.

The Windows-only `play-audio` adapter validates through the core decoder and
then calls synchronous WinMM `PlaySoundW` at the existing platform/unsafe
seam. An interactive-session scheduled-task run on win-001 completed with
`LastTaskResult = 0`; the temporary task was unregistered immediately.
Automated coverage includes PCM16 decode, PCM8 normalization, and a real
binary-process `audio-info` integration test.

Final win-001 gate: all 419 workspace tests/doctests passed; the release
binary is 1,725,440 bytes and the committed WAV fixture is 4,078 bytes.
The existing WPT/Test262 baselines remain 17/17 CSS2, 22/22 flex/grid, and
196/196 supported Test262 tests; no dedicated audio WPT subset is imported.

Known gaps: float/24/32-bit PCM, compressed WAV codecs, resampling/mixing,
streaming, an HTML `<audio>` element/event model, and playback timing remain
follow-on media work. The Phase 9.1 image, WebRTC, and hardware-acceleration
gaps remain unchanged.

Recommended next sub-phase: Phase 9.3 WebRTC transport foundations (STUN and
RTP parsing plus a UDP loopback artifact), followed by Windows hardware-
accelerated presentation. The user's explicit “do all phases now” instruction
pre-authorizes continuing past this sub-phase gate.

## Phase 9.3 (WebRTC) — STUN/RTP UDP transport foundation

Shipped: the new safe, std-only `amazing-webrtc` crate encodes and parses STUN
Binding requests/success responses with RFC magic-cookie and 96-bit
transaction validation, including IPv4 XOR-MAPPED-ADDRESS recovery. It also
encodes and parses the RTP v2 fixed header (marker, payload type, sequence,
timestamp, SSRC) and owned payloads.

Runnable artifact: `target\release\amazing.exe webrtc-loopback`. A real pair
of `std::net::UdpSocket`s performs a STUN exchange, verifies the discovered
client endpoint and transaction, then sends an RTP packet with sequence 7,
timestamp 960, and a 15-byte payload. The server decodes and echoes it; the
client and server both compare the full packet. The binary-process integration
gate is 1/1 and uses a cooperative single-threaded exchange plus two-second
socket deadlines to fail rather than hang.

Known gaps: ICE candidate gathering/checklists, TURN, STUN authentication,
MESSAGE-INTEGRITY/FINGERPRINT, IPv6 mapped addresses, RTP CSRC/extensions/
padding, RTCP, SDP, DTLS-SRTP, SCTP/data channels, jitter buffering,
congestion control, and real audio/video codec flow remain separate WebRTC
sub-phases. This loopback is transport evidence, not a claim of browser-
compatible WebRTC.

Conformance: no dedicated WebRTC WPT subset is imported yet. Existing
baselines remain 17/17 CSS2, 22/22 flex/grid, and 196/196 supported Test262.

Final win-001 gate: all 423 workspace tests/doctests passed, the release
binary is 1,744,896 bytes, and the release loopback artifact completed with
one successful STUN/RTP exchange (integration gate 1/1).

Recommended next sub-phase: Phase 9.4 Windows hardware-accelerated surface
presentation with a software fallback and pixel-equivalence gate. The user's
explicit “do all phases now” instruction pre-authorizes continuing.

## Phase 9.4 (hardware acceleration) — D3D11 capability foundation

Shipped: `crates/amazing/src/gpu.rs` isolates a hand-written D3D11/COM FFI
adapter behind `gpu::probe`. It requests only `D3D_DRIVER_TYPE_HARDWARE`,
negotiates feature levels 11.1 through 10.0 (with the documented 11.1
`E_INVALIDARG` retry), requires non-null device/context interfaces, and
releases each owned COM reference exactly once. Every unsafe operation states
its pointer/ownership invariants and why `std` cannot express Direct3D/COM.

Runnable artifact: `target\release\amazing.exe gpu-info`. On win-001 the
integration gate created a real **D3D11 hardware device at feature level
11.1**. No WARP/software device is accepted, so a passing command is direct
hardware evidence rather than a fallback disguised as acceleration.

Known gaps: the software RGBA compositor is not yet uploaded to a D3D11
texture or presented through a DXGI swap chain; resize/device-loss handling,
frame pacing, dirty rectangles, GPU rasterization, and GDI fallback selection
are therefore still pending. This sub-phase proves capability, not completed
hardware presentation.

Conformance: the Windows GPU integration gate is 1/1. Existing baselines
remain 17/17 CSS2, 22/22 flex/grid, and 196/196 supported Test262; no GPU WPT
subset is imported yet.

Final win-001 gate: all 423 hardware-independent workspace tests/doctests
passed, plus the explicitly invoked hardware integration gate passed 1/1.
The release binary is 1,745,920 bytes; release `gpu-info` again negotiated
D3D11 hardware feature level 11.1. The GPU test is ignored in the ordinary
suite so generic Windows CI runners without an exposed adapter stay portable.

Recommended next sub-phase: Phase 9.5 DXGI swap-chain presentation of the
existing software surface, retaining the current GDI path as a fallback and
adding pixel/dimension equivalence evidence. The user's explicit “do all
phases now” instruction pre-authorizes continuing.

## Phase 9.5 (hardware acceleration) — D3D11 surface presentation

Shipped: the Windows GPU adapter now creates a hardware D3D11 device plus
DXGI swap chain, obtains the back-buffer texture, uploads tightly packed BGRA
frames through `ID3D11DeviceContext::UpdateSubresource`, handles buffer
resizes, calls `IDXGISwapChain::Present`, and releases every COM reference in
dependency order. COM method indices and ABI structures were verified against
the installed Windows 10 SDK headers. The headless SSH session correctly
reports DXGI presentation unavailable; an interactive-session run of
`gpu-present-smoke` completed with `GPU_PRESENT_RESULT=0`.

Ferric's real `WM_PAINT` path now composes the software RGBA surface into a
full-client BGRA frame (including scroll offset and white control region),
presents it through D3D11 when available, and permanently falls back to the
existing GDI blit if creation or presentation fails. Window titles expose the
active backend as `[D3D11]` or `[GDI]`. A deterministic unit gate proves the
GPU frame matches GDI scroll semantics and RGBA→BGRA channel order.

Final win-001 code gate: all 424 hardware-independent workspace
tests/doctests passed; two hardware tests remain explicitly ignored in normal
CI and are invoked only in a real desktop/hardware environment. The release
binary is 1,756,160 bytes and `gpu-info` still reports feature level 11.1.

Conformance baseline remains 17/17 curated CSS2, 22/22 curated flex/grid, and
196/196 supported Test262 tests. No GPU-specific WPT subset is imported yet;
the hardware capability and presentation gates are each 1/1 when explicitly
run in an interactive win-001 session.

Known gaps: the GPU presents the existing CPU-rasterized surface; style,
layout, text, and paint commands are not GPU-native. Device-loss recreation,
dirty rectangles, adaptive frame pacing/tearing, HDR/color management, and
multi-window resource sharing remain pending. A final `[D3D11]` desktop
screenshot is pending the user's required acceptance of Windows App's RDP
certificate warning; the hidden swap-chain artifact is already verified.

Recommended next sub-phase: Phase 9.6 deepen WebRTC with ICE checklist/state,
authenticated STUN, RTCP, and DTLS-SRTP foundations. The user's explicit “do
all phases now” instruction pre-authorizes continuing.

## Phase 9.6 (WebRTC) — authenticated ICE connectivity checks

Shipped: the safe `amazing-webrtc` core now implements SHA-1, HMAC-SHA1, and
IEEE CRC32 internally for STUN MESSAGE-INTEGRITY and FINGERPRINT. Verification
passes RFC 5769's published sample-request bytes exactly and uses a
no-early-exit best-effort digest comparison (ordinary Rust optimization does
not provide a formal constant-time guarantee). Authenticated Binding requests encode
USERNAME, PRIORITY, ICE-CONTROLLING, optional USE-CANDIDATE, integrity, and
fingerprint; authenticated success responses retain XOR-MAPPED-ADDRESS.
Tampering is rejected by both integrity and fingerprint gates.

`IceChecklist` forms same-component candidate pairs, applies RFC 8445's pair
priority formula, sorts highest-first, and exposes Frozen/Waiting/In-Progress/
Succeeded/Failed transitions plus nomination-driven Running/Completed/Failed
checklist state. The upgraded `webrtc-loopback` command performs authenticated
request and response checks, validates the mapped endpoint, nominates the pair
to `Completed`, then permits the existing RTP echo. The binary integration
output now includes `ICE state=Completed integrity=ok` and remains 1/1.

Known gaps: this is one-component IPv4 host-candidate ICE. Foundation-based
unfreezing across multiple checklists, triggered-check queues, retransmission
timers, role conflict, peer/server-reflexive candidate insertion, TURN, IPv6,
credential signaling/restarts, RTCP, SDP, DTLS-SRTP, SCTP, jitter/congestion,
and codecs remain pending. The in-crate SHA-1/HMAC is protocol compatibility
work, not a general-purpose cryptographic library.

Conformance: RFC 5769 vector gate 1/1 and authenticated UDP integration gate
1/1. Existing baselines remain 17/17 CSS2, 22/22 flex/grid, and 196/196
supported Test262; no dedicated WebRTC WPT subset is imported yet.

Final win-001 gate: all 429 hardware-independent workspace tests/doctests
passed and the release binary is 1,778,688 bytes. The release loopback again
reported `ICE state=Completed integrity=ok` before RTP delivery.

Recommended next sub-phase: Phase 9.7 RTCP packet/report handling and minimal
SDP/ICE credential signaling, followed by DTLS-SRTP. The user's explicit “do
all phases now” instruction pre-authorizes continuing.

## Phase 9.7 (WebRTC) — RTCP sender timing/report foundation

Shipped: `RtcpSenderReport` encodes and parses RFC 3550 RTCP v2 Sender Reports
with zero reception blocks: SSRC, 64-bit NTP timestamp, RTP timestamp, packet
count, and octet count. Parsing enforces version 2, no first-packet padding,
packet type 200, RC=0, 32-bit word-minus-one length 6, and the exact 28-byte
sender-info shape; unsupported report blocks are rejected explicitly.

The release `webrtc-loopback` artifact now sends an RTCP report after the
authenticated ICE check and RTP echo. Both endpoints decode and compare the
report, proving the RTP timestamp/SSRC and one-packet/15-octet counters survive
the UDP wire. The integration output includes `RTCP packets=1 octets=15`.

Known gaps: Receiver Reports, reception report blocks, SDES/CNAME compound
packets, BYE/APP, reduced-size RTCP, feedback/NACK/PLI/TWCC, scheduling and
bandwidth fractions, RTT/jitter calculations, and wall-clock NTP generation
remain pending. This is the sender-info wire foundation, not full RTCP.

Conformance: RFC 3550 Sender Report unit gate 1/1 and the extended UDP
integration gate 1/1. Existing baselines remain 17/17 CSS2, 22/22 flex/grid,
and 196/196 supported Test262; no WebRTC WPT subset is imported yet.

Final win-001 gate: all 430 hardware-independent workspace tests/doctests
passed and the release binary is 1,782,784 bytes. The release loopback output
included `RTCP packets=1 octets=15` after authenticated ICE and RTP.

Recommended next sub-phase: minimal SDP offer/answer parsing for ICE
credentials, candidates, DTLS fingerprint/setup, and RTP payload mapping,
followed by a vetted DTLS-SRTP infrastructure decision. The user's explicit
“do all phases now” instruction pre-authorizes continuing.

## Phase 9.8 (WebRTC signaling) — minimal ICE/DTLS/RTP SDP

Shipped: `SdpDescription` and `RtpMap` provide a strict single-media SDP
model. The parser accepts session defaults with media-level overrides for
RFC 8839 ICE ufrag/password, UDP/IPv4 host candidates, RFC 8122 SHA-256
certificate fingerprints/setup roles, `UDP/TLS/RTP/SAVPF` payload IDs, and
`rtpmap` clock/channel mappings. It retains and validates the required ordered
`v=`, `o=`, `s=`, `t=` session skeleton and media `c=` address rather than
inventing them on output. Incoming ufrags accept RFC's 4–256 range while the
serializer enforces the RFC 8839 outbound maximum of 32; password, candidate
foundation/component, uppercase 32-byte SHA-256, seven-bit RTP payload, and
mapping-membership bounds are also enforced. Unsupported transports,
candidate forms, fingerprint algorithms, and multiple media sections fail
explicitly.

The serializer emits deterministic RFC 8866 ordering and reparses its own
output, refusing a model that cannot be represented without semantic loss.
Runnable artifact: `target\release\amazing.exe sdp-roundtrip
examples\phase9-offer.sdp`. The committed offer carries one audio payload,
one host candidate, ICE credentials, DTLS fingerprint/setup, and Opus mapping;
the binary integration gate reports `roundtrip ok` (1/1).

Known gaps: multi-media/BUNDLE/MID, media-level override retention in the
serialized model, answers/rejected media, IPv6/reflexive/relay candidates,
Trickle ICE/end-of-candidates, RTCP mux/rsize, extmap/fmtp/rtcp-fb, simulcast/
RID, codec negotiation, ICE restarts, certificate generation/verification,
and actual DTLS-SRTP remain pending. This parser is a bounded signaling seam,
not a general SDP implementation.

Conformance: SDP semantic round-trip and unsupported-profile gates pass 2/2;
the binary signaling gate passes 1/1. Existing baselines remain 17/17 CSS2,
22/22 flex/grid, and 196/196 supported Test262; no WebRTC WPT subset is
imported yet.

Final win-001 gate: all 433 hardware-independent workspace tests/doctests
passed; the release binary is 1,820,160 bytes and the committed SDP offer is
371 bytes. The release signaling artifact reported `roundtrip ok`.

Recommended next sub-phase: document and implement the vetted DTLS-SRTP
infrastructure boundary rather than hand-rolling security-critical TLS, then
bind negotiated keys to RTP/RTCP. The user's explicit “do all phases now”
instruction pre-authorizes continuing.

## Phase 9.9 (secure media) — vetted DTLS-SRTP adapter

Shipped: the new `amazing-webrtc-crypto` crate is the sole third-party
security-provider exception. It pins WebRTC.rs DTLS 0.12.0 and SRTP 0.17.2,
uses a current-thread cooperative Tokio runtime, forbids first-party unsafe,
and keeps every engine core including `amazing-webrtc` safe and `std`-only.
The application now commits `Cargo.lock`, so the reviewed crypto dependency
graph and checksums are reproducible.

The adapter generates ephemeral P-256 certificates, performs a mutual DTLS
1.2 handshake with `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`, negotiates
`AEAD_AES_128_GCM` through RFC 5764 `use_srtp`, and verifies both peer
certificate SHA-256 fingerprints before exporting any media keys. It exports
56 bytes with `EXTRACTOR-dtls_srtp`, applies RFC 5764's client/server key and
salt ordering, zeroizes the temporary exported buffers, then delegates RTP
and RTCP protection to the vetted SRTP provider with 64-packet replay windows.

Runnable artifact: `target\release\amazing.exe secure-media-loopback`. The
command protects and recovers Amazing's real 15-byte RTP payload in both
directions. The server uses its own SSRC and counters before sending its RTCP
Sender Report, so the opposite-direction SRTCP proof is semantically valid.
The public integration gate proves that altered and replayed SRTP and SRTCP
packets are all rejected. Observed debug wire sizes are 43 bytes for SRTP and
48 bytes for SRTCP.

Known gaps: this milestone uses an in-memory datagram transport, not the
ICE-nominated UDP sockets from `webrtc-loopback`; generated fingerprints are
pinned directly rather than exchanged through the Phase 9.8 SDP model; DTLS
client/server roles are fixed rather than derived from SDP `setup`; there is
no persistent identity, certificate rotation, rekey/overlap, key-lifetime
enforcement, multiple SSRC association table, packet demultiplexer, remote
interop gate, or WebRTC WPT import. TURN, full ICE, media negotiation/codecs,
congestion/jitter, and SCTP/data channels also remain pending. HTTPS still
needs a separate vetted TLS adapter.

Conformance: the secure command integration gate passes 1/1. The existing
baselines remain 17/17 CSS2, 22/22 flex/grid, and 196/196 supported Test262;
no WebRTC WPT subset is imported yet.

Final win-001 gate: all 434 hardware-independent workspace tests/doctests
passed with `--locked`; the release command passed and the binary is 4,191,232
bytes. The increase from Phase 9.8 is the isolated vetted crypto provider and
its exact dependency closure.

Recommended next sub-phase: expose a datagram-session seam from the secure
adapter and replace the plaintext RTP/RTCP portion of `webrtc-loopback` with
SRTP/SRTCP over its nominated UDP pair, using the parsed SDP fingerprint as
the peer identity. The user's explicit “do all phases now” instruction
pre-authorizes continuing.

## Phase 9.10 (secure transport) — ICE-nominated UDP + SDP identity

Shipped: internal `SecureIdentity` and `SecureMediaSession` seams isolate
certificate/key and secure-packet behavior. The former exposes only the
signaled SHA-256 fingerprint inside the adapter; the latter keeps provider
contexts behind RTP/RTCP protect/unprotect operations with directional keys
and 64-packet replay windows. These seams are intentionally not public until a
single-endpoint handshake constructor exists. WebRTC.rs async/provider types
do not leak into engine cores or the driver.

The upgraded `secure-media-loopback` binds and connects one real IPv4 UDP
socket per endpoint and serializes/reparses SDP containing the actual
ephemeral certificate fingerprints, ICE credentials, host candidates, and
`actpass`/`passive` setup roles. The controlling checklist is formed from the
parsed candidates; STUN USERNAME, PRIORITY, MESSAGE-INTEGRITY key, and mapped
endpoint checks come from those parsed descriptions. The parsed answer makes
the offerer `active` (DTLS client) and answerer `passive` (DTLS server); those
roles and parsed peer fingerprints drive the mutual DTLS 1.2 handshake.

An internal RFC 7983-style datagram demultiplexer owns the sole socket reader
for each endpoint. It routes DTLS records and SRTP/SRTCP datagrams to separate
bounded channels with nonblocking UDP-loss behavior, so a full queue for one
protocol cannot starve the other. The DTLS association stays live while
Amazing's real RTP and RTCP packets cross the same ICE-nominated UDP
five-tuple in both directions. The wire gate still rejects altered and
replayed SRTP and SRTCP, and the server's Sender Report follows its own RTP
SSRC/counters.

Runnable artifact: `target\release\amazing.exe secure-media-loopback`. New
observable evidence includes `ICE state=Completed credentials=SDP`, both
loopback UDP ports, and `SDP identity=ok roles=active/passive` before the
existing DTLS/SRTP, fingerprint, AEAD, replay, RTP, and SRTCP evidence.

Known gaps: the two peers still run in one process and construction of a
single remote-facing `SecureMediaSession` remains internal to the proof.
There is no two-process/browser interop gate, ICE consent freshness,
keepalives/retransmission/role conflict, pre-handshake unified STUN/TURN demux,
TURN/IPv6, certificate persistence/rotation, DTLS rekey overlap, packet/key
lifetime enforcement, multi-SSRC association table, codec media pipeline,
congestion/jitter, SCTP/data channels, or WebRTC WPT import.

Conformance: the secure nominated-UDP integration gate passes 1/1. Existing
baselines remain 17/17 CSS2, 22/22 flex/grid, and 196/196 supported Test262;
no WebRTC WPT subset is imported yet.

Final win-001 gate: all 434 hardware-independent workspace tests/doctests
passed with `--locked`; the exact CI build and release command passed, and the
release binary is 4,311,040 bytes with the repository's stripped ThinLTO
release profile.

Recommended next sub-phase: extract a single-endpoint handshake constructor,
run the secure association across two processes for an interop-shaped gate,
then add ICE consent freshness and RTCP compound receiver reporting. The
user's explicit “do all phases now” instruction pre-authorizes continuing.

## Phase 9.11 (secure endpoint) — caller seam + two-process gate

Shipped: `SecureIdentity` is now a usable public opaque identity and
`SecureEndpointConfig`/`SecureUdpEndpoint::establish` form the caller-facing
secure transport interface. The caller supplies a connected std UDP socket,
parsed local/remote SDP, offerer status, and controlling/controlled ICE role.
The adapter validates that identity fingerprint and both candidates match
those inputs, then owns its current-thread runtime, authenticated ICE role,
live DTLS association, RFC 7983 demux, and directional SRTP/SRTCP state.
Provider certificate, key, async, and cipher-context types stay private.

Runnable artifact: `target\release\amazing.exe secure-media-processes`. The
parent spawns a second Ferric process through a hidden peer mode. A
timeout-bounded loopback TCP control channel carries canonical hex-framed SDP:
the child generates an `actpass` offer and the parent returns an `active`
answer. SDP intentionally contains ICE credentials, but certificate private
keys, DTLS/exporter secrets, and SRTP keys never enter signaling. Each process
owns a distinct certificate, runtime, endpoint, and UDP socket. The parent
controls ICE and acts as DTLS client; the child responds as controlled DTLS
server using the same parsed credentials, candidates, roles, and fingerprints
as the public endpoint interface.

After the cross-process handshake, the parent sends one protected RTP packet
to the child. The child validates it, returns its own protected RTP packet and
a matching SRTCP Sender Report, then both endpoints close DTLS cleanly. The
parent validates the response SSRC/counters and child exit status. Observable
output reports `processes=2`, `signaling=tcp-loopback`, `ICE credentials=SDP`, both
RTP directions, SRTCP, and `two-process interop ok`. A child-process guard
kills and reaps the exact child on every parent error path; nonblocking accept,
control-stream timeouts, a ten-second DTLS handshake timeout, per-datagram
timeouts, and bounded `try_wait` completion prevent indefinite waits.

Known gaps: this is process-to-process on one Windows host, not external
browser interoperability; loopback TCP signaling is an artifact protocol, not
a web signaling service. ICE still has one IPv4 host pair and no retransmission,
consent freshness, keepalive, role conflict, peer-reflexive insertion, TURN,
or IPv6. There is no DTLS rekey overlap/key lifetime enforcement, compound
RTCP/Receiver Report/SDES, remote codec media, multi-SSRC routing,
congestion/jitter, SCTP/data channels, or WebRTC WPT import.

Conformance: the public-endpoint two-process integration gate passes 1/1.
Existing baselines remain 17/17 CSS2, 22/22 flex/grid, and 196/196 supported
Test262; no WebRTC WPT subset is imported yet.

Final win-001 gate: all 435 hardware-independent workspace tests/doctests
passed with `--locked`; the exact CI build and stripped ThinLTO release command
passed, the two-process artifact passed, and the binary is 4,505,088 bytes.

Recommended next sub-phase: add periodic ICE consent freshness with timeout
failure, then RTCP Receiver Reports plus mandatory SDES/CNAME compound packet
handling. The user's explicit “do all phases now” instruction pre-authorizes
continuing.

## Later phases (unchanged from goal prompt)

- **3 Layout** block+inline FC, box model, TTF glyph metrics parser.
  WPT css/CSS2 tracking starts here.
- **3.5 Flexbox/Grid** · **4 Paint** software rasterizer + PPM writer ·
  **5 Net** HTTP/1.1 (TLS exception decision due) · **6 JS** Test262-gated ·
  **7 DOM/JS bridge** · **8 GUI shell** (one OS first) · **9 stretch media**
  (images/audio, authenticated ICE/RTP/RTCP, SDP, and D3D11 presentation
  milestones landed; consent freshness and compound RTCP next).

## Server cheat sheet

```
# sync (from mac)
scp -q -r -i ~/.ssh/win001-build-key <local paths> 'server@win-001:C:/Users/server/src/amazing/'

# build/test/commit (on server)
ssh -i ~/.ssh/win001-build-key -o BatchMode=yes server@win-001 ^
  "cd /d C:\Users\server\src\amazing && cargo test --workspace --locked"
git -C C:\Users\server\src\amazing add -A
git -C C:\Users\server\src\amazing -c user.name=server -c user.email=server@win-001 commit -m "..."
```

Known server env: cargo 1.97.1, VS2022 BuildTools, Python312 + LLVM installed,
user env vars LIBCLANG_PATH/PYTHON3 already set. Default ssh shell is cmd.exe
(quote-wrap carefully; PowerShell -EncodedCommand for anything hairy).
