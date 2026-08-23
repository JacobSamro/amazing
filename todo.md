# todo.md — Amazing engine handoff

> Handoff note. Read this top-to-bottom before touching code.
> Authoritative docs: `ARCHITECTURE.md` (locked decisions), `AGENTS.md`
> (build policy: **everything builds/tests on win-001**, never macOS).

## Where the project stands

| Phase | Status |
|-------|--------|
| 0 — skeleton | ✅ done, all exit criteria met on win-001 |
| 1 — HTML parser | 🔄 expanded html5lib/WPT tree subset 243/243; general fragment/template/attribute-namespace gaps remain |
| 2 — CSS | ✅ done on win-001 (88 workspace tests green) — see Phase 2 section |
| 3 — Layout | ✅ done on win-001 (workspace green; curated css/CSS2 subset 17/17) |
| 3.5 — Flexbox | ✅ done on win-001 (13 flex tests; row/column/wrap/grow/shrink/justify/align/gaps/auto-margins/order) |
| 3.6 — Grid | ✅ done on win-001 (16 grid tests; track lists incl. fr/%/repeat+auto-fill, row/column/dense placement, line numbers + negative anchors, implicit tracks, alignment) |
| 4 — Paint | ✅ crate integrated, workspace green (43 paint tests); `render` demo writes BMP+PPM |
| 5 — Net | ✅ crate integrated, workspace green (61 net tests incl. loopback HTTP); `fetch` demo |
| 6 — JS | ✅ tree-walking milestone exit criteria met (143 crate tests); focused Test262 slice 196/196 supported (100%) |
| 7 — DOM/JS | ✅ live DOM exit criteria met: handlers mutate layout/style and repaint visibly end-to-end |
| 8 — GUI | ✅ Windows native shell and interactive desktop exit criteria verified on win-001 |
| 9 — Media | 🔄 22-subphase foundation milestone landed; VP8 encode/TURN/browser interop remain |

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
- **Phase 9:** twenty-two committed subphases provide decoded images/audio,
  authenticated ICE/consent, RTP and compound RTCP, SDP identity, vetted
  two-process DTLS-SRTP, D3D11 presentation, retransmission/role state, and
  computed reception statistics. Each subphase records its bounded scope;
  TURN, codecs/video, full browser interop, and WebRTC WPT remain future
  production work rather than hidden completion claims.

Completion is not yet claimed. The current audit has real external measurements
and runnable artifacts for every numbered phase. Phase 1's original four-file
foundation is 100/100 and its expanded upstream subset is 243/243, but broader
parser modes remain partial; Phase 9 is a foundation with explicit media/interop
gaps. Final workspace count/size are
remeasured after the audit fixes before each commit. Current gate: **496**
hardware-independent workspace tests/doctests pass with `--locked`; the
stripped ThinLTO release is **7,724,032 bytes**.

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
