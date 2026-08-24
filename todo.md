# todo.md — Amazing engine handoff

> Handoff note. Read this top-to-bottom before touching code.
> Authoritative docs: `ARCHITECTURE.md` (locked decisions), `AGENTS.md`
> (build policy: **everything builds/tests on win-001**, never macOS).

## Where the project stands

| Phase | Status |
|-------|--------|
| 0 — skeleton | ✅ done, all exit criteria met on win-001 |
| 1 — HTML parser | ✅ milestone exit criteria met; expanded html5lib/WPT tree subset 243/243, broader conformance gaps tracked |
| 2 — CSS | ✅ done on win-001 (88 workspace tests green) — see Phase 2 section |
| 3 — Layout | ✅ done on win-001 (workspace green; curated css/CSS2 subset 17/17) |
| 3.5 — Flexbox | ✅ done on win-001 (13 flex tests; row/column/wrap/grow/shrink/justify/align/gaps/auto-margins/order) |
| 3.6 — Grid | ✅ done on win-001 (16 grid tests; track lists incl. fr/%/repeat+auto-fill, row/column/dense placement, line numbers + negative anchors, implicit tracks, alignment) |
| 4 — Paint | ✅ crate integrated, workspace green (43 paint tests); `render` demo writes BMP+PPM |
| 5 — Net | ✅ HTTP + verified HTTPS integrated (62 core tests + 2 TLS-adapter tests); `fetch` demo |
| 6 — JS | ✅ tree-walking milestone exit criteria met (143 crate tests); focused Test262 slice 196/196 supported (100%) |
| 7 — DOM/JS | ✅ live DOM exit criteria met: handlers mutate layout/style and repaint visibly end-to-end |
| 8 — GUI | ✅ Windows native shell and interactive desktop exit criteria verified on win-001 |
| 9 — Media | ✅ 47-subphase stretch milestone includes live Chromium DTLS-SRTP video |

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
table; template modes; namespaced foreign attributes remain prefix-encoded strings;
PUBLIC/SYSTEM ids not captured; quirks flag approximated. WPT tracking starts
Phase 3 per plan.

Actual upstream conformance (completion audit): at WPT revision
`53bc812da2ed81179ebd1ba76b2d197cc4747d12`, `scripts/html5lib.py` runs the
migrated html5lib tree files `blocks.dat`, `comments01.dat`, `tables01.dat`,
`adoption01.dat`, `tests1.dat`, `svg.dat`, `math.dat`, and
`namespace-sensitivity.dat`, and `tests11.dat` through the public release
artifacts. It discovered 243 cases: 243 passed, 0 failed, 0 skipped; measured pass rate **243/243
(100.00%)**. Declared ordinary and table-family fragment contexts run through
the public `parse-fragment` artifact. This runner compares the DOM tree
minus doctype, because the public dump stores doctype separately. The audit fixed EOF comment token
order, a bare `<table><th>` implicit-row recursion/stack-overflow bug,
namespace-aware SVG/MathML integration, table-adjusted adoption placement,
the adoption outer-loop stack direction, and cell-marker cleanup. This is a
declared complete-document tree subset, not full parser conformance.

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

Actual WPT tracking now also runs through `scripts/wpt.py` at revision
`53bc812da2ed81179ebd1ba76b2d197cc4747d12`: ten named reftests spanning four
CSS2, four flexbox, and two grid files are rendered with Amazing and compared
byte-for-byte to their declared `rel=match` references. Result: **3/10
(30.00%)**, 7 named mismatches, 0 skipped. The older 17/17 and 22/22 numbers
remain useful curated diagnostics but are no longer presented as actual WPT.

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
for fonts without outlines. Deviations: pages without an available TTF use
placeholder bars; borders are always solid; no radius/shadows/images.

Completion-audit visual gate: the win-001 release rendered
`examples/layout.html` at 800×804 to BMP+PPM. Edge rendered the same fixture
and viewport independently on win-001; committed Amazing, Edge,
absolute-diff, and side-by-side PNGs live under `docs/screenshots/phase4-*`.
Normalized pixel RMSE is 0.460741. The geometry comparison visibly preserves
the centered card and overall block flow, while the high pixel delta records
the known fallback bar glyphs, subset layout, and UTF-8-vs-browser-default
encoding difference rather than pretending at pixel parity.

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

Completion-audit integration artifact: `amazing fetch-parse <http-url>` feeds
successful response text directly into `amazing_html::parse` and dumps the
DOM. Its bounded loopback integration gate passes, and the win-001 release
fetched `http://example.com/` into a 652-byte DOM dump.

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
`cargo test --workspace` is green on win-001 (143 tests in `amazing-js`).

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
closing are implemented. The runtime meets the Phase 6 tree-walking/Test262
milestone exit criteria; it is not a claim of full ECMA-262 completion.

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

## Phase 9.12 (session maintenance) — consent + compound RTCP

Shipped: std-only `ConsentFreshness` models RFC 7675 on a caller-supplied
monotonic millisecond clock anchored immediately after successful ICE, before
DTLS setup. Refresh checks can be recorded only when due in the required
randomized 4–6 second window, while 30 seconds without a matching authenticated
response expires consent and clears all outstanding transactions. Requests are
one-shot, up to eight may be outstanding, authenticated responses to any older
outstanding request refresh consent, every transaction ID remains unique for
the session, and the history is capped at 65,536 IDs with a required ICE
restart rather than reuse or unbounded memory growth.

The secure adapter now routes STUN through the live RFC 7983 demux alongside
DTLS and SRTP/SRTCP. `SecureUdpEndpoint::refresh_consent` waits until due,
generates a 96-bit transaction ID and next interval with ring's system RNG,
authenticates once with the parsed remote ICE password, and accepts only a
matching authenticated response. Once consent expires, endpoint RTP and RTCP
sends fail until a new ICE session; late responses cannot restore it. The
controlled peer validates parsed
USERNAME/PRIORITY/role semantics and answers from its local SDP password. The
two-process artifact performs one real refresh and separately gates exact
30-second expiry, reporting `consent=Fresh expiry=detected`.

The std-only RTCP module now supports one-block Receiver Reports and bounded
RFC 3550 compound Sender/Receiver shapes: SR or RR first, then one-chunk SDES
with mandatory CNAME. Signed 24-bit cumulative loss, sequence/jitter/LSR/DLSR
fields, packet lengths, SSRC agreement, CNAME UTF-8, END item, and zero padding
are validated. In the two-process artifact, the child sends compound
SR+`child@ferric`; the parent validates it and reports the child's received RTP
from a receiver-only SSRC. The child decrypts and validates source SSRC,
highest sequence, LSR correlation to the Sender Report's middle NTP 32 bits,
and `parent@ferric` CNAME. Output reports compound SR+CNAME child-to-parent
and RR+CNAME parent-to-child success.

Runnable artifact: `target\release\amazing.exe secure-media-processes`.

Known gaps: consent refresh is caller-driven for one demonstrated cycle, not
a continuous event-loop timer; RTT-based response waiting, immediate consent
revocation, ICE restart execution, credential rotation, and multi-pair consent
remain pending. RTCP still lacks multiple report blocks, computed loss/jitter/
RTT statistics, periodic bandwidth scheduling, BYE/APP, feedback/NACK/PLI/TWCC,
and reduced-size negotiation. TURN/IPv6, remote browser interop, codecs,
jitter/congestion, multi-SSRC routing, SCTP/data channels, and WebRTC WPT also
remain pending.

Conformance: consent scheduling/expiry plus compound SR/RR+CNAME public gates
pass 3/3; the enhanced two-process secure integration gate passes 1/1. Existing
baselines remain 17/17 CSS2, 22/22 flex/grid, and 196/196 supported Test262;
no WebRTC WPT subset is imported yet.

Final win-001 gate: all 438 hardware-independent workspace tests/doctests
passed with `--locked`; all three strict first-party Clippy gates, the exact CI
build, stripped ThinLTO release, and enhanced two-process artifact passed. The
release binary is 4,559,872 bytes.

Recommended next sub-phase: integrate periodic consent into the cooperative
event loop, then add ICE retransmission/role conflict and computed RTCP
reception statistics before moving to jitter buffering/codecs. The user's
explicit “do all phases now” instruction pre-authorizes continuing.

## Phase 9.13 (transport robustness) — ICE timing + computed RR stats

Shipped: std-only `StunRetransmission` implements RFC 8489's default UDP
transaction schedule with a minimum 500 ms RTO and seven one-transaction-ID
sends at 0, 0.5, 1.5, 3.5, 7.5, 15.5, and 31.5 seconds. It waits the final
`16 * RTO`, fails at 39.5 seconds, supports triggered-check cancellation, and
cannot be resurrected by a response after failure. Success, failure, and
cancellation are terminal.

Pure RFC 8445 role-conflict logic now covers controlling/controlling and
controlled/controlled tiebreaker comparisons, 487 rejection, role switching,
opposite-role acceptance, and the mandatory client role switch after a 487.
The release artifact runs the full retransmission schedule plus a role switch
probe and reports `ICE retransmit=7 role-conflict=resolved`.

`RtpReceptionStats` observes real RTP packets for one SSRC and computes the
extended highest sequence across wrap, cumulative/interval loss and the
8-bit loss fraction, plus RFC 3550's integer 1/16 interarrival-jitter filter.
It samples those values with caller-provided LSR/DLSR into `ReceptionReport`.
Public gates cover nonzero loss/jitter plus sequence wrap, a late prior-cycle
packet, duplicates, interval accounting, and DLSR propagation.
The two-process artifact feeds the received child RTP through this module
before building the parent's compound RR+CNAME, and reports
`RTCP stats=computed`.

Runnable artifact: `target\release\amazing.exe secure-media-processes`.

Known gaps: retransmission and role-conflict behavior are deterministic public
state probes, not yet forced by dropped live packets or an encoded 487 error;
triggered-check queues, pair-priority recomputation, peer-reflexive insertion,
and random replacement tiebreakers remain pending. Reception tracking is one
SSRC with a bounded wrap/out-of-order model; full RFC probation, dropout/
misorder thresholds, duplicate accounting policy, real arrival-clock
conversion, periodic report scheduling, and multi-source aggregation remain
pending. TURN/IPv6, browser interop, jitter buffering/codecs, congestion,
SCTP/data channels, and WebRTC WPT are still out of this slice.

Conformance: RFC retransmission, role-conflict, and reception-statistics public
gates pass 4/4; the enhanced two-process integration gate passes 1/1.
Existing baselines remain 17/17 CSS2, 22/22 flex/grid, and 196/196 supported
Test262; no WebRTC WPT subset is imported yet.

Final win-001 gate: all 442 hardware-independent workspace tests/doctests
passed with `--locked`; all three strict first-party Clippy gates, exact CI
build, stripped ThinLTO release, and enhanced two-process artifact passed. The
release binary is 4,561,408 bytes.

Recommended next sub-phase: wire retransmission and 487 role switching into a
loss-injected live ICE gate, then add peer-reflexive candidates before starting
an RTP jitter buffer and codec boundary. The user's explicit “do all phases
now” instruction pre-authorizes continuing.

## Phase 9.14 (live ICE robustness) — loss, 487, peer-reflexive remote

Shipped: authenticated Binding error responses now encode and parse ERROR-CODE
487 with MESSAGE-INTEGRITY and FINGERPRINT. ICE requests can carry either role
attribute, so a controlling client can process the 487, switch to controlled,
generate a fresh nonzero tie-breaker from the vetted system RNG, and retry with
a new transaction ID. `IceChecklist` inserts and deduplicates a
peer-reflexive remote candidate from an authenticated request source address,
recomputes pair priority when the local role changes, and schedules the learned
pair as Waiting.

The release artifact now uses real loopback UDP rather than the old pure-state
probe: it deliberately drops the first two checks, sends the third with the
same transaction ID according to the RFC retransmission state, authenticates an
encoded 487, switches role, and completes a controlled retry. The receiving
checklist verifies that the client source became a peer-reflexive pair. The
two-process artifact reports `ICE live-loss=2 sends=3 487=resolved
prflx=learned` before continuing DTLS-SRTP and compound RTCP exchange.

Runnable artifact: `target\release\amazing.exe secure-media-processes`.

Known gaps: peer-reflexive local candidates, triggered-check queue ordering,
multi-pair foundation unfreezing, RTT-derived RTO, TURN/IPv6, ICE restart
execution, and browser interoperability remain.
The next transport milestone is an RTP jitter buffer and codec boundary.

Conformance: the Phase 9.14 authenticated 487, mutually exclusive role,
peer-reflexive insertion/deduplication, and live loss/retry public gates pass
4/4; the enhanced two-process secure integration gate passes 1/1. No WebRTC
WPT subset is imported yet.

Final win-001 gate: all 480 hardware-independent workspace tests/doctests
passed with `--locked`; the three strict first-party Clippy gates, exact CI
build, stripped ThinLTO release, and enhanced two-process artifact passed. The
release binary is 4,617,216 bytes.

## Phase 9.15 (RTP playout) — bounded jitter buffer + audio codec seam

Shipped: `amazing-webrtc::jitter` is a separate std-only module whose small
interface owns the RTP playout complexity. `RtpJitterBuffer` validates one
negotiated SSRC/payload type, extends 16-bit sequence numbers across wrap,
reorders within bounded storage, calculates fixed playout deadlines from the
RTP clock, rejects duplicate/late/overflow packets with measured counters, and
emits an explicit gap when a missing packet reaches playout time.

`RtpAudioDecoder` is a real codec seam with two adapters. `L16Decoder` converts
network-order signed 16-bit samples for negotiated rate/channel counts;
`PcmuDecoder` decodes G.711 mu-law samples. Both reject mismatched payload types
and malformed sample frames. The public probe deliberately receives sequence
`65535, 1, 0`, emits `65535, 0, 1`, rejects a duplicate, and decodes both codec
adapters.

Runnable artifact: `target\release\amazing.exe rtp-jitter-probe`, which reports
`RTP jitter packets=3 reordered=yes duplicate=rejected codecs=L16+PCMU
samples=3+2; jitter/codec probe ok`.

Known gaps: playout delay is fixed rather than adaptive; one buffer tracks one
SSRC/payload type; timestamp discontinuities and source changes require a new
buffer; gaps are reported but concealment is caller-owned. The secure
two-process path currently carries one packet per direction, so it does not
force live reordering or loss. Frame aggregation, codec encoders, Opus, video
codecs, audio-device playout, congestion feedback, and WebRTC WPT remain.

Conformance: jitter wrap/reordering, timed gap, bounded duplicate/late/overflow,
two real codec adapters, and public-probe gates pass 5/5; the command integration
gate passes 1/1. RFC 3551 defines the static PCMU and L16 profile mappings used
by the adapters; no WebRTC WPT subset is imported yet.

Final win-001 gate: all 486 hardware-independent workspace tests/doctests
passed with `--locked`; strict WebRTC/application Clippy, exact CI build,
stripped ThinLTO release, and the runnable jitter/codec artifact passed. The
release binary is 4,636,160 bytes.

## Phase 9.16 (secure playout) — SRTP → jitter → decoded audio

Shipped: the two-process DTLS-SRTP artifact now carries real codec payloads
instead of opaque marker text. After authenticated SRTP removal and RTP parsing,
each process creates negotiated per-source jitter state, waits for the fixed
playout deadline, and invokes the codec seam. The parent sends two network-order
L16 samples that the child validates as `1000, -1000`; the child returns PCMU
samples that the parent validates as `0, 32124`. RTCP sender/reception accounting
continues from the same decoded RTP packets.

Runnable artifact: `target\release\amazing.exe secure-media-processes`, which
now reports `playout=L16 parent->child=2 PCMU child->parent=2` before the compound
SRTCP/RTCP results and `two-process interop ok`.

Known gaps: this gate proves one secure packet per direction, not live network
reordering, gap concealment, adaptive delay, continuous clock-driven playout,
or audio-device output. Opus, video frame assembly/codecs, codec-specific fmtp,
NACK/PLI, congestion control, TURN, and browser interoperability remain.

Conformance: the enhanced secure two-process playout gate passes 1/1 alongside
the 5/5 jitter/codec public gates. No WebRTC WPT subset is imported yet.

Final win-001 gate: all 486 hardware-independent workspace tests/doctests
passed with `--locked`; strict application Clippy, stripped ThinLTO release,
and the secure two-process playout artifact passed. The release binary is
4,642,304 bytes.

## Phase 9.17 (codec negotiation) — SDP-authoritative jitter + decoder setup

Shipped: secure SDP now advertises the codecs actually sent—dynamic PT96 L16
at 48 kHz mono and PT0 PCMU at 8 kHz mono—instead of the former contradictory
Opus/PT111 mapping. `NegotiatedAudioCodecs` intersects offer and answer mappings,
requires the answer to preserve payload number/encoding/clock/channels, ignores
unsupported codecs only when a supported common mapping remains, and rejects
unnegotiated RTP payload types.

The negotiated set constructs both `JitterBufferConfig` and a boxed decoder
adapter from the received packet's payload type. Both secure processes use that
path; no fixture-owned clock rate, channel count, or codec constructor remains
in the playout call sites.

Runnable artifact: `target\release\amazing.exe secure-media-processes`, which
reports `codecs=SDP L16+PCMU` before the secure playout evidence.

Known gaps: negotiation supports one audio media section and exact common
rtpmap entries; it does not implement codec preference scoring, asymmetric
payload mappings, fmtp, rtcp-fb, direction attributes, bundle, or multiple
transceivers. Opus, video, and browser interoperability remain.

Conformance: two public negotiation gates cover construction and rejection;
the secure two-process negotiation/playout gate passes 1/1. No WebRTC WPT
subset is imported yet.

Final win-001 gate: all 488 hardware-independent workspace tests/doctests
passed with `--locked`; all three strict Clippy targets, stripped ThinLTO
release, and SDP-driven secure artifact passed. The release binary is
4,646,912 bytes.

## Phase 9.18 (adaptive playout) — bounded transit-variation control

Shipped: `RtpJitterBuffer::new_adaptive` adds an optional controller without
changing fixed-delay callers. It observes arrival transit in negotiated RTP
clock units, maintains an integer 1/16 EWMA of variation, multiplies that
estimate into a bounded delay target, raises the target immediately under
jitter, and decays by one millisecond per stable observation. Queued packets
derive playout from media offset plus the current target, so a delay increase
reschedules already buffered media instead of releasing it early.

The secure two-process path now constructs adaptive buffers from SDP-derived
clock rates with a 60–200 ms bound and reports `playout=adaptive` before its
L16/PCMU sample evidence.

Known gaps: the controller has no percentile history, underrun feedback,
clock-skew estimator, time-stretching, advanced concealment, or audio-device
clock. Delay decreases are deliberately conservative but not synchronized to
audio frame boundaries.

Conformance: the adaptive raise/reschedule public gate passes 1/1 alongside
the existing 7/7 jitter/codec/negotiation gates and secure process gate. No
WebRTC WPT subset is imported yet.

Final win-001 gate: all 489 hardware-independent workspace tests/doctests
passed with `--locked`; strict WebRTC/application Clippy, stripped ThinLTO
release, and adaptive secure playout artifact passed. The release binary is
4,647,936 bytes.

## Phase 9.19 (loss concealment) — secure gap → decoded silence → RTCP loss

Shipped: jitter gaps now include an estimated RTP timestamp and duration derived
from the last output and the next buffered packet. Gap output advances the
playout timestamp so consecutive missing packets remain ordered. Both codec
adapters implement concealment at their negotiated rate/channel count; the
current L16 and PCMU policy is duration-correct silence with checked allocation.

The secure child sends PCMU sequence 84 and 86 with 160-sample spacing, omitting
85. The parent decrypts both SRTP packets, emits sequence 84, produces a gap at
timestamp 2080 for 160 samples, inserts negotiated PCMU silence, then resumes
sequence 86. The same two received packets feed reception statistics, and the
child validates extended sequence 86 with RTCP cumulative loss 1.

Runnable artifact: `target\release\amazing.exe secure-media-processes`, which
reports `playout=adaptive loss=1 conceal=160 ... PCMU child->parent=320` and
`two-process interop ok`.

Known gaps: silence concealment is intentionally basic; there is no waveform
repetition, comfort-noise generation, Opus PLC/FEC, NACK recovery, burst-loss
policy, or audio-device clock. The live gate injects one missing packet, not a
stochastic loss/reorder profile.

Conformance: timestamp/duration gap and codec concealment assertions pass in
the public jitter gate; live secure loss/concealment/RTCP correlation passes
1/1. No WebRTC WPT subset is imported yet.

Final win-001 gate: all 489 hardware-independent workspace tests/doctests
passed with `--locked`; strict WebRTC/application Clippy, stripped ThinLTO
release, and secure loss artifact passed. The release binary is 4,657,152 bytes.

## Phase 9.20 (compressed audio) — isolated Opus encode/decode/PLC

Shipped: new `amazing-webrtc-codecs` keeps third-party codec code outside the
std-only protocol core and implements its existing decoder seam. It pins
`opus2` 0.4.0 with bundled, statically linked upstream libopus. First-party
adapter code forbids unsafe. The evaluated pure-Rust `mousiki` backend was
rejected after panicking on a valid 20 ms mono CELT frame; that failure is not
hidden behind a fallback.

`OpusEncoderAdapter` validates one/two-channel legal frame durations and emits
RTP payloads. `OpusDecoderAdapter` decodes negotiated packets and delegates an
empty packet to libopus PLC for loss. Both construct from the exact SDP mapping.
Secure SDP now offers Opus/PT111 first, followed by L16 and PCMU; parent-to-child
media is a real 960-sample Opus packet while the reverse loss gate stays PCMU.

Runnable artifacts: `target\release\amazing.exe opus-probe` reports
`Opus encoded=127 bytes decoded=960 samples PLC=960 samples; opus probe ok`;
`secure-media-processes` reports `codecs=SDP Opus+L16+PCMU` and
`Opus parent->child=960`.

Known gaps: the secure Opus leg does not inject loss/FEC; encoder bitrate,
complexity, DTX, in-band FEC, packet-loss percentage, stereo, and fmtp are not
configured. There is no resampling or audio device, and video remains pending.

Conformance: isolated Opus encode/decode/PLC passes 1/1; CLI Opus and enhanced
secure process gates pass 2/2. No WebRTC WPT subset is imported yet.

Final win-001 gate: all 491 hardware-independent workspace tests/doctests
passed with `--locked`; strict codec/WebRTC/application Clippy, stripped
ThinLTO release, standalone Opus artifact, and secure Opus artifact passed. The
release binary is 5,214,208 bytes.

## Phase 9.21 (video transport) — bounded RTP frame assembly

Shipped: new std-only `video` module consumes ordered `JitterOutput`, validates
one SSRC/payload type, and aggregates packet payloads sharing one RTP timestamp
until the marker bit. Packet count and byte caps prevent unbounded frames.
Missing packets, timestamp changes before a marker, and overflow drop the
damaged frame; discard continues through its marker so tail fragments cannot
be misreported as a complete frame.

The public probe inserts sequence 10, 12, 11 into the jitter buffer, emits
10, 11, 12, and assembles `frame-middle-end`. It separately injects a gap and
verifies damage is dropped. A bounded-overflow gate verifies recovery at the
next complete marked frame.

Runnable artifact: `target\release\amazing.exe video-frame-probe`, which reports
`video RTP packets=3 frame=16 bytes reordered=yes damaged=dropped; video frame
probe ok`.

Known gaps: this module assembles opaque encoded frames; it does not parse VP8/
VP9/AV1/H.264 payload descriptors, identify keyframes, request PLI/NACK, decode
pixels, synchronize audio/video, or present video. One assembler handles one
SSRC and payload type.

Conformance: reorder/marker/damage and bound/recovery public gates pass 2/2;
the command artifact passes 1/1. No WebRTC WPT subset is imported yet.

Final win-001 gate: all 494 hardware-independent workspace tests/doctests
passed with `--locked`; strict WebRTC/application Clippy, stripped ThinLTO
release, and video-frame artifact passed. The release binary is 5,218,816 bytes.

## Phase 9.22 (video decode) — stateful VP8 key/interframe reconstruction

Shipped: the isolated codec adapter now uses maintained `media-codec-vpx` 0.8.0
over libvpx 1.16.0. vcpkg is pinned to release `2026.07.29` commit
`9e593bb18ea69cc5095e012465dcd675a822ed0d`; the static `/MD` triplet matches
Rust's MSVC runtime without linker warnings. Explicit pkg-config/include/lib
environment values make clean builds reproducible. FFI and unsafe remain in
third-party adapter dependencies, not first-party Rust or the std-only core.

`Vp8DecoderAdapter` drives a stateful decoder and exposes decoded dimensions,
plane count, and pixel checksum. The committed 22,766-byte official libvpx
vector `vp80-02-inter-1402.ivf` matches published SHA-1
`71ea772d3e9d315b8cbecf41207b8a237c34853b`; the probe decodes its keyframe and
nine following interframes to three-plane I420 at 176×144, with changing pixel
checksums proving reconstructed frame output rather than header parsing.

Runnable artifact: `target\release\amazing.exe vp8-decode-probe`, reporting
`VP8 frames=10 interframes=9 size=176x144 pixels_changed=yes; vp8 decode probe
ok`.

Known gaps: the compatible maintained wrapper is intentionally decoder-only.
VP8 encode, RTP VP8 payload descriptors/fragmentation, SDP video negotiation,
secure video transport, keyframe detection/PLI, A/V sync, scaling, and GPUI
presentation remain.

Conformance: official key/interframe vector gate passes 1/1; command integration
passes 1/1. No WebRTC WPT subset is imported yet.

Final win-001 gate: all 496 hardware-independent workspace tests/doctests
passed with `--locked`; strict codec/WebRTC/application Clippy, stripped
ThinLTO release, and embedded official VP8 artifact passed. The release binary
is 7,724,032 bytes.

## Phase 9.23 (video codec) — bundled H.264 key/delta roundtrip

Shipped: the isolated codec adapter now pins `openh264` 0.9.8 with its bundled
Cisco OpenH264 source. Its safe high-level Rust interface preserves the
first-party `forbid(unsafe_code)` rule while providing both encoding and
stateful decoding on win-001. No preinstalled codec binary is required.

The artifact synthesizes two different 32×32 RGB frames, converts them to
YUV420, encodes them through one persistent encoder, and decodes the resulting
access units through one persistent decoder. The first access unit is 265 bytes,
the following delta unit is 153 bytes, dimensions stay 32×32, and reconstructed
pixel checksums change.

Runnable artifact: `target\release\amazing.exe h264-roundtrip-probe`, reporting
`H264 first=265 bytes delta=153 bytes size=32x32 pixels_changed=yes; h264
roundtrip probe ok`.

Known gaps: H.264 RTP packetization/depacketization, SPS/PPS signaling through
fmtp, profile-level-id negotiation, secure video transport, loss recovery/PLI,
A/V synchronization, scaling, and presentation remain. VP8 encoding remains
unavailable through the compatible safe wrapper.

Conformance: isolated key/delta encode/decode gate passes 1/1; command
integration passes 1/1. No WebRTC WPT subset is imported yet.

Final win-001 gate: all 498 hardware-independent workspace tests/doctests
passed with `--locked`; strict codec/application Clippy, stripped ThinLTO
release, and H.264 artifact passed. The release binary is 8,455,680 bytes.

## Phase 9.24 (multi-media signaling) — ordered audio + video SDP sections

Shipped: `SdpDescription` now owns ordered `SdpMediaSection` values instead of
one flat media tuple. Each section validates and preserves its media kind,
connection address, port, secure RTP profile, payload list, ICE candidates, and
rtpmap mappings. Session origin/name/timing, ICE credentials, DTLS fingerprint,
and setup role remain session-level. `media(kind)` and `primary_media()` keep
callers explicit.

The parser accepts multiple audio/video sections and rejects incomplete sections;
the encoder emits each section canonically and reparses for semantic equality.
The secure description builder now emits audio Opus/L16/PCMU and video VP8/H264
sections. Existing ICE/consent intentionally uses the primary audio section,
preserving all secure transport gates while video transport remains separate.

Runnable artifact: `target\release\amazing.exe sdp-roundtrip
examples\phase9-offer.sdp`, reporting `SDP audio+video payloads=3 candidates=2
rtpmaps=3 ICE=ferric setup=actpass; roundtrip ok`.

Known gaps: BUNDLE group, mid, rtcp-mux, direction attributes, shared-candidate
deduplication, rejected m-lines, section-level ICE credentials/fingerprints,
fmtp/rtcp-fb, and RTP demultiplexing by negotiated payload/SSRC remain. The
fixture uses distinct ports; the secure builder temporarily advertises the same
connected port in both sections and still selects only primary audio for ICE.

Conformance: multi-media parser/serializer public gate passes 1/1; upgraded
command artifact and existing secure process gates pass. No WebRTC WPT subset
is imported yet.

Final win-001 gate: all 499 hardware-independent workspace tests/doctests
passed with `--locked`; strict WebRTC/crypto/application Clippy, stripped
ThinLTO release, and multi-media SDP artifact passed. The release binary is
8,463,872 bytes.

## Phase 9.25 (bundled media) — BUNDLE + mid + rtcp-mux demultiplexing

Shipped: SDP session state now carries an ordered BUNDLE mid list. Each media
section owns a unique alphanumeric mid and explicit rtcp-mux flag. Parsing and
canonical serialization preserve these attributes. Validation requires every
bundled section to have rtcp-mux, every media mid to appear exactly once in the
group, and every group member to resolve to a real section.

`media_by_mid` resolves section identity and `media_for_payload_type` maps an
incoming RTP payload to exactly one negotiated media section. Missing payloads
are unsupported; reuse across audio/video is rejected as ambiguous before
transport dispatch. The secure builder advertises BUNDLE `0 1` with audio mid
0 and video mid 1 on the shared connected socket.

Runnable artifact: `target\release\amazing.exe sdp-roundtrip
examples\phase9-offer.sdp`, reporting `SDP audio+video bundle=2 payloads=3
candidates=2 rtpmaps=3 ... roundtrip ok`.

Known gaps: bundled RTP is not yet routed by SSRC/PT into independent audio and
video consumers. There is no MID RTP header extension, rejected m-line support,
direction negotiation, fmtp/rtcp-fb, or separate RTCP scheduling per media.

Conformance: positive BUNDLE roundtrip plus missing-member, duplicate-mid,
missing-rtcp-mux, and payload-collision assertions pass in the public SDP gate;
the upgraded command and existing secure process gates pass. No WebRTC WPT
subset is imported yet.

Final win-001 gate: all 499 hardware-independent workspace tests/doctests
passed with `--locked`; strict WebRTC/crypto/application Clippy, stripped
ThinLTO release, and bundled SDP artifact passed. The release binary is
8,473,088 bytes.

## Phase 9.26 (secure video) — bundled H.264 SRTP → I420

Shipped: the secure two-process artifact now negotiates H.264/PT121 on bundled
video mid 1 and sends a real encoded 32×32 access unit over the existing
DTLS-SRTP association. The controlled peer authenticates/decrypts RTP,
demultiplexes the payload to video, uses the negotiated 90 kHz clock for jitter,
assembles the marked encoded frame, and decodes it through the isolated
OpenH264 adapter to three-plane 32×32 output before acknowledging success.

The audio direction, live PCMU loss/concealment, Opus playout, consent, compound
SRTCP/RTCP, and reception statistics continue on the same association. The
artifact reports `codecs=SDP Opus+L16+PCMU+H264` and `video=H264 secure 32x32`.

Runnable artifact: `target\release\amazing.exe secure-media-processes`.

Known gaps: the small access unit fits one RTP packet; RFC 6184 single-NAL,
STAP-A, and FU-A packetization/depacketization are not implemented. Video has no
RTCP sender/receiver report, PLI/NACK, loss recovery, multiple-frame timing,
A/V sync, scaling, colorspace conversion, or GPUI presentation.

Conformance: enhanced secure bundled video gate passes 1/1 alongside all audio,
ICE, consent, DTLS-SRTP, and RTCP assertions. No WebRTC WPT subset is imported.

Final win-001 gate: all 499 hardware-independent workspace tests/doctests
passed with `--locked`; strict codec/WebRTC/application Clippy, stripped
ThinLTO release, and secure H.264 artifact passed. The release binary is
8,480,768 bytes.

## Phase 9.27 (video presentation) — decoded I420 → RGBA → GPUI

Shipped: both isolated video decoders now return owned RGBA pixels in addition
to geometry/checksum evidence. VP8 I420 planes are bounds/stride validated and
converted with integer BT.601 limited-range math; H.264 uses the safe wrapper's
RGBA conversion. Tests require exact `width × height × 4` storage and opaque
alpha.

The standalone GPUI shell depends on the isolated codec adapter, generates a
320×180 patterned RGB frame, encodes and decodes H.264, converts I420 to RGBA,
swizzles to GPUI's BGRA render image, and presents it in the former WPT tab now
labeled `Secure video · H.264 decoded frame`. `--video` launches directly into
that tab. Engine status remains the default tab.

Runnable artifact: `target\release\amazing-shell.exe --video`. The committed
native win-001 capture is `docs/screenshots/gpui-video-presentation-win001.png`
(SHA-256 `7fc2d4c9a2c9bf089052650c59a65a49183fed273cab34803617dfd95b36087b`).

Known gaps: this is a deterministic decoded frame, not a continuous remote
video clock. The secure-process H.264 frame and GPUI preview use the same codec
path but are separate runtime processes. Animation, frame queues, A/V sync,
scaling policy, colorspace metadata, and direct secure-session-to-GPUI handoff
remain.

Conformance: opaque RGBA adapter and GPUI video-surface gates pass; standalone
shell tests pass 11/11. Native Windows capture, locked ThinLTO shell build, and
all root gates pass.

Final win-001 gate: all 500 hardware-independent root workspace tests/doctests
passed with `--locked`; strict codec Clippy and stripped ThinLTO root release
passed. Root release is 8,496,640 bytes; standalone GPUI release is 14,133,760
bytes. Both manifests declare the dependency graph's actual Rust 1.89 MSRV.

## Phase 9.28 (clocked video) — RTP playout queue + continuous GPUI frames

Shipped: new generic std-only `VideoPlayoutQueue<T>` extends 32-bit RTP
timestamps across wrap, orders frames, bounds capacity, rejects duplicates and
already-emitted late frames, computes due time from the negotiated clock and a
caller delay, and reports queue/emission/drop counters. Its gate crosses the
`u32::MAX → 0` wrap and proves 40/50/60 ms release order.

The GPUI video tab now holds six frames generated by one persistent H.264
encoder and decoder. A detached 500 ms GPUI task advances the render image and
notifies the shell while preserving tab/address/history state. The app remains
responsive and browser chrome stays fixed.

Runnable artifact: `target\release\amazing-shell.exe --video`. Two captures
from the same native window prove frame advancement:

- frame A SHA-256 `069a1876fa2c0d73c41af4b47c08b5aa77c6119d6b259480b4ee3a86a84bc9a9`;
- frame B SHA-256 `62fd42795001f63479b8574aad221833ec9ad10eb08141d81562933122940bee`.

Known gaps: GPUI currently advances a deterministic in-memory loop rather than
the secure receiver's decoded queue. The timer is fixed at 500 ms and not yet
driven from RTP due times; there is no frame skipping, A/V master clock,
backpressure, visibility pause, or end-of-stream handling.

Conformance: RTP timestamp wrap/order/late/duplicate queue gate passes; GPUI
model and timer-compatible frame-cycle gates pass. Root workspace tests pass
501/501 and standalone shell tests pass 12/12. Native two-frame proof and both
locked ThinLTO builds pass.

Final win-001 sizes: root release 8,492,544 bytes; standalone GPUI release
14,161,920 bytes.

## Phase 9.29 (continuous secure video) — six-frame SRTP clocked playout

Shipped: the parent secure process uses one persistent H.264 encoder to send six
key/delta access units on bundled video PT121/SSRC `0x56494445`. Sequence numbers
200–205 and 90 kHz timestamps 90000–315000 advance at 500 ms intervals. Every
access unit is authenticated and encrypted by the existing SRTP context.

The controlled peer verifies every payload/SSRC/sequence/timestamp, buffers all
six through RTP jitter, assembles marked frames, decodes them with one persistent
H.264 decoder, and enqueues owned RGBA frames in `VideoPlayoutQueue`. Simulated
clock release proves strict timestamp order, six emissions, stable 32×32×4
geometry, and changing pixel checksums.

Runnable artifact: `target\release\amazing.exe secure-media-processes`, now
reporting `video=H264 secure frames=6 32x32 clock=90k changed=yes` before
`two-process interop ok`.

Known gaps: each small access unit still fits one RTP packet, so FU-A/STAP-A and
loss recovery remain pending. The secure decoded queue lives in the child
process and is not yet streamed into GPUI. Wall-clock sleeping, drift, A/V sync,
visibility/backpressure, and video RTCP feedback are not implemented.

Conformance: enhanced secure six-frame timing/geometry/pixel gate passes 1/1;
all prior ICE, consent, audio loss, Opus, RTCP, and video assertions remain.

Final win-001 gate: all 501 hardware-independent workspace tests/doctests pass
with `--locked`; strict application Clippy, stripped ThinLTO release, and
continuous secure-video artifact pass. The release binary is 8,511,488 bytes.

## Phase 9.30 (secure frame stream) — decoded RGBA process seam

Shipped: `amazing secure-video-stream <host:port>` runs the complete existing
two-process ICE/DTLS-SRTP/audio/video session while passing the optional sink
address only to the controlled child. After secure H.264 decode and clocked
playout, the child writes six bounded binary records to that localhost stream.

Each `AMVF` record contains a 24-byte little-endian header—magic, zero-based
index, width, height, original RTP timestamp, RGBA byte length—followed by owned
opaque RGBA. Frames are capped at 16 MiB and must equal `width × height × 4`.
The sender uses write timeout and explicit flush; stream EOF is completion.

The integration listener parses all six records independently, verifies indices
0–5, dimensions 32×32, timestamps 90000–315000, length 4096, opaque alpha, no
trailing bytes, and changing pixel checksums. The release artifact delivers
exactly 24,720 bytes and reports `secure RGBA stream frames=6 size=32x32
timestamps=90k; stream ok`.

Known gaps: the localhost sink has no separate authentication or origin policy;
it assumes a listener created by the same desktop application. There is no
reconnect, version negotiation, backpressure beyond blocking writes, frame-drop
policy, shared memory, or cross-machine transport. GPUI consumption is next.

Conformance: framed-stream integration passes 1/1 alongside every secure media
assertion. Root workspace tests pass 502/502; strict application Clippy and
release artifact pass. Release binary is 8,515,584 bytes.

## Phase 9.31 (direct secure UI) — DTLS-SRTP child → RGBA stream → GPUI

Shipped: `amazing-shell.exe --secure-video` binds an ephemeral loopback listener
before launching sibling `amazing.exe secure-video-stream`. A named background
thread owns process/accept/read lifecycle, handles Windows WSAEWOULDBLOCK 10035,
restores blocking mode on the accepted socket, applies a 15-second read timeout,
and validates AMVF magic, monotonic indices, dimensions, 16 MiB cap, exact RGBA
length, and opaque alpha before emitting UI events.

The existing 500 ms GPUI task drains those events. The first secure frame clears
the synthetic set; all six become native render images on the UI context. The
badge reports `SECURE SRTP COMPLETE · 6 decoded frames`, proving that display
origin, and browser chrome/history remain interactive.

Runnable artifact: `target\release\amazing-shell.exe --secure-video`. Native
proof hashes from one running window differ as secure frames cycle:

- frame A `4eb20d7b45c5b6ece367ab4587c9b8df076419fe0d7aa83ebeec287abd3dda8d`;
- frame B `3c1630d8ac3aaa7bde8594f789bb8ce46149e88b6bdd7b56f8567ed00341f39c`.

Known gaps: loopback IPC trusts same-user local processes and has no handshake
version beyond AMVF magic. Frames arrive as a completed burst and then loop;
GPUI does not yet preserve original RTP wall-clock spacing, synchronize audio,
apply backpressure/drop policy, restart a failed session, or pause when hidden.

Conformance: bounded frame-reader unit gate passes; standalone shell tests pass
13/13; root secure stream integration remains green. Both native captures,
locked ThinLTO shell build, strict root gates, and 502/502 workspace tests pass.

Final win-001 sizes: root release 8,515,584 bytes; standalone GPUI release
14,231,552 bytes.

## Phase 9.32 (A/V synchronization) — RTCP SR anchored media timeline

Shipped: new std-only `MediaClockAnchor` converts an RTCP Sender Report's 32.32
NTP timestamp plus signed wrap-safe RTP delta into nanoseconds at the negotiated
clock rate. `AvSyncTimeline` uses separate audio/video anchors and a bounded
tolerance to classify each video timestamp as `Hold`, `Present`, or `Drop`
against the audio-master RTP timestamp.

Public gates cover NTP fractional conversion, RTP wrap through `u32::MAX → 0`,
and exact ±100 ms hold/drop decisions around a 20 ms tolerance. Invalid zero
clock, same-SSRC anchors, and excessive tolerance are rejected.

The secure parent sends authenticated compound Sender Reports for Opus audio at
the common NTP origin and H.264 video at its final 2.5-second point. The child
validates SSRC/CNAME/count/timestamp, constructs the timeline, and requires all
six decoded video frames to be `Present` against audio timestamps advancing by
24000 samples per 500 ms. Output reports `avsync=RTCP-SR 6/6 aligned`.

Runnable artifact: `target\release\amazing.exe secure-media-processes`.

Known gaps: NTP anchors are fixture timestamps rather than wall-clock samples;
there is no drift/skew estimator, periodic SR refresh, audio device cursor,
resampling/time-stretch, discontinuity recovery, or GPUI due-time integration.

Conformance: two public media-clock/sync gates and enhanced secure A/V gate
pass. Workspace tests pass 504/504; strict WebRTC/application Clippy and
stripped ThinLTO artifact pass. Release binary is 8,521,216 bytes.

## Phase 9.33 (UI media clock) — secure RTP timestamp-driven GPUI timing

Shipped: GPUI retains each AMVF frame's original 90 kHz RTP timestamp alongside
its render image. `next_video_delay_ms` converts the current-to-next wrapping
delta to milliseconds, clamps unsafe values to 16–5000 ms, and uses the final
observed interval when the six-frame demo loops. Synthetic preview frames carry
the same 45000-tick schedule.

The detached GPUI task now carries its previous computed delay across awaits,
polls secure events, advances the frame, notifies rendering, and derives the
next delay from the newly current frame. Unit gates cover 100/200 ms irregular
intervals and 500 ms loop wrap; tab/address/history state remains unchanged.

`--secure-video` now reports `SECURE SRTP COMPLETE · 6 decoded frames · 90 kHz
clock`. Two captures from one window differ while retaining that badge:

- A `9f3e953025b18b60d8afbff96f502b6d1de3c1d65b733dc7834e40064cd81acd`;
- B `624fb8e5bbe2892579c88292a46326b99f298d0b9aafd2b918e2573ad3e4355d`.

Known gaps: AMVF does not carry the audio Sender Report anchor, so GPUI honors
video cadence but does not independently apply audio-master hold/drop decisions.
There is no drift correction, visibility pause, backpressure, or dropped-frame
telemetry in the shell.

Conformance: standalone shell tests pass 14/14; RTP timing unit gate, direct
secure stream, warning-free locked ThinLTO build, and two-frame native proof
pass. Shell release is 14,232,576 bytes; root remains 504 tests and 8,521,216
bytes.

## Phase 9.34 (UI A/V timeline) — RTCP presentation time reaches GPUI

Shipped: the localhost frame protocol is versioned from AMVF to AMV2. Its
28-byte little-endian header contains magic, index, dimensions, original RTP
timestamp, audio-master-relative presentation milliseconds, and RGBA length.
The secure child maps each decoded video timestamp through the video RTCP Sender
Report and subtracts the audio Sender Report origin, producing exact presentation
times 0, 500, 1000, 1500, 2000, and 2500 ms for the six-frame session.

The standalone reader rejects old magic, non-monotonic indices, a non-zero
presentation origin, backward/equal presentation times, overflowing dimensions,
non-exact RGBA lengths, frames above 16 MiB, and non-opaque alpha. The GPUI model
stores only the resulting presentation milliseconds beside render images;
`next_video_delay_ms` uses their direct difference and the last observed interval
for loop wrap, clamped to 16–5000 ms. It no longer reconstructs timing from a
hard-coded 90 kHz video clock.

`--secure-video` reports `SECURE SRTP COMPLETE · 6 decoded frames · RTCP A/V
clock`. Native captures from one live win-001 session retain that badge while
showing different decoded frames:

- A `55250245860f1c9bd5bfd4381dd95293fb9701b0a8d3a3f7c8265a979abe71a3`;
- B `eafee00629defccce91375a9a06875a63765187fed5fdb33fda93a1e5f02b538`.

Known gaps: the presentation timeline is a completed local burst that loops;
there is still no live audio device cursor, periodic Sender Report refresh,
drift/skew correction, visibility pause, backpressure/drop policy, or restart
after a failed secure session. AMV2 is a fixed local protocol version without a
capability-negotiation handshake.

Conformance: the exact stream integration gate receives six AMV2 records and
24,744 bytes with no trailing data. Standalone shell tests pass 15/15, including
malformed-origin rejection and irregular 100/200 ms scheduling. Root workspace
tests remain 504/504. Strict `-D warnings` Clippy passes for both changed
application packages with `--no-deps`; locked stripped ThinLTO builds pass.
Final win-001 sizes are 8,521,728 bytes for `amazing.exe` and 14,233,600 bytes
for `amazing-shell.exe`.

## Phase 9.35 (absolute UI clock) — deadline catch-up without cadence drift

Shipped: a pure in-process `VideoPlaybackClock` module now owns the looping
presentation timeline, monotonic origin, absolute sequence, cumulative late
drops, and next bounded deadline. Its interface takes only a validated timeline
and host `now_ms`; each update returns the due frame index, total drops, and next
wait. GPUI no longer advances exactly one frame after every relative timer.

For `[0, 100, 300]`, deterministic gates prove deadlines of 100, 200, and 150
ms across the irregular schedule and inferred 200 ms loop interval. A first wake
at 350 ms jumps directly from frame 0 to frame 2, counts exactly one skipped
frame, and schedules the next absolute boundary 150 ms later. A second sample at
360 ms neither recounts the drop nor shifts that boundary. Replacement resets
origin, sequence, and telemetry; waits remain clamped to 16–5000 ms.

The live secure timeline is installed atomically after all six validated AMV2
records arrive. The badge reports `SECURE SRTP COMPLETE · 6 decoded frames ·
RTCP A/V clock · late 0`. Native captures from one win-001 process show changing
decoded pixels with no late frames:

- A `aae9683f4638e482fe50d08955c1a4d4c5aa74a8334951bafbad6a4b1e7256aa`;
- B `8b238b02f5906dd84b41f141515709ef71aec5d25e19065808ad4804b31e8adb`.

Known gaps: late frames are skipped only at the UI image-selection layer; the
completed six-frame burst still decodes every frame before presentation. There
is no live queue backpressure, audio device cursor, drift estimator, visibility
pause, periodic RTCP refresh, or discontinuity/restart policy.

Conformance: five clock-interface gates plus the retained stream and shell gates
pass; standalone GPUI tests are 19/19. Strict `-D warnings` Clippy and the locked
stripped ThinLTO build pass on win-001. Root remains 504/504 tests and 8,521,728
bytes; the shell release is 14,235,648 bytes.

## Phase 9.36 (progressive UI stream) — paced AMV2 delivery and display

Shipped: when the optional RGBA sink is active, the secure child anchors an
`Instant` at the first decoded output and sleeps only the remaining duration to
each RTCP-derived `presentation_ms` before writing its AMV2 record. The normal
secure media artifact remains unpaced; pacing is isolated to the presentation
adapter and does not enter the std-only WebRTC core.

`VideoPlaybackClock` now has a progressive mode. The first validated zero-time
frame resets an unsealed timeline; later strictly increasing presentations are
appended without loop wrap. While no next record is known, GPUI polls at 50 ms.
EOF seals the same timeline into its inferred-interval replay loop. Appends after
seal, duplicate/backward times, and non-zero protocol origin remain errors.

The integration receiver reads every 28-byte header and RGBA payload separately
and timestamps completion of each record. The measured win-001 inter-arrival
deltas were exactly `[500, 499, 500, 499, 500] ms`; gates require every delta to
be at least 300 ms and the first-to-last span at least 2200 ms. The wire result
remains six records, 24,744 bytes, changing opaque pixels, and no trailing data.

Native captures prove the intermediate UI state rather than only replay after
EOF:

- frame 1 / A/V 0 ms / RTP 90000:
  `9d17d224f51361ae202de7decb98cb8ad55a1f94f7b1ef8be518026271b7abe8`;
- frame 2 / A/V 500 ms / RTP 135000:
  `7f1a88cd77123196eca8a0bb713f195cb43b5b27d98f69a0c054c6269a04b96f`.

Known gaps: all six H.264 access units still arrive and decode before the RGBA
adapter paces output, so this is progressive delivery/presentation rather than
incremental network decode. There is no bounded cross-process queue,
backpressure, producer-side late-drop policy, audio device cursor, visibility
pause, periodic RTCP refresh, or restart after failure.

Conformance: the paced secure-stream integration, six progressive-clock gates,
and retained shell gates pass. Root workspace remains 504/504; standalone GPUI
is 20/20. Strict changed-package Clippy and locked stripped ThinLTO builds pass
on win-001. Release sizes are 8,521,728 bytes (`amazing.exe`) and 14,236,160
bytes (`amazing-shell.exe`).

## Phase 9.37 (incremental secure decode) — paced SRTP packet to live RGBA

Shipped: the parent now encodes the six fixture access units into owned RTP
packets, sends authenticated audio/video RTCP Sender Reports before video, and
paces SRTP packet transmission at 0, 500, 1000, 1500, 2000, and 2500 ms when a
frame sink is active. The ordinary `secure-media-processes` artifact retains its
fast unpaced path; only the UI stream requests real-time network pacing.

The child constructs `AvSyncTimeline` before video reception. Every paced packet
then independently crosses SRTP authentication, the 30 ms jitter deadline,
bounded frame assembly, persistent H.264 decode, `VideoPlayoutQueue`, RTCP A/V
validation, and AMV2 output before the next packet is received. `SecureVideoOutput`
concentrates dimension/alpha/order/sync/checksum validation and optional sink
writes behind one internal presentation interface.

The release reports `pace=RTCP decode=incremental`. The measured win-001 AMV2
inter-arrival deltas after this reorder were `[499, 500, 499, 500, 499] ms`;
each record can only be written after that packet has decoded. Native captures
show the intermediate end-user state:

- frame 1 / A/V 0 ms / RTP 90000:
  `afdc6857c127ead5ef3ce8bb02d41c58c4876f0681caf73a9107d4e4432ab31b`;
- frame 2 / A/V 500 ms / RTP 135000:
  `bfc83bcef59b8729296cef3ce18155caab96762ae20a2e69134586c111dd894f`.

Both badges visibly identify `LIVE SRTP → JITTER → H.264 → RGBA`.

Known gaps: the local fixture still encodes all access units ahead of send and
uses one RTP packet per access unit. The progressive path has no induced video
loss/reorder, NACK/PLI/FIR feedback, keyframe recovery, bounded cross-process
backpressure, audio device cursor, network peer, or browser interoperability.

Conformance: the enhanced secure-stream gate, every retained secure-media gate,
and full root workspace pass 504/504. Standalone GPUI remains 20/20. Strict
changed-package Clippy and locked stripped ThinLTO builds pass on win-001.
Release sizes are 8,523,264 bytes (`amazing.exe`) and 14,236,160 bytes
(`amazing-shell.exe`).

## Phase 9.38 (bounded UI handoff) — decoded-frame backpressure

Shipped: the background AMV2 reader now sends `SecureVideoEvent` values through
a synchronous channel with capacity three instead of an unbounded MPSC queue.
When GPUI is slow, the fourth pending event blocks the reader; unread TCP bytes
then apply kernel/socket backpressure to the secure child writer, whose existing
five-second timeout turns a persistently stalled consumer into a bounded error.
Decoded RGBA memory is therefore independent of stream duration.

The interface-level saturation gate uses capacity one and writes two complete,
ordered AMV2 records. It proves the reader cannot finish during a measured 50 ms
interval while the first event occupies the channel, then drains frame indices
0 and 1, unblocks the second send, observes EOF, and joins the reader within one
second. Header, alpha, index, timeline, and byte bounds remain enforced before
an event can occupy queue capacity.

The production `--secure-video` run completed all six incremental frames with
`late 0 · queue 3`, proving that the 2 fps path remains lossless. Native evidence:
`4cbb6c8ab4983ae47482f55a1c47e2255a5dd10b4959e7f30c9e0c633d37b8de`.

Known gaps: saturation blocks rather than selecting a stale frame to drop, and
a consumer stalled longer than the child write timeout fails the session. There
is no high/low-water telemetry, adaptive queue sizing, restart/resume, visibility
pause, producer-side late-drop policy, or long-running network peer yet.

Conformance: standalone GPUI passes 21/21, including the deterministic bounded
handoff gate. The incremental secure stream and root workspace remain 504/504.
Strict changed-package Clippy and locked stripped ThinLTO builds pass on win-001.
Release sizes are 8,523,264 bytes (`amazing.exe`) and 14,236,672 bytes
(`amazing-shell.exe`).

## Phase 9.39 (video-loss recovery) — authenticated PLI and forced IDR

Shipped: the std-only WebRTC core now owns the exact 12-byte RFC 4585
Payload-Specific Feedback/Picture Loss Indication shape. The public value carries
sender and media SSRCs; encode emits V=2/FMT=1/PT=206/length=2, and parse rejects
wrong format, packet type, padding/length, and truncation. The isolated H.264
adapter exposes the safe OpenH264 0.9.8 `force_intra_frame` control without
leaking codec or FFI state across its interface.

The secure parent sends seven authenticated RTP packets. Packet three advances
encoder reference state but carries a deliberately damaged access unit. The
child reaches that packet through its normal jitter/assembly path, requires
decode failure, and sends PLI over SRTCP. The parent validates both SSRCs, forces
the next encoded frame to IDR, and resumes deltas afterward. Six decoded frames
reach GPUI at RTP timestamps 90000, 135000, 225000, 270000, 315000, and 360000;
their A/V presentations are 0, 500, 1500, 2000, 2500, and 3000 ms.

Measured AMV2 inter-arrival deltas are `[499, 999, 499, 500, 500] ms`: one
visible damaged-frame gap followed by restored 500 ms cadence. GPUI detects that
single discontinuity and completes with `PLI/IDR 1 · late 0 · queue 3`. Native
evidence:
`4d04b310d47b24a55b7d847c4cc562917633d55d4326951cb8792a5d4ba08db8`.

Known gaps: the fault is an authenticated damaged access unit rather than an
actual dropped UDP datagram. There is no Generic NACK, FIR, PLI rate limiting,
retransmission buffer, repeated-loss escalation, decoder reset fallback, or
remote browser peer. The fixture still uses one RTP packet per access unit.

Conformance: new PLI shape and forced-IDR codec gates pass; both secure
two-process modes recover; root workspace passes 506/506 and standalone GPUI
passes 22/22. Strict changed-package Clippy and locked stripped ThinLTO builds
pass on win-001. Release sizes are 8,525,312 bytes (`amazing.exe`) and 14,236,672
bytes (`amazing-shell.exe`).

## Phase 9.40 (actual RTP loss) — jitter gap to PLI/IDR recovery

Shipped: the secure encoder advances through phases 0–7 but deliberately omits
the phase-2 SRTP datagram (sequence 202, timestamp 180000). It then sends the
dependent phase-3 delta. The child receives explicit sequences 200, 201, and
203–207; its fixed-delay jitter buffer emits exactly one gap for 202. The frame
assembler enters discard-through-marker state for the dependent delta and sends
one PLI over the authenticated SRTCP association.

The parent waits after phase 3, validates the PLI sender/media SSRC pair, forces
phase 4 to IDR, and resumes ordinary delta encoding. Six decoded outputs reach
GPUI at RTP timestamps 90000, 135000, 270000, 315000, 360000, and 405000, mapped
to audio-master presentations 0, 500, 2000, 2500, 3000, and 3500 ms. Both fast
and real-time secure modes require exactly one jitter loss and one PLI.

Measured AMV2 inter-arrival deltas are `[499, 1499, 500, 499, 500] ms`: the
missing frame and discarded dependent delta form one 1.5-second gap, then IDR
restores 500 ms cadence. The end-user badge reports `RTP loss 1 · IDR 1 · late 0
· queue 3`. Native evidence:
`813de38903661c15fd52f5917fa23ec80ba1ccd94294a37edcb8ce7b6d068988`.

Known gaps: this is one deterministic single-packet loss. There is no burst-loss
matrix, packet reordering in the secure video path, Generic NACK/retransmission,
FIR, PLI rate limiting, repeated-loss escalation, long-term reference recovery,
or remote browser peer. Access units still fit one RTP packet each.

Conformance: both secure process modes, exact paced-stream assertions, and all
retained media gates pass. Root workspace remains 506/506; standalone GPUI is
22/22. Strict changed-package Clippy and locked stripped ThinLTO builds pass on
win-001. Release sizes are 8,525,312 bytes (`amazing.exe`) and 14,237,184 bytes
(`amazing-shell.exe`).

## Phase 9.41 (audio-device master) — secure PCM → WinMM cursor → GPUI

Shipped: the localhost stream is versioned to AMA3/AMV3. A 20-byte AMA3 header
declares 48 kHz mono PCM, 1,440,000 sample frames, and 2,880,000 little-endian
bytes. The child derives the track solely from the decoded secure Opus frame and
repeats it to a bounded 30-second device buffer. Six AMV3 RGBA records follow,
making the exact stream 2,904,764 bytes with no trailing data.

The standalone shell directly pins `windows-sys` 0.61.2 with only
`Win32_Media_Audio`. New `audio_output` is the sole unsafe module: a named thread
opens `WAVE_MAPPER` for PCM16, keeps boxed samples and `WAVEHDR` stable through
prepare/write/done/unprepare/close, polls `waveOutGetPosition` in samples (or
bytes when substituted), and publishes cursor/active state through atomics. The
crate root changed from `forbid` to `deny` unsafe so only this explicitly allowed
platform adapter can contain reviewed unsafe blocks.

GPUI starts audio on the AMA3 event, roots the progressive video timeline at
device sample zero, and samples the device cursor after each event poll. Native
captures measured cursor A at 10,518 ms and cursor B at 11,526 ms—1,008 ms apart—
while decoded pixels changed:

- A `b65244c682117580d072307d0bd6596615a3d705bf7943c5530c943d1d2e3d9c`;
- B `69fa4b8d14ba812f9e8e3bbe5c674e998fd0b2e12f5691c432e40b960026eab3`.

The device remained `playing`. Six late video frames were counted once when the
unfocused RDP window was throttled; the count stayed stable while the hardware
cursor and video advanced, proving absolute audio-master catch-up rather than
ongoing cadence drift.

Known gaps: the 20 ms secure Opus fixture is repeated rather than receiving a
continuous audio RTP stream. The whole 30-second PCM track is buffered at once;
there is no streaming wave-header ring, underrun recovery, volume/mute control,
pause on visibility, device-change handling, or playback beyond 30 seconds.
TURN and external browser interoperability also remain open.

Conformance: AMA3 bounds/PCM assertions pass inside the enhanced secure-stream
gate; two pure audio cursor/validation gates bring standalone GPUI to 24/24.
Root workspace remains 506/506. Strict changed-package Clippy and locked stripped
ThinLTO builds pass on win-001. Release sizes are 8,526,848 bytes (`amazing.exe`)
and 14,268,928 bytes (`amazing-shell.exe`).

## Phase 9.42 (TURN allocation) — authenticated UDP relay lifecycle

Shipped: new std-only `turn` module implements the bounded IPv4/UDP subset of
RFC 8656 behind one `run_turn_udp_probe` interface. It owns Allocate request,
401 ERROR-CODE challenge, REALM/NONCE, UDP REQUESTED-TRANSPORT, LIFETIME,
XOR-RELAYED/XOR-MAPPED/XOR-PEER addresses, CreatePermission, Send/Data
indications, and lifetime-zero Refresh deletion. Duplicate required attributes,
bad cookies/fingerprints, changed client 5-tuples, wrong peer permissions, and
integrity failures are rejected.

Long-term authentication uses the explicitly agreed legacy MESSAGE-INTEGRITY
mode. Its HMAC key is MD5(`username:realm:password`); the RFC 8489 example
`user:realm:pass → 8493fbc53ba582fb4c044c456bdc40eb` is a byte-exact gate.
Every authenticated request and response validates HMAC-SHA1 plus final STUN
FINGERPRINT. The nonce starts with the RFC 8489 zero-feature cookie
`obMatJos2AAAA` and is echoed on all subsequent requests.

The real loopback adapter binds separate TURN control, relayed, client, and peer
UDP sockets. It completes challenge/retry allocation, creates a permission,
relays `through-turn` client→peer, returns `relay-return` as a Data indication,
and deletes the allocation. The stripped win-001 artifact reported:

`TURN challenge=401 relay=127.0.0.1:59189 mapped=127.0.0.1:59190 lifetime=600s
permission=ok data=12/12 delete=ok; turn probe ok`.

Runnable artifact: `target\release\amazing.exe turn-probe`.

Known gaps: the interoperable legacy credential path does not yet negotiate
PASSWORD-ALGORITHMS or MESSAGE-INTEGRITY-SHA256. The module supports IPv4 UDP
only: no DNS/SRV discovery, TLS/DTLS/TCP transport, IPv6/dual allocation,
ChannelBind/ChannelData, stale-nonce retry, alternate server, quotas, or external
TURN deployment. Relay candidates are not yet inserted into the ICE checklist.

Conformance: three TURN module gates and one root artifact gate bring the root
workspace to 510/510; standalone GPUI remains 24/24. Strict changed-package
Clippy and the locked stripped ThinLTO root build pass on win-001. Release sizes
are 8,575,488 bytes (`amazing.exe`) and 14,268,928 bytes (`amazing-shell.exe`).

## Phase 9.43 (relay candidate nomination) — TURN allocation enters ICE

Shipped: `IceCandidate` now carries an explicit `IceCandidateKind` (`Host`,
`ServerReflexive`, `PeerReflexive`, or `Relay`) plus an optional related
transport address. The public priority helper applies type preferences
126/110/100/0, 16-bit local preference, and inverse component ID; invalid
components outside 1–256 fail. Existing host and learned peer-reflexive
candidates are classified at their construction sites.

SDP candidate parsing/encoding now round-trips `host`, `srflx`, `prflx`, and
`relay`. Non-host candidates may carry the exact `raddr`/`rport` pair; host
candidates reject a related address. A relay fixture serializes as
`typ relay raddr 127.0.0.1 rport 5000`, parses back identically, and retains the
lowest candidate type priority.

`TurnProbeReport` now returns the constructed relay candidate with its mapped
address as the related address. Only after the real client→peer and peer→client
relay payloads validate does the module start its relay/host candidate pair,
mark it succeeded and nominated, and require `ChecklistState::Completed` for
component 1. The stripped artifact reported:

`TURN challenge=401 relay=127.0.0.1:63839 mapped=127.0.0.1:63840 lifetime=600s
permission=ok data=12/12 nominate=ok delete=ok; turn probe ok`.

Known gaps: TURN gathering is not yet driven by a configured external server,
and the nominated relay pair is not passed into DTLS-SRTP endpoint construction.
The SDP parser remains IPv4/UDP and does not accept browser extension tokens such
as generation/network-id/network-cost. SHA-256 credentials, ChannelData, DNS,
TLS/TCP, and external browser interoperability remain open.

Conformance: enhanced SDP and TURN interface gates, the root artifact, and all
retained suites pass. Root workspace remains 510/510; standalone GPUI remains
24/24. Strict changed-package Clippy and the locked stripped ThinLTO root build
pass on win-001. Release sizes are 8,579,584 bytes (`amazing.exe`) and 14,268,928
bytes (`amazing-shell.exe`).

## Phase 9.44 (Chromium SDP ingest) — observed offer shape and dynamic PTs

Shipped: a real current Chromium offer was generated with Playwright from two
recvonly audio/video transceivers. The captured shape included BUNDLE mids,
media-level ICE/fingerprint/setup, mDNS host candidates, candidate
`generation`/`network-id`/`network-cost` extensions, recvonly, rtcp-mux, Opus PT
111, and H.264 PT 102 among a larger codec set. A curated faithful subset is
preserved as `examples/phase9-chromium-offer.sdp` and is a runnable artifact.

Candidate parsing now accepts up to eight bounded extension pairs. Numeric
generation/network fields are validated; unknown nonempty extension pairs are
semantically ignored. Unresolved `.local` host candidates are skipped instead
of aborting the description, while literal relay candidates and `raddr`/`rport`
(including browser `0.0.0.0:0`) remain modeled. The fixture keeps exactly two
relay candidates after dropping two mDNS hosts.

`SdpDirection` adds sendrecv/sendonly/recvonly/inactive to every media section;
the parser retains Chromium recvonly and `answer_for_offer` maps it to sendonly.
Canonical SDP emits direction and relay related addresses. The secure process's
Opus lookup now uses encoding rather than PT 111. H.264 offer/answer negotiation
selects the first identical common mapping by encoding and clock, using browser
PT 102 in its gate while the existing Amazing session continues with PT 121.

The stripped win-001 artifact reported:

`SDP audio+video bundle=2 payloads=4 candidates=2 rtpmaps=4 ICE=PBBd
setup=actpass; roundtrip ok`.

Known gaps: mDNS names are skipped, not resolved. Codec `fmtp` and RTCP feedback
parameters are still ignored, so packetization-mode/profile compatibility is not
yet answerable. There is no HTTP signaling page, browser-generated answer, live
Chromium ICE check, or cross-implementation DTLS-SRTP packet exchange yet.

Conformance: one observed-Chromium core gate, one dynamic-codec root gate, and
one root artifact gate bring the workspace to 513/513; standalone GPUI remains
24/24. Strict changed-package Clippy and the locked stripped ThinLTO root build
pass on win-001. Release sizes are 8,578,560 bytes (`amazing.exe`) and 14,268,928
bytes (`amazing-shell.exe`).

## Phase 9.45 (Chromium answer synthesis) — fmtp symmetry and supported feedback

Shipped: media sections now own order-independent `RtpFormatParameters` and
`RtcpFeedback` collections keyed by payload type. The parser accepts fmtp up to
1024 non-control bytes and feedback values up to 128 bytes, validates payload
membership and duplicates, and handles Chromium's feedback-before-rtpmap order.
Canonical SDP emits both collections and parses back to the same model.

The H.264 parameter interface provides case-insensitive lookup without exposing
string parsing to callers. Browser answer synthesis accepts only 90 kHz H.264
with packetization-mode 1 and a six-hex-digit constrained-baseline (`42xxxx`)
profile. It copies the offer's exact profile/packetization configuration and
payload ID as required by RFC 6184. Audio retains compatible offered Opus and
PCMU mappings plus Opus fmtp. Feedback is reduced to implemented `nack pli`;
unsupported transport-cc and `ccm fir` are not advertised.

`build_local_answer` retains BUNDLE mids, inverts Chromium recvonly to sendonly,
uses a new local certificate/ICE credential set and host candidate, and returns
a canonical parsed answer. Runnable artifact:

`target\release\amazing.exe sdp-browser-answer
examples\phase9-chromium-offer.sdp`.

The measured answer selected audio PTs 111/0 and video PT 102, emitted
`a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f`,
and emitted only `a=rtcp-fb:102 nack pli` for video.

Known gaps: the model does not yet interpret every codec-specific fmtp field or
generate sprop-parameter-sets. H.264 bytes are not yet RFC 6184 packetized, and
there is still no HTTP signaling page, live browser ICE/DTLS handshake, SRTP sent
to Chromium, or visible browser media proof.

Conformance: the enhanced Chromium parameter gate and browser-answer artifact
gate bring the root workspace to 514/514; standalone GPUI remains 24/24. Strict
changed-package Clippy and the locked stripped ThinLTO root build pass on
win-001. Release sizes are 8,599,040 bytes (`amazing.exe`) and 14,268,928 bytes
(`amazing-shell.exe`).

## Phase 9.46 (RFC 6184 packetization) — Annex-B → single NAL/FU-A RTP

Shipped: new std-only `h264_rtp` module implements H.264 packetization-mode 1
behind one packetization function and one stateful depacketizer interface. It
splits three- or four-byte Annex-B start codes, validates F/NRI/type, emits NAL
types 1–23 directly when they fit, and fragments larger NALs into consecutive
FU-A packets. FU indicator F/NRI is preserved; start/end/type headers are exact,
sequence numbers wrap, every fragment keeps one RTP timestamp, and only the
final packet of the access unit carries marker.

Depacketization validates payload type/SSRC, ascending sequence, stable
timestamp, FU start/continuation/end state, reserved/start+end bits, reconstructed
NAL header, and a caller byte cap. It emits normalized four-byte Annex-B start
codes, drops an access unit on sequence/timestamp damage, and accepts the next
complete marker-delimited unit. STAP/MTAP/FU-B are rejected rather than silently
misparsed.

The real codec gate encodes a deterministic 320×180 OpenH264 keyframe, packets it
under a 1,200-byte RTP payload cap, depacketizes it, and decodes the result back
to 320×180 pixels. The stripped win-001 artifact measured:

`H264 AnnexB=34652 bytes RTP=31 packets FU-A=29 max-payload=1200 size=320x180;
h264 rtp probe ok`.

Runnable artifact: `target\release\amazing.exe h264-rtp-probe`.

Known gaps: packetization-mode 1 supports single NAL and FU-A only; it does not
aggregate SPS/PPS into STAP-A. Interleaved mode, DON, FU-B/MTAP, retransmission,
and congestion-driven MTU changes are absent. The packetizer is not yet wired to
a live Chromium SRTP sender.

Conformance: three core packetization/damage gates, one real-codec gate, and one
root artifact gate bring the workspace to 519/519; standalone GPUI remains
24/24. Strict changed-package Clippy and the locked stripped ThinLTO root build
pass on win-001. Release sizes are 8,614,400 bytes (`amazing.exe`) and 14,268,928
bytes (`amazing-shell.exe`).

## Phase 9.47 (live browser interop) — Edge ICE/DTLS-SRTP/H.264 decode

Shipped: new `browser_peer` module exposes one `run_browser_peer(port)` interface
that owns local signaling, answer generation, secure transport, timed media, and
the browser result. Its HTTP adapter caps headers at 16 KiB and offer/result
bodies at 256 KiB, requires HTTP/1.1 plus exact Content-Length, serves one
same-origin page, and closes every response. A deterministic parser gate covers
exact bodies and oversized rejection.

The page creates recvonly audio/video transceivers, gathers a real Edge offer,
POSTs SDP, installs Amazing's sendonly answer, attaches the remote video track,
and reports only after five `requestVideoFrameCallback` callbacks. It draws the
decoded frame to canvas and returns size/checksum/connection state. Amazing binds
UDP on all local interfaces, selects Edge's literal IPv4 host candidate, answers
with the routed local address, and runs as controlled ICE. The endpoint now
accepts a browser's pre-nomination check before USE-CANDIDATE and accepts
`c=0.0.0.0` when the concrete candidate matches the connected socket.

After ICE nomination and DTLS-SRTP key export, a persistent OpenH264 encoder
sends 45 frames at 10 fps. Every access unit crosses the RFC 6184 packetizer with
a 1,200-byte payload cap and is protected by the negotiated AEAD_AES_128_GCM
SRTP profile. The stripped release session measured:

`BROWSER_INTEROP http=127.0.0.1:8787 browser=192.168.0.3:50061
media=192.168.0.3:65272 PT=102 frames=45 RTP=264 result=decoded frames=5
size=320x180 checksum=4156991624 state=connected; browser peer ok`.

Edge visibly displayed the decoded pixels. Native evidence:
`383ef2dd28d76502359b6f87b7d9873ac46c233b1a5c8d22563c32402c3be780`.

Runnable artifact: `target\release\amazing.exe browser-peer 8787`, then open
`http://127.0.0.1:8787/` in Chromium. The authoritative run used Edge with mDNS
host obfuscation disabled and autoplay enabled.

Known gaps: the live proof is same-host IPv4 and video-only; Edge's mDNS hiding
was disabled instead of resolving `.local` names. Browser audio RTP, external
TURN relay use, trickle updates, renegotiation, long-running RTCP/congestion
control, and automated interactive CI remain production follow-ons. The artifact
closes after the first verified decode report.

Conformance: the HTTP parser gate brings the static workspace to 520/520; the
live Edge gate separately proves cross-implementation ICE/DTLS-SRTP and decoded
media. Standalone GPUI remains 24/24. Strict changed-package Clippy and the locked
stripped ThinLTO root build pass on win-001. Release sizes are 8,649,728 bytes
(`amazing.exe`) and 14,268,928 bytes (`amazing-shell.exe`).

## Later phases (unchanged from goal prompt)

- **3 Layout** block+inline FC, box model, TTF glyph metrics parser.
  WPT css/CSS2 tracking starts here.
- **3.5 Flexbox/Grid** · **4 Paint** software rasterizer + PPM writer ·
  **5 Net** HTTP/1.1 (TLS exception decision due) · **6 JS** Test262-gated ·
  **7 DOM/JS bridge** · **8 GUI shell** (one OS first) · **9 stretch media**
  (images/audio, authenticated ICE/RTP/RTCP, bounded jitter playout, L16/PCMU,
  SDP, secure two-process playout, and D3D11 presentation milestones landed;
  Opus, live loss/concealment, and adaptive playout landed; video, TURN, and
  browser interoperability next).

## Numbered-phase completion audit (2026-08-23, win-001 authoritative)

- **Phase 0:** `target\release\amazing.exe version` reports every workspace
  module; `.github/workflows/ci.yml` and `scripts/ci.sh` run locked all-target
  build/test gates on pushes/PRs. Workspace structure and architecture decisions
  are committed.
- **Phase 1:** release `parse examples\sample.html` produced a 1,528-byte
  indented DOM dump. The 60-test `amazing-html` unit suite covers malformed recovery,
  insertion modes, implicit table structure, foster parenting, and adoption
  agency. Expanded migrated html5lib/WPT DOM-minus-doctype tree measurement is
  243/243 with no failures or skips. Template, broader fragment parsing, and
  structural attribute-namespace gaps mean full parser completion is not claimed.
- **Phase 2:** release `style examples\sample.html` produced a 2,307-byte
  computed-style tree through the HTML→CSS cascade pipeline; selector,
  specificity, inheritance, shorthand, and recovery tests remain green.
- **Phase 3/3.5:** release `layout examples\layout.html 800` produced a
  4,496-byte absolute layout tree. Measured curated WPT gates are 17/17 CSS2
  and 22/22 flexbox/grid. An actual named WPT reftest subset is 3/10; both
  scopes are now labeled separately.
- **Phase 4:** release render wrote 800×804 BMP+PPM artifacts (1,929,654 and
  1,929,615 bytes). An independent win-001 Edge reference, absolute diff, and
  side-by-side visual gate are committed; normalized RMSE is 0.460741 with
  fallback-glyph/subset differences explicitly recorded.
- **Phase 5:** release `fetch http://example.com/ -i` completed a real external
  HTTP/1.1 request with `200 OK` and an 856-byte captured response. The 61-test
  network suite covers loopback framing/redirect/error behavior. HTTPS remains
  `fetch-parse` fed that live body into Phase 1 and dumped 652 bytes of DOM.
  HTTPS remains the documented vetted-TLS follow-on, not a hand-rolled exception.
- **Phase 6:** the repaired release artifact executes Array/Promise plus
  `Object.keys(...).join`, `Set`, and `Map` (`a,b`, `2`, `2`). The root cause
  was `Object.keys` bypassing the shared array builder and therefore missing
  `Array.prototype`; a focused regression now passes. All 143 JS tests pass.
  Fresh Test262 measurement: 641 discovered, 196 passed, 0 failed, 445
  explicitly skipped; supported pass rate 196/196 (100%).
- **Phase 7:** release `domrun examples\phase7.html examples\phase7.js`
  logged `click!! ready red` and dumped the mutated DOM with click state,
  style, dataset, and appended span visible.
- **Phase 8:** the double-clickable release shell, input/history/tabs, event
  loop, external resources, repaint/reflow, and Win32 FFI gates are green. The
  committed `phase8-win001.png` shows the real Windows application. A separate
  final `[D3D11]` title screenshot remains optional proof pending the user's
  acceptance of Windows App's certificate warning; hardware presentation
  itself passed the interactive smoke gate.
- **Phase 9:** forty-seven committed subphases now include decoded media,
  authenticated ICE/consent/TURN, compound RTCP, browser-shaped SDP answers,
  vetted two-process DTLS-SRTP, D3D11/GPUI presentation, Windows audio-master
  playback, RTP loss recovery, RFC 6184 packetization, bounded handoff state, and
  live Chromium H.264 decode. Each subphase records its bounded scope; broader
  WebRTC WPT and production-network coverage remain follow-on work.

Numbered milestone completion is now claimed. The current audit has real
external measurements and runnable artifacts for every numbered phase. Phase 1's original four-file
foundation is 100/100 and its expanded upstream subset is 243/243, but broader
parser modes remain partial; Phase 9 is a stretch milestone with explicit
production gaps. Final workspace count/size are remeasured after the audit fixes
before each commit. Current gate: **520**
hardware-independent workspace tests/doctests pass with `--locked`; the
stripped ThinLTO release is **8,649,728 bytes**.

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
