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
| 6 — JS | 🔄 lexer/parser + tree-walking runtime milestone landed (140 crate tests); focused Test262 slice measured at 196/196 (100%) |
| 7–9 | pending |

Local editing tree: `/Users/jMac/Projects/OpenSource/amazing`.
Server tree: `C:\Users\server\src\amazing` (git lives there).
Sync loop: edit locally → `scp -r` changed paths → build/test via ssh → commit on server.
**No rsync on Windows. No git push from macOS (cmd.exe breaks it) — commit on the server.**

## Phase 1 in-flight — finish these, in this order

### 1. REWRITE `crates/amazing-html/src/tokenizer.rs` (broken, do not patch)

The committed draft is **invalid Rust**: struct fields declared inside an
impl block (`was_end_tag_flag`, `finish_attr_placeholder`), a half-wired
`PendingAttr`, and a mangled `emit_tag`. Delete and rewrite from the design
below. It was never compiled.

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

Not started. This is the biggest piece of Phase 1. Spec: §13.2.6
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

### 6. Report (per goal prompt format) and STOP for confirmation

What's implemented vs deferred, demo command (`amazing parse page.html`),
correctness gaps, then ask before Phase 2. Do not auto-proceed.

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
`cargo test --workspace` is green on win-001 (140 tests in `amazing-js`).

Conformance evidence: the official Test262 repository was shallow-sparsely
checked out on win-001 with 641 tests across the selected classic-language
directories. `python scripts/test262.py C:\\Users\\server\\src\\test262
--binary target\\debug\\amazing.exe` measured 196 passed, 0 failed, and
445 skipped; measured pass rate = **196/196 (100%)**. Skips are explicit
for modules/async/negative tests, BigInt/Symbol/private/accessor/destructuring
features, and unavailable harness hooks. This is a focused slice, not whole-suite
coverage.

Known gaps: there is no browser event loop or Promise timing/job integration
surface yet, and generators, symbols, modules, real Symbol identity, regex
literals, and full class semantics are deferred. Basic
`super` property and constructor calls now work.
The `Symbol.iterator` key bridge, lazy `next()` consumption, and abrupt-body
closing are implemented. The runtime is
a milestone, not Phase 6 exit-criteria completion.

## Phase 7 (DOM/JS bridge) — interactive tracer slice landed (win-001 green)

Shipped: `amazing_js::DomHost` keeps the JS core independent of arena DOM
types while exposing live node wrappers. The driver implements a bounded,
mutation-capable surface: `document.querySelector`/`querySelectorAll`,
`getElementById` (tag/id/class selectors), `document.body`,
`document.createElement`, element `textContent`, `id`/`className`,
`getAttribute`, `setAttribute`, `append`/`appendChild`/`remove`,
`innerHTML`, `document.title`, the `window`/`globalThis` aliases, and
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

Known gaps: selector coverage is intentionally limited to a single simple
selector; styles are represented as inline declarations but do not yet
trigger layout/paint invalidation; there is no HTML parser script scheduling
or browser window/global object yet.

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

The shell compiles and the complete workspace suite passes on win-001. The
interactive window itself requires a Windows desktop session (not available
to the headless SSH service), so visual GUI proof is still pending. Current
Inline page scripts now execute against the live DOM before the first layout
and repaint. Each tab retains its JS runtime; layout-box hit testing dispatches
mouse clicks and repaints DOM/style mutations. Limitations are blocking
navigation during fetch/paint, no native scrollbar thumb, no network event
sources, and no module/async script scheduling. Classic
external `script[src]` resources now resolve relative to the page URL/path and
load through the existing fetcher; `setTimeout` and Promise microtasks use the
cooperative GUI timer pump. `document.addEventListener("DOMContentLoaded", …)`
is dispatched after classic scripts complete; DOM events bubble through arena
parents and expose `currentTarget`, `stopPropagation`, and `preventDefault`.
`window.location.href`/`assign`/`replace` requests resolve relative to the
active tab and enter its history. GUI anchor clicks perform the same default
navigation unless a handler prevents the default. Page `fetch()` is host
mediated and returns a minimal Promise-backed Response (`ok`, `status`, `url`,
`headers.get/has`, `text()`/`json()`); Promise resolution now adopts returned
Promises for normal chains.
External `<link rel="stylesheet" href="…">` resources are fetched and merged
into the author cascade before layout.

## Later phases (unchanged from goal prompt)

- **3 Layout** block+inline FC, box model, TTF glyph metrics parser.
  WPT css/CSS2 tracking starts here.
- **3.5 Flexbox/Grid** · **4 Paint** software rasterizer + PPM writer ·
  **5 Net** HTTP/1.1 (TLS exception decision due) · **6 JS** Test262-gated ·
  **7 DOM/JS bridge** · **8 GUI shell** (one OS first) · **9 stretch media**.

## Server cheat sheet

```
# sync (from mac)
scp -q -r -i ~/.ssh/win001-build-key <local paths> 'server@win-001:C:/Users/server/src/amazing/'

# build/test/commit (on server)
ssh -i ~/.ssh/win001-build-key -o BatchMode=yes server@win-001 ^
  "cd /d C:\Users\server\src\amazing && cargo test --workspace"
git -C C:\Users\server\src\amazing add -A
git -C C:\Users\server\src\amazing -c user.name=server -c user.email=server@win-001 commit -m "..."
```

Known server env: cargo 1.97.1, VS2022 BuildTools, Python312 + LLVM installed,
user env vars LIBCLANG_PATH/PYTHON3 already set. Default ssh shell is cmd.exe
(quote-wrap carefully; PowerShell -EncodedCommand for anything hairy).
