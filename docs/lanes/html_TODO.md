# HTML lane decision map

## HTML-1 — XML parser and integrated renderer cascade complete on fixed base

HTML-1's final renderer ticket was gated on branch `lane/html` at fixed base
`a292537252b34b74aee4c4772e732cf7c384f888` after one HTML-only Windows
refresh. IA-XML dispatch and the Git-free runner landed earlier in `2021707`.
No parser, driver, or runner source edit was made in this final ticket. The
integrator's post-merge authoritative gate covers later combined heads; the
HTML lane does not chase unrelated `dev` movement. At the last check,
`git diff --name-only a292537 origin/dev -- crates/amazing-html` was empty.

### Owned parser contract and boundary

- `parse_xml(&str) -> Result<Dom, XmlError>` builds the real DOM with fatal
  well-formedness errors and no HTML recovery. `xml::parse_xml_with_declaration`
  additionally returns XML declaration metadata. The input is decoded Unicode.
- XML QName and namespace binding rules are checked before using DOM's expanded
  name APIs. DOM owns namespace identity and prefix storage; default namespace
  does not apply to unprefixed attributes. The temporary `XmlTreeSink` was
  retired after DOM's namespace APIs landed.
- XML 1.0 declarations, elements, attributes, the five predefined and numeric
  entities, CDATA, comments, PIs, and external PUBLIC/SYSTEM doctype identifiers
  are supported without external loads. XML 1.1, DTD internal subsets, and
  unresolved external-DTD entities remain Unsupported. Limits: 16 MiB input,
  512 open elements, 1024 attributes per element. XML scripts do not execute.
- Driver dispatch is integrator-owned; it calls `parse_xml` for local XML/XHTML/SVG
  extensions and recognized HTTP XML MIME types without HTML fallback. This
  final ticket does not adopt `parse_bytes`, wire Fetch, or expand encoding.

### Exact final render measurement

WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293` was verified by a
revision marker on win-001; no server Git command was used. The unchanged
`tests/wpt-render/cascade-manifest.json` SHA-256 is
`bf3d76b0f4b07e1189fdbb9e3dac8a7897328be394c2fcefcf4503cf240b5343`.
It selects 72 cases by SHA-256 path stratification: 12 each from css-cascade,
selectors, css-values, css-color, css-fonts and css-variables, with no declared
exclusions. Each test and its upstream reference were rendered at 800×600,
scale 1, with WPT font loading through HarfRust 0.13.2 and Swash 0.2.10.

The frozen pre-dispatch source was `aa9cb9be3ca79425bb979519c75b442646b37028`,
executable SHA-256 `10eac183bb83c91b8c6360554dbef939a927ff997fddbbab91d2f7ed668a4355`;
its executable remains at `C:\Users\server\target\html\before-aa9cb9b\debug\amazing.exe`.
The current source was fixed base `a292537`, executable SHA-256 at sweep time
`600f30c52b8f76e97420cc4949ce1915d238a240c3c62492b92010a5a39a32b1`.
The current target executable was subsequently replaced by Cargo test/build
commands; the sweep-time hash and PPM/report artifacts identify this run.
The prior runner excluded XHTML tests and references, while the current runner
attempted them and classified XML failures. Both used the same manifest bytes
and WPT checkout.

| Side, twice stable | Pass | Mismatch | Error | Timeout | Unsupported | Crash | Per-case classification/pixel projection SHA-256 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Pre-dispatch | 35 | 24 | 0 | 0 | 13 | 0 | `33165bc36299847812c95d2f46a75ce5968da1316ab4213b20f3c76c0c57b8b7` |
| Integrated | 38 | 32 | 0 | 0 | 2 | 0 | `fea336c5e6e0f00bd3f08809cb9a982c0913652a40187133776c96dc2fc9f318` |

Both repeats on each side had identical full per-case projections, including
pixel hashes, and all four raw runner reports had 72 cases and `unexpected=0`.
The exact projections and a result summary are in
`tests/wpt-html/html1-cascade-before-projection.json`,
`html1-cascade-after-projection.json`, and `html1-cascade-results.json`.
Raw reports and PPM artifacts are retained under
`C:\Users\server\target\html\diagnostics\html1-final-cascade\`.
The historical 35/24/13 baseline is now freshly reproduced from the named
frozen revision; parser-only 12 parsed/2 Unsupported counts are not render
passes and are kept separate.

All 13 prior exclusions were XHTML-related. Four HTML tests had XHTML
references: `css-cascade__revert-layer-002.html` (now pass), and
`css-cascade__revert-layer-011.html`, `css-cascade__all-prop-002.html`,
`css-cascade__revert-layer-005.html` (now mismatch). Nine XHTML tests were
excluded: `css-color__t425-hsla-values-b.xht` and
`css-color__t421-rgb-clip-outside-gamut-b.xht` (still Unsupported);
`css-fonts__font-family-name-014.xht` and
`css-fonts__font-family-name-001.xht` (now pass); and
`css-fonts__font-family-name-000.xht`, `css-fonts__font-family-name-022.xht`,
`css-fonts__font-family-name-018.xht`, `css-fonts__font-family-name-024.xht`,
`css-fonts__font-family-name-019.xht` (now mismatch). Thus 3 old exclusions
became pass, 8 mismatch, and 2 remain Unsupported. The two colour cases still
need external-DTD entities; `XmlErrorKind::Unsupported` is the honest outcome
because external entities are not loaded. There were no other exclusions.
`use-descendant-combinator-003.html` is outside this selection and gets no
pass credit; the corrected root mapping in the broader IA diagnostic exposed a
page-script TypeError/crash.

### Windows source and test gates

The fixed-base hash manifest `tests/wpt-html/html1-fixed-base-source-hashes.json`
(SHA-256 `b2bc5f20a2b6d2d2d3785736ce77b61964caa51a2f4cc815519f1ca72c591228`)
records 231 tracked Rust, Cargo, runner and selection inputs. All 231 matched
the refreshed Windows HTML tree before and after; local inputs did not drift.
Root `Cargo.lock` SHA-256 stayed
`f4ff57271aa44d67cd1a9d2eee65074b7fb885d8d181cf71b8f738e4a4159fb3`.
All full sweeps and workspace commands held the shared HTML build slot and used
`CARGO_TARGET_DIR=C:\Users\server\target\html` for the current source.

- HTML package nextest: **95 passed / 0 failed / 0 skipped**; separate HTML
  doctest: **1 passed / 0 failed**.
- Workspace nextest: **1321 passed / 0 failed / 3 skipped**; separate workspace
  doctests: **2 passed / 0 failed**.

The HTML-1 final ticket stops here for integrator preflight/merge. It does not
start another ticket. HTML-3's decoded-text streaming API and HTML-2's bounded
byte decoder remain their separately measured decisions below.

## HTML-2 — bounded HTML byte input

The HTML parser now has `parse_bytes(&[u8], Option<&str>) -> Dom` beside the
unchanged `parse(&str) -> Dom`. `encoding::decode` also returns decoded text,
selected encoding and the source of that choice. This is an owned crate seam;
the browser CLI still calls `String::from_utf8_lossy` before `parse`, so product
adoption needs an integrator-owned driver change.

The initial slice uses WHATWG precedence: leading UTF-8/UTF-16 BOM, supported
transport charset label, a 1024-byte HTML meta prescan, then windows-1252
fallback. It recognizes plain `<meta charset>` and `http-equiv=content-type`
with a `content` charset, and handles the UTF-16 XML preamble. A UTF-16 meta
declaration maps to UTF-8 per HTML. Invalid UTF-8 and UTF-16 sequences decode
with U+FFFD; the BOM is consumed. The supported decoders are UTF-8, UTF-16LE,
UTF-16BE and windows-1252. Accepted labels are explicitly enumerated in
`encoding.rs`, including UTF-8 aliases and ISO-8859-1/ASCII aliases that map
to windows-1252. Other labels do not claim coverage. The complete Encoding
label/decoder table and full HTML prescan/reparse algorithm remain future work.

Final HTML-2 gate at `579874a` (base `ffb143b`), after an HTML-only Windows
refresh and source re-sync: `amazing-html` nextest **91 passed / 0 failed /
0 skipped** twice; separate package doctest **1 passed / 0 failed**. Seven
byte-input unit cases are new, compared with the previous measured HTML crate
gate of 84 unit tests and 1 doctest. The ten-case projection of seven new
byte tests plus three existing `parse` tests matched across both final runs,
SHA256 `248db09cc4ef101ccf2ef9fc41707ee5635a07152634dbaf975fe96996a8a3e0`.
Final full workspace nextest **1269 passed / 0 failed / 3 skipped**; separate
workspace doctests **2 passed / 0 failed**. Root lock and both owned source
hashes matched before and after that gate. This count is for the complete
rebased workspace, not an attributed HTML-only workspace delta.

This ticket does not change XML byte handling or the HTML-1 XML cascade. Its
strict XML byte-input idea remains a separate possible ticket after HTML-2.

## HTML-3 — incremental decoded-text parsing

`IncrementalParser::push_chunk(&str)` feeds one tokenizer and one `TreeBuilder`;
`finish()` signals the sole final EOF and returns the DOM. Input chunks must
already be decoded Unicode. A pending CR normalizes to LF across a chunk
boundary, including CRLF. The tokenizer pauses on temporary input exhaustion
and ambiguous `<`, character reference and external-doctype lookahead, then
releases consumed scalar input. It retains the source of a tag under
construction for malformed-EOF literal recovery. Current-token fields such as
an unfinished comment or attribute can still grow with that token.

Co-integrator explicitly released the Windows html tree/target after the
authoritative HTML-2 merged gate. The final HTML-3 source `5bb22d3` was
rebased on dev `2021707`, refreshed only into that tree through the primary
checkout script, and re-synced. The gate compares every scalar-boundary split
and a deterministic multi-chunk schedule against `parse(&str)` for 15 fixtures
covering tags, quoted/unquoted attributes, comments, doctypes, named/numeric
references, RAWTEXT/RCDATA, malformed EOF, CRLF and non-ASCII text. Direct
tests assert deferred ambiguous tokens and a sole final EOF. Repeating 2,000
complete `<p>1234567890</p>` chunks kept the retained tokenizer input at no
more than 12 scalars after each push; this excludes the growing DOM and fields
of a token still under construction.

Final win-001 evidence at dev `2021707`: package nextest **95 passed / 0
failed / 0 skipped** twice, four HTML-3 test-name projection SHA256
`7a0785e40fbc66216c9e18030526191181ab2ba035ce5fa43f0d2c86891086e9`
both times; separate package doctest **1 passed / 0 failed**. The prior
authoritative HTML-2 HTML crate gate had 91 unit tests and one doctest, so
HTML-3 adds four passing unit tests. Final workspace nextest **1321 passed /
0 failed / 3 skipped**; separate workspace doctests **2 passed / 0 failed**.
`Cargo.lock` and all four owned source hashes matched before/after the
workspace gate. The workspace total includes contemporaneous merges from
other lanes and is not an attributed HTML-only delta. A previous green gate
at dev `ed6a44c` was superseded when dev advanced mid-gate. No byte-stream
encoding or driver change is included.

The pinned sample has two measured DTD-entity blockers and no UTF-8 input
failures among 14 files. A bounded XHTML entity policy is a separate decision;
do not silently use HTML's entity table or fetch arbitrary external resources.

## HTML-4 — bounded incremental HTML byte input

`IncrementalByteParser::new(transport_label: Option<&str>)` fixes the optional
out-of-band charset before input begins. Repeated `push_chunk(&[u8])` calls
accept arbitrary byte boundaries; consuming `finish()` signals the only EOF
and returns `Dom`. The bridge uses the same `encoding::sniff` decision as
`parse_bytes`, then feeds decoded text to HTML-3's `IncrementalParser`.
No text is emitted until either the first 1024 bytes have arrived or `finish`
is called. Thus the complete HTML-2 prescan has its exact available prefix,
including unfinished comments or meta attributes. BOM wins over a supported
transport label, then the UTF-16 XML preamble, then the first supported meta,
then windows-1252 fallback. An unsupported label is ignored.

After each push, retained undecoded input is at most 1024 bytes before the
choice, or at most three partial UTF-8 bytes, or a UTF-16 odd byte and one
pending high surrogate afterward. Decoded text is passed in batches capped at
4099 UTF-8 bytes (a 4096-byte flush threshold plus one scalar) and cleared
after each push. `buffered_input_bytes()` and
`buffered_input_scalars()` expose retained input for checks. A single unfinished
HTML token's decoded fields, any tokenizer lookahead needed for that token,
and the growing DOM are outside this consumed-input bound. The caller owns
the supplied chunk; it is not copied wholesale. CR/CRLF normalization remains
in the text parser and therefore crosses byte and scalar chunk boundaries.

The deliberately non-WHATWG HTML-2 behavior remains: only four decoders and
an enumerated label subset are supported; unknown labels fall through, and
there is no full HTML Encoding prescan/reparse algorithm. The existing
UTF-16 XML-preamble sniff and windows-1252 default are preserved exactly.

The fixed HTML-4 Windows base was `da7f6c8`. After an HTML-only refresh and
source re-sync, 250 Rust source, Cargo manifest and root lockfile SHA-256
checks matched before and after the gates (manifest SHA-256
`4a2a13aac87803f51161f88ecf7880d1d2f4a6e5ae0066686dfc9e80ca004daa`).
Package nextest passed **98/98 twice**, with a separate **1/1** doctest.
Shared-slot workspace nextest passed **1419/1419**, with 3 skipped; separate
workspace doctests passed **2/2**. The 3 new byte-stream tests account for
the package increase from HTML-3's measured 95 tests. This ticket supplies
the byte API only; later driver streaming integration is owned elsewhere.

## HTML-5 — deterministic parser consistency fuzz

The owned `fuzz_tests.rs` harness uses xorshift64 with recorded seed
`0x8a41d72ec09b6153`. It generates 256 cases of at most 192 bytes from
HTML token fragments and bounded truncate, byte replacement, deletion and
insertion mutations. The fragments cover insertion modes, RAWTEXT/RCDATA,
references, malformed EOF, CRLF, meta declarations and bad UTF-8. It also
injects BOM and transport conflicts, plus valid and truncated UTF-16LE/BE
surrogates. Eight longer cases cross the 1024-byte sniff boundary.

Every case compares `parse_bytes` with whole-input, one-byte, uneven 2/7/3/11
and 17/5 byte chunk schedules; the first 24 generated cases also check every
two-chunk byte split. The harness asserts the retained undecoded byte bound
after every push. Three literal expected DOM dumps check basic element,
entity and CRLF behavior independently of the complete/streaming comparison.
This is a consistency property and panic/termination stress test, not a WPT
or independent parsing conformance score. A failure prints the seed, case,
bytes and schedule or split so its input can be minimized and kept as a fixed
regression fixture.

Final HTML-5 fixed base `133db58` includes the primary's `Throw::describe`
hotfix. After an HTML-only refresh and re-sync, 254 Rust source, Cargo
manifest and root lockfile hashes matched on Mac and win-001 before and after
the gates (manifest SHA-256
`55273a0f062d4d5ef5985dffe58eb5cf65fd09ff06b0b37986fcf6f81d61476f`).
Package nextest passed **100/100 twice**, with **1/1** separate doctest.
Shared-slot workspace nextest passed **1438/1438** with 3 skipped, and
separate workspace doctests passed **2/2**. The two new harness tests account
for the package increase from HTML-4's 98. On the earlier fixed base
`cfb13a9`, three driver/JSRT-2 thrown-error-text tests failed in a full
workspace sweep; the primary's hotfix superseded that result. The harness
found no HTML mismatch or panic, so there is no new minimized fixture.

## HTML-6 — bounded fragment context initialization

Before implementation, `parse_fragment` returned the exact error
`unsupported HTML fragment context` for `textarea`, `title`, `style`,
`script`, `select` and `option`; this was asserted on win-001 at fixed base
`262f211`. The predetermined tree fixtures are retained in the lane's
`html6-fixtures.md` scratch note. This ticket adds `textarea` and `title`
with initial RCDATA tokenization, `style` with initial RAWTEXT tokenization,
and `select` with a bounded select insertion mode. `option` uses in-body
insertion mode with the context kept off the open-element stack. The new
contexts extract children from the synthetic `html` root; old table/body
contexts retain their existing wrapper path. A fragment context's own end tag
is not appropriate because its start tag was never tokenized; the literal
text is retained. The tokenizer now replays the source spelling for a
mismatched RAWTEXT/RCDATA end tag, including mixed case, instead of replaying
its lowercased tag builder.

`script`, `template`, `noscript` and `plaintext` remain explicit errors.
In particular, script data tokenization is absent and RAWTEXT is not a safe
replacement for it. The select mode covers option/optgroup insertion,
end-tag recovery, character references and ignored disallowed tags; broader
fragment contexts remain future work. No Encoding expansion was made.

Fixed-base `262f211` HTML-only Windows gates: package nextest **105/105
twice**, separate package doctest **1/1**, shared-slot workspace nextest
**1451/1451** with 3 skipped, and separate shared-slot workspace doctests
**2/2**. A 256-input Rust source, Cargo manifest and root lockfile hash
manifest (`e118de499146db29439521fe1461e045ecfdd1f5e8f7d77039568dbe62be16ad`)
matched on Mac and win-001 before and after the gates. Five new fragment
tests account for the package increase from HTML-5's 100 tests. The earlier
baseline error selection is retained in `.agents/html/html6-baseline.log`;
its independent expected trees were fixed in `html6-fixtures.md` before
implementation. No WPT conformance claim is made for other contexts.

## HTML-7 — script-data fragment parsing

The tokenizer now uses the WHATWG script-data states for ordinary HTML script
start tags and for a `script` fragment context. This includes escaped and
double-escaped transitions, case-insensitive `script` delimiters, appropriate
end-tag matching, source-spelling recovery for mismatched candidates, including null
reconsumption and replacement, and EOF recovery. A fragment's script context is virtual: its
start tag was never tokenized, so even a matching `</script>` is literal text.
The script fragment children are extracted from the synthetic `html` root.
`template`, `noscript` and `plaintext` fragment contexts remain unsupported.
No Encoding or driver behavior changes.

The fixed fixture oracles were recorded before implementation in
`.agents/html/html7-fixtures.md`; the initial Windows baseline selected the
unsupported-context test (1 passed, 104 skipped). The implementation adds four
unit tests, including document token regressions and every-scalar-boundary
streaming comparisons. At fixed base `262f211`, a 256-input Rust/Cargo/lockfile manifest
(SHA-256 `8fcafa12a0a2c865fdd32ae552a4b866bbfb574ad1d66da06c14fdb9bb9541a7`)
matched the HTML Windows tree before and after the gates. Package nextest
passed **109/109 twice**, package doctests **1/1**, shared-slot workspace
nextest **1455/1455** with 3 skipped, and separate shared-slot workspace
doctests **2/2**. Four new unit tests account for the package increase from
HTML-6's 105. No push or PR; the lane stops for co-integrator preflight.

## HTML-8 — PLAINTEXT tokenizer and fragment context

The tokenizer now enters the PLAINTEXT state after an HTML `plaintext` start
tag, retaining all subsequent `<` and `&` characters literally through EOF.
Null becomes U+FFFD; the existing parse and incremental preprocessing handles
CRLF. The tree builder remains in its insertion mode, as required by the
HTML parsing rules. A `plaintext` fragment starts directly in the same state
with a virtual context off the open-element stack, so its apparent end tag is
text. The fragment returns children of the synthetic `html` root.

The exact pre-change Windows error and two ordinary-document trees, plus
independent expected trees, were recorded in `.agents/html/html8-fixtures.md`
before implementation. Three added unit tests assert two changed document
trees, fragment literal/null/CRLF/EOF behavior, and incremental equivalence
at every scalar split and a multi-chunk schedule. This bounded work does not
claim support for other fragment contexts. `template` and `noscript` remain
explicitly unsupported. No driver, DOM or Encoding implementation changed.

At fixed base `21031f1`, a 279-input Rust source, Cargo manifest and root
lockfile hash manifest (SHA-256
`a31db9834484689a28d5bfed8e4a02811c0c464206c2b07582bbad58628d2d7d`)
matched on Mac and Windows before and after the gates. Package nextest passed
**112/112 twice**, package doctest **1/1**, shared-slot workspace nextest
**1529/1529** with 3 skipped, and separate shared-slot workspace doctests
**2/2**. Three tests account for the package increase from HTML-7's 109.
No push or PR; the lane stops for co-integrator preflight.

## HTML-9 — CDATA tokenization in foreign content

The tree builder now feeds its adjusted current node's namespace to the
tokenizer after each token. A complete `<![CDATA[` declaration switches to
CDATA section, bracket and end states only while that node is outside the
HTML namespace. Ordinary HTML content still recovers it as a bogus comment.
The delimiter `]]>` ends the section; `<` and `&` are literal, null becomes
U+FFFD, and pending brackets are emitted at premature EOF. Streaming input
waits for a partial CDATA opener, then preserves bracket/end state across
chunk boundaries.

The exact Windows baseline token and seven DOM trees, together with independent
expected trees, were fixed in `.agents/html/html9-fixtures.md` before the
implementation. SVG and MathML foreign nodes now receive CDATA text; an SVG
`foreignObject` or MathML `mtext` integration point itself is still foreign
and permits CDATA, while its HTML child takes bogus-comment recovery. This is
a bounded tokenizer/namespace seam, not a claim of complete foreign-content
parsing. Existing foreign-attribute adjustments remain incomplete.

At fixed base `f3d2e3f`, a 279-input Rust source, Cargo manifest and root
lockfile hash manifest (SHA-256
`eea90bc5b2a41aa9901c6b12b3de5f3c6d417a8094f016a3f36dfb5c8ab4800e`)
matched on Mac and Windows before and after the gates. Package nextest passed
**117/117 twice**, package doctest **1/1**, shared-slot workspace nextest
**1534/1534** with 3 skipped, and separate shared-slot workspace doctests
**2/2**. Five tests account for the package increase from HTML-8's 112.
No push or PR; the lane stops for co-integrator preflight.

## HTML-10 — foreign attribute namespace adjustments

Foreign-element insertion now applies the WHATWG eleven-entry foreign
attribute table after the existing SVG camelCase and MathML `definitionURL`
adjustments. Listed `xlink:*`, `xml:*`, `xmlns` and `xmlns:xlink` attributes
use DOM's expanded-attribute storage with their exact namespace URI, prefix,
local name, qualified name and value. Other colon-bearing names stay in the
null namespace. The same table runs for SVG and MathML roots and descendants;
HTML children of foreign integration points retain ordinary null-namespace
attribute names.

The fixed-base Windows baseline in `.agents/html/html10-fixtures.md` records
exact null-namespace storage and absent expanded lookups before implementation,
plus independent expected identities for the table, nested foreign nodes,
HTML children and unrelated colon names. DOM `attrs_of`/`dump` exposes only
null-namespace attributes; namespaced values are checked through
`attribute_ns`/`get_attribute_ns`. This follows the existing DOM storage seam
without edits to DOM, bindings, driver, manifests or lockfile. Two new tests
assert actual expanded attributes, while the prior SVG/MathML casing test was
updated to check the XLink expanded identity.

At fixed base `a3d973e`, a 292-input Rust source, Cargo manifest and root
lockfile hash manifest (SHA-256
`a9fb10c9f81bf73c3bfc8514969d836cf0255455b2cb518d920a577f7f4fe0d9`)
matched on Mac and Windows before and after the gates. Package nextest passed
**119/119 twice**, package doctest **1/1**, shared-slot workspace nextest
**1564/1564** with 3 skipped, and separate shared-slot workspace doctests
**2/2**. Two new tests account for the package increase from HTML-9's 117.
No push or PR; the lane stops for co-integrator preflight.

## HTML-11 — bounded noscript fragment context

`parse_fragment("noscript", …)` now starts the tokenizer in RAWTEXT and uses a
virtual context under the synthetic `html` root. The context is not on the
open-element stack, so even an appropriate `</noscript>` remains literal
fragment text. `<` and `&` stay literal; null becomes U+FFFD; CRLF and lone CR
normalize to LF; incomplete end-tag candidates recover at EOF. `template`
remains rejected. The parser's scripting flag is fixed enabled: this ticket
does not claim scripting-disabled `noscript` behavior or alter document tree
construction.

Independent expected trees and exact pre-edit source/lockfile hashes are in
`.agents/html/html11-fixtures.md`. The fixed-base Windows probe showed the
`unsupported HTML fragment context` error for all five selected inputs and
the existing document RAWTEXT behavior before implementation. Focused tests
assert fragment trees independently and compare streaming RAWTEXT output at
every Unicode scalar boundary and across multiple chunks inside an end tag.

At fixed base `6481c90`, the HTML-only Windows tree was refreshed again after
the code was complete and owned files were re-synced. A 293-input Rust source,
Cargo manifest and root lockfile SHA-256 manifest (`8ef6c6e20410125e9c2c08910d1fb8f4390115aa3e8d998ecd15f50ae053f12c`)
matched on Mac and Windows before and after the gates. Package nextest passed
**121/121 twice**, and the separate package doctest passed **1/1**. Shared-slot
workspace nextest passed **1591/1591** with 3 skipped, and separate shared-slot
workspace doctests passed **2/2**. Two new tests account for the package increase
from HTML-10's 119. No push or PR; the lane stops for co-integrator preflight.

## HTML-12 — pending table-character tokens

The tree builder now buffers adjacent character tokens in the `in table text`
insertion mode. On the next non-character token or EOF it tests the complete
run: whitespace-only text stays under the current table-family element; a run
with any other scalar is passed once through the in-body rules with foster
parenting, so all characters land before the relevant table in source order.
It then reprocesses the trigger in the saved table, tbody, or tr mode. A single
combined run preserves adjacent-text coalescing. No template, scripting,
quirks, tokenizer, DOM, driver, or manifest behavior was widened.

Independent DOM tree oracles and pre-edit source/lock hashes are in
`.agents/html/html12-fixtures.md`. The fixed-base Windows probe selected one
test (1 passed, 121 skipped) before implementation. Splitting `<table> A`
after the space gave `"A"` before the table and `" "` inside it, whereas
one-shot parsing gave one `" A"` node before the table. Tbody, nested-table
and EOF examples also differed. The exact trees are in `html12-baseline.log`.
Two new tests assert table, tbody, tr, nested-table, reference, whitespace-only,
EOF and coalescing trees, every scalar split, and multiple chunks within a
character reference and across non-ASCII scalars.

The fixed final base was `bbec0c9`. After an HTML-only Windows refresh and
source re-sync, a 305-input Rust source, Cargo manifest and root lockfile
SHA-256 manifest (`edf1911bed55c7ed291ffefcd0c02ebb088afdcf6a83013ca2f5b7fc61b802fc`)
matched on Mac and Windows before and after the gates. Package nextest passed
**123/123 twice**, and separate package doctest passed **1/1**. Shared-slot
workspace nextest passed **1631/1631** with 3 skipped; a separate workspace
doctest command under the same slot passed **2/2**. The two new unit tests
account for the package increase from HTML-11's 121. No push or PR; the lane
stops for co-integrator preflight.

## HTML-13 — preserve DOCTYPE public and system identifiers

The HTML tokenizer now uses the WHATWG DOCTYPE public/system states instead of
an all-at-once scanner. It preserves missing versus present-empty identifiers,
quoted values, U+FFFD for null, and partial values on abrupt `>` or EOF.
Malformed missing quotes and missing identifiers set the token's force-quirks
flag; unexpected trailing data after a closed system identifier enters bogus
DOCTYPE without forcing quirks. The initial tree mode appends the existing DOM
`DocumentType` node before `html`, with empty strings for absent identifiers,
while preserving the legacy `dom.doctype` name. XML parsing, DTD loading and
full document quirks-mode behavior are outside this change.

Independent expected triples and a fixed-base Windows pre-edit probe are in
`.agents/html/html13-fixtures.md` and `html13-baseline.log`. The probe passed
1 test with 123 filtered, showing all 14 cases lacked a DOM doctype node and
lost identifiers. Focused tokenizer, parser and character-stream tests now
check the identifier values, token force-quirks flags and DOM child order.

At fixed base `58dd9cebe9d9f7be173bab9c7d73f3d68999f7b7`, the HTML-only
Windows tree was refreshed again and owned files were re-synced. A 314-input
tracked Rust source, Cargo manifest and root lockfile SHA-256 manifest
(`b27370cd0b342d2b85893e98129e2663d203c20f56c2a5e30c9d73df85588ac5`)
matched on Mac and Windows before and after the final gates, with zero
mismatches. Package nextest passed **126/126 twice**, separate package doctest
**1/1**, shared-slot workspace nextest **1647/1647** with 3 skipped, and
separate shared-slot workspace doctests **2/2**. Three new tests account for
the package increase from HTML-12's 123. No push or PR; the lane stops for
co-integrator preflight. DOM-14 remains a separate queued ticket.

## HTML-14 — parser custom-element construction seam

At fixed base `859bb53` (DOM-14 merged), the document parser accepts a DOM with
preinstalled definitions and a `ParserCustomElementHost`. Its create-element
path allocates the HTML element with start-tag attributes first, then checks
DOM-14's document registry for an autonomous definition or a matching
customized built-in `is` definition. The host receives the element, document,
namespace, local name, definition and constructor root while the element is
detached and `Precustomized`. The host is handed the mutable DOM directly, so
the parser retains no borrow into the arena while it calls the constructor.
It reports whether construction returned the same element; DOM-14's
`custom_element_super` and `finish_custom_element_upgrade` own the construction
stack and state transitions. The parser attaches the element before finish so
DOM queues initial observed-attribute and connected reactions in that order.
A reported failure marks the element `Failed` and parsing continues. A missing
host or definition leaves an ordinary `Undefined` element eligible for later
DOM upgrade. The parser neither stores another registry nor executes JS.

Frozen before/expected trees and states are recorded in
`.agents/html/html14-fixtures.md`. Mock-host tests check nested synchronous
construction order, attributes at callback time, detached-to-connected
transition, custom versus undefined versus failed states, customized `is`,
initial attribute/connected reactions, descendant disconnection order,
reentrant DOM mutation, and ordinary document/fragment tree stability.
The embedding still must connect the parser host to real JS constructors,
report constructor exceptions, and call `deliver_custom_element_reactions`
after parser tasks. Mock tests grant no WPT credit. Declarative shadow DOM and
template work remain separate.

The fixed-base Windows tree was refreshed from `859bb53` and the owned source
files re-synced before gating. The final 319-input Rust source, Cargo manifest
and root lockfile SHA-256 manifest (`4581e733a8ae549854ea18355a47f0c2847b95fe0ba1da8bc5efa2f34b61be2f`)
matched on Mac and Windows before and after the gates with zero mismatches.
Package nextest passed **131/131 twice**, with a separate package doctest
**1/1**. Shared-slot workspace nextest passed **1667/1667** with 3 skipped,
and separate shared-slot workspace doctests passed **2/2**. Five new parser
mock tests account for the package rise from HTML-13's 126. No push or PR;
the lane stops for co-integrator preflight.

## HTML-15 — WHATWG Encoding codec from pinned indexes

At fixed source base `d2a610ede8f254b8da1be9121d59dbd7cf598e78`,
`amazing-encoding` now contains std-only, unsafe-forbidden streaming decoders
and encoders for the Encoding Standard's UTF, single-byte, CJK, replacement and
x-user-defined families. The checked-in generator verifies WHATWG encoding
revision `a985b62a9b45c17da3e17a9f0a0b4e30c34c4a8a` input hashes:
`indexes.json` `b7c8961095f0b4cae8f4a16a5011e7c84047a60c1277e28ee8df9e56074ee509`
and `encodings.json`
`078212b3697f60b81225b6671bd9da2604497abff0aa8e96e4d46605c02ac9e7`.
`tools/generate.py --check` reproduced all three generated tables. Reverse
pointer exceptions for Big5 and Shift_JIS and gb18030 range/compatibility
cases are handled by the encoder. The HTML parser and incremental byte parser
now share this codec and the existing BOM, transport, meta and fallback
selection. Their public `parse_bytes` entry and four formerly supported
encodings remain covered by existing tests. The primary explicitly granted
one `amazing-encoding` dependency line in root `Cargo.lock`; no other lock
entry changed. Fetch, bindings and the browser driver remain separate owner
adoptions.

Pinned WPT `encoding/` revision is
`53bc812da2ed81179ebd1ba76b2d197cc4747d12`. The frozen data selection
hash is `a4dbb0faa4784085c56321d864a63acaf8c15c897a09c3bccd13a0ef95ea3cc3`;
the checked-in TSV hash is
`a78116be18ae160882afbd2e127aabdd99102aea125a63ee0bc1629eaae7436f`.
It ports 222 label cases and 27 single-byte index vectors. Two Windows BEFORE
runs had identical raw hash
`bfbe5065da36561c2f374e3f3ef94ec64bef537f30b96c6e40b3a52161352f9e`;
two AFTER runs had identical raw hash
`30c71ff13e8fb2f7d3ca97ac016b195983a055eaf71c20d98fd55f42ff5ab932`.

| Selection | BEFORE pass/fail/unsupported | AFTER pass/fail/unsupported |
| --- | --- | --- |
| Labels (222) | 25 / 0 / 197 | 222 / 0 / 0 |
| Single-byte vectors (27) | 1 / 0 / 26 | 27 / 0 / 0 |

The exact 249 case transitions are in
`crates/amazing-html/evidence/html15-wpt-results.tsv` (SHA-256
`c72492a5b830a9e976287bfa7e06b6a1c90728a533f1efa356da44cb039d3473`):
223 unsupported-to-pass, 26 pass-to-pass, zero losses. This is a ported data
selection through the HTML bridge, not unmodified WPT harness credit. Owned
vectors cover all codec families, each single-byte encoder index, every-byte
splits, malformed replacement/fatal behavior, labels, BOMs, compatibility
exceptions, and bounded decoder state. They do not measure Fetch or
`TextDecoder` adoption.

The final 18-file source/input/output/fixture/lock hash manifest in
`crates/amazing-html/evidence/html15-source-hashes.json` was identical
before and after the gates
(SHA-256 `5f43b398eb1096b04e5af3f6d26d12e4409e4584f7cd3c3fc961ca06b4f10f89`),
with zero Mac/Windows mismatches before and after the fixed-tree gates.
The final locked Windows scoped nextest ran **137/137 twice** (encoding 6,
HTML 131); separate doctests ran **0 encoding, 1 HTML**, all passing. The
shared-slot workspace nextest ran **1694 passed, 3 skipped**, with separate
workspace doctests **2 passed**. The lane stops at this ticket for
co-integrator preflight; no push or PR.

## HTML-16 — template contents in the tree builder

Implemented on the fixed combined HTML-15/DOM-template base `6dff430ce81ff5c41f272d6e82491bc9fcb32c41` (primary-reported pushed combined base `582a6af`, gated at `6dff430`). The parser now creates persistent inert contents for each HTML template, redirects the insertion place to that fragment, tracks nested template insertion modes, and closes templates at explicit end tags or EOF. Fragment parsing accepts a template context. Copying the parser's fragment result preserves nested template contents and their inert document ownership. Declarative shadow DOM and the JS constructor adapter remain separate work.

Pinned WPT resource revision: `53bc812da2ed81179ebd1ba76b2d197cc4747d12`. `crates/amazing-html/evidence/html16/upstream.json` holds five exact resource paths and hashes; `selection.tsv` adds seven focused tree cases. This probe ports reachable parsing assertions and does not claim unmodified WPT harness credit. The two BEFORE runs were identical: **1 pass, 10 fail, 0 timeout, 1 unsupported** (SHA-256 `2e33d5ee84d6833a8fe0cae872d42d3509091126cdf6002ef3c630ce32f04c8b`). Two AFTER runs were identical: **12 pass, 0 fail, 0 timeout, 0 unsupported** (SHA-256 `02922e1d38a28044d3e97cfc27b06400acef623dfdaa2f24da8e415898f36783`). Exact transitions: 10 fail-to-pass, 1 unsupported-to-pass, 1 pass-to-pass, zero losses. The first manifest draft treated an upstream empty template's whitespace as no content and missed a trailing LF in the EOF fixture. Those two expected predicates were corrected to match their pinned input bytes; both remained baseline failures because the baseline had no parsed template element. The raw per-case output hashes are in `before-*.tsv` and `after-*.tsv`.

Owned tests cover inert contents, nested templates, EOF recovery, a template fragment context, table placement, serialization, clone/adopt, and incremental parsing. The locked Windows `amazing-html` nextest gate passed **135/135 twice**; the separate doctest passed **1/1**. The fixed-base workspace nextest passed **1722, with 3 skipped**; separate workspace doctests passed **2/2**. The 20-file local/Windows hash audit had zero mismatches before and after the workspace gates; the full report is in `.agents/html/report.md`.

## HTML-17 — parser construction and declarative shadow roots

Fixed base `15147471daf91fafa09a9bcd5e49d635da5945b3` (the pushed HTML-16 head). The parser now selects custom-element definitions from the intended parent's owner document, invokes the embedding host's constructor on a detached element before token attributes or children exist, then appends attributes, finishes the upgrade, and attaches the element. This orders initial observed attribute reactions before connection reactions. Constructor failure leaves a failed element and parsing continues. An inert template document has no live registry, so its elements remain eligible for later upgrade. The streaming parser accepts the same backend-neutral constructor host.

For document parsing, a valid `shadowrootmode` template attaches a DOM-owned root to its host through `attach_shadow`, passing mode, delegates-focus, clonable, serializable, and slot-assignment options. Its children parse into that root; ordinary invalid, repeated, and fragment templates retain the HTML-16 inert contents path. Nested roots and incremental EOF/chunk recovery are covered. DOM-18 owns the still-missing declarative/registry flags and shadow-root serialization; the routed request is in `.agents/html/questions.md`.

Pinned upstream WPT revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12` and exact file hashes are in `crates/amazing-html/evidence/html17/upstream.json`. The nine-case `selection.tsv` is a ported parser probe, not unmodified WPT harness credit. Two Windows BEFORE rounds were identical: **2 pass, 7 fail, 0 timeout, 0 unsupported**, raw SHA-256 `ebaac0676d6623f97b9956a237e63a1c0fe36632d4d6b5380f180cbc6619464f`. Two Windows AFTER rounds were identical: **9 pass, 0 fail, 0 timeout, 0 unsupported**, raw SHA-256 `d2609dab1a6e7f4564c645fbfb173190c667ec8c7452a912d4b7c4251c401e03`. The exact cases in `transitions.tsv` show seven fail-to-pass, two pass-to-pass, and zero losses. The pinned opt-in upstream resource named by the original selection was added to `upstream.json` after the baseline; no selected fixture or probe predicate changed. The 25-file source/fixture/lock audit matched the Windows tree before the final gate with zero mismatches. Linux iteration passed 141/141 package nextest tests, 1770 workspace nextest tests with two skipped, and two workspace doctests.

The fixed-base Windows final gate used Cargo.lock SHA-256 `30c1b66612223068a45b4f9429f7921b60ef45aae7ed4cdfa029d8610506d25a` before and after. Package nextest passed **141/141**, package doctest **1/1**, workspace nextest **1773/1773 with 4 skipped**, and workspace doctests **2/2**. The same 25-file Mac/Windows hash audit had zero mismatches after the gate. `windows-final-gate.log` records the complete run. The lane stops for co-integrator preflight; no push or PR.

## HTML-18 — frameset tree-construction conformance

Fixed base `4020079470872d00b490279672e5c18d47818d2f` (pushed HTML-17 head). The tree builder now replaces an implied body with a frameset while the frameset-ok flag is still true, retains the body after explicit `<body>` or disqualifying content, preserves that flag across whitespace in HTML and foreign content, and disables it for the relevant text and void elements. It also discards leading whitespace before the implied head and places trailing whitespace after `</html>` in a frameset document. Chunk-split and fragment regressions cover the changed behavior.

The frozen 20-case ported tree probe draws exact expected trees from WPT `html/syntax/parsing/resources/tests19.dat` and `webkit01.dat` at revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12`; `evidence/html18/manifest.json` records source and fixture SHA-256s and selection rationale. This is not unmodified WPT harness credit. Two Windows BEFORE rounds matched byte-for-byte at **10 pass, 10 fail, 0 timeout, 0 unsupported** (raw SHA-256 `914cfad980c2bf4e73a7301221eedb971ef1a50fbcfbe9b33bdd6274b071080e`). Two AFTER rounds matched byte-for-byte at **20 pass, 0 fail, 0 timeout, 0 unsupported** (raw SHA-256 `7a91d30b0e58f0bad4bf925833432fd07d35ada851586d557879b8fd232a7f32`). Exact transitions are in `transitions.tsv`: ten fail-to-pass, ten pass-to-pass, zero losses. An exploratory 538-case pinned Linux tree scan changed only those same ten classifications, with no losses. Linux iteration passed 143/143 package nextest tests, 1793 workspace nextest tests with two skipped, and 2/2 workspace doctests. The 49-file source/fixture/lock audit matched Windows before the final gate.

The final locked Windows gate ran on win-002 from a fresh copy of this fixed-base worktree. Cargo.lock SHA-256 was `04b4c609b18e2f8cf87a81515a72e1a3b063fba07593e89ef08212a6fc110f74` before and after. Package nextest passed **143/143**, package doctest **1/1**, workspace nextest **1796/1796 with 4 skipped**, and workspace doctests **2/2**. The 49-file source/fixture/lock audit matched both before and after; `evidence/html18/windows-final-gate.log` records the run. The lane stops for co-integrator preflight; no push or PR.

## HTML-19 — after-body tail recovery

Fixed base `7afb7b74c63289231c526c988fd951a16d77d888` (merged HTML-18). The after-body and after-after-body insertion modes now retain whitespace and comment placement until an unexpected non-whitespace token switches the parser back to in-body mode. Comments following that recovery enter the body, while comments after a later explicit `</html>` remain at the document root. Document, fragment, and every scalar chunk boundary have independent regression assertions.

The frozen 20-case ported tree selection comes from WPT `html/syntax/parsing/resources/webkit01.dat` at revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12`; `evidence/html19/manifest.json` records its source and exact input/expected hashes. It is not unmodified WPT harness credit. Two Linux BEFORE rounds were identical at **17 pass, 3 fail, 0 timeout, 0 unsupported**, SHA-256 `d55647534fdf7f33f66cf280a73a04f89482c096058744db3b621a02c6cd0265`. Two Linux AFTER rounds and two Windows AFTER rounds matched byte for byte at **20 pass, 0 fail, 0 timeout, 0 unsupported**, SHA-256 `3a5e488356a796984c1cd8f9ba2cf6e2810f064f933984364055a4546f2637b6`. `transitions.tsv` records three gains, 17 retained passes, and zero losses. Linux package iteration passed **146/146** nextest tests and **1/1** doctest.

The fixed-base win-001 final gate passed package nextest **146/146**, package doctest **1/1**, workspace nextest **1855/1855 with 5 skipped**, and workspace doctests **2/2**. The 72-file source/fixture/lock audit matched before and after, and Cargo.lock SHA-256 stayed `db334679e89fc127e27d712506d8918dadad629da0d3b7ff1587d59699c7d9e1`. Full output is in `evidence/html19/windows-final-gate.log`. DOM-18 parser flag and serialization adoption remains a separate follow-up.

## HTML-20 — DOM-18 declarative shadow attachment adoption

Fixed base `c4653e66f08087bcb88d2ebd6d5f42a3479f643d` (merged HTML-19). For valid parser-created declarative templates, `amazing-html` now calls DOM-18's `attach_declarative_shadow` and passes the presence of `shadowrootcustomelementregistry` with the existing mode, focus, clonable, serializable, and slot-assignment options. DOM owns the declarative, available-to-ElementInternals, and registry-null state. Template insertion modes and child placement remain intact; invalid, repeated, and fragment templates remain inert. Owned tests cover open/closed serialization through `serialize_children_with_shadow`, nested roots, the registry flag, EOF, and every scalar chunk boundary.

The frozen 12-case selection is ported from pinned WPT `shadow-dom/declarative/` basic, attachment, serialization, and opt-in resources at revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12`. `evidence/html20/manifest.json` records the four upstream source hashes and exact input/expected hashes. These are focused parser probes, **not** unmodified WPT harness credit. Two Linux BEFORE rounds matched at **3 pass, 9 fail, 0 timeout, 0 unsupported**, SHA-256 `64ca0d98721c4c9b8d1762d5ee1c68e3ecc419a6aa3a75f5b1c0673236d8df87`. Two Linux AFTER and two Windows AFTER rounds matched byte for byte at **12 pass, 0 fail, 0 timeout, 0 unsupported**, SHA-256 `3121f59e4e0453503dc7baf06c9692c3916313fc258294b80b5b8a7c873ddab2`. `transitions.tsv` records nine gains, three retained passes, and zero losses. Linux package iteration passed **150/150** nextest tests and **1/1** doctest.

The fixed-base win-001 final gate passed package nextest **150/150**, package doctest **1/1**, workspace nextest **2028/2028 with 5 skipped**, and workspace doctests **2/2**. The 63-file source/fixture/lock audit matched before and after; Cargo.lock SHA-256 remained `5805cd2f1cfb89a839377662df59c2d78ab43633c9674ca0caf9a568b47892c9`. Full output is in `evidence/html20/windows-final-gate.log`. HTML-21's bulk html5lib tree sweep remains separate.

## HTML-21 — bulk html5lib tree-construction corpus

Fixed base `660d32b54a229c29eabafa31eda576944216394f` (merged HTML-20). At pinned WPT revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12`, `evidence/html21/manifest.json` inventories all 62 `.dat` resources (1,936 `#data` cases, each exact hash) and freezes all 501 cases in nine document-plus-fragment files before parser changes. This is a ported html5lib-derived parser corpus, not unmodified WPT harness credit. The standalone `html21_probe` and unchanged selected sources are durable under `evidence/html21/`; `README.md` specifies the mechanical tree normalization, unsupported input modes, per-source/context counts and rerun command.

Two byte-identical baseline rounds: **450 pass, 37 fail, 14 unsupported** (raw SHA-256 `e7ef427f20af89b1353353bcef0e2c0f56103a39719a49935068dec6b2214962`). Two byte-identical post-edit rounds: **488 pass, 11 fail, 2 unsupported** (raw SHA-256 `fb5dabf1ac0fbc34da7360672e8633bb2827797a54e0e5fe468c0ed464aa5708`). The fragment-context repair yields 26 fail-to-pass and 12 unsupported-to-pass transitions, 450 retained passes, zero losses. `transitions.tsv` contains every classification and raw output hash; 11 failures and 2 unsupported legacy-keygen panics remain. Linux iteration passed 153/153 package nextest tests and 1/1 doctest.
