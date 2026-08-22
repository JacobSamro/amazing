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
| 3.5–9 | in progress (paint/net/js crates being authored) |

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
