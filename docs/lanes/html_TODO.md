# HTML lane decision map

## HTML-1 — XML/XHTML (in progress)

- Base: `d4613a9` (`origin/dev` at initial rebase).
- Owned production code: `crates/amazing-html`; integration needs go through
  DOM/driver owners. No changes to the HTML recovery parser.
- Entry point: `amazing_html::parse_xml(&str) -> Result<Dom, XmlError>`.
  The caller chooses XML by media type / file extension. An error is fatal;
  callers must never fall back to HTML. Input is decoded Unicode, with leading
  BOM removal and XML end-of-line normalization. Encoding names from the XML
  declaration are validated and available from `xml::parse_xml_with_declaration`; this is not
  byte-level encoding sniffing or transcoding.
- Parsing is iterative. Namespace bindings are pushed/restored per element;
  attributes resolve after all declarations on the start tag are known.
  Default namespaces never apply to unprefixed attributes. Duplicate expanded
  attribute names, reserved namespace bindings, and undeclared prefixes fail.
  HTML/SVG/MathML and null/arbitrary URIs use the DOM-owned namespace identity
  through `create_element_ns`; local names and element prefixes are preserved.
  Qualified attribute-prefix preservation remains a routed DOM follow-up.
- The temporary `XmlTreeSink` contract is retired. Parsing owns one real `Dom`
  and returns it only on success; no partial document escapes a fatal error.
  `parse_xml_with_declaration` additionally returns declaration metadata.
- Limits: 16 MiB source, depth 512, 1024 attributes per element. No recursive
  descent stack. Invalid XML characters are checked before DOM construction.
- Supported: XML declaration (version 1.0, encoding, standalone), elements,
  quoted attributes, five predefined entities, numeric character references,
  CDATA, comments, processing instructions, external PUBLIC/SYSTEM doctype
  identifiers (stored, never loaded).
- Explicit boundaries: DTD internal subsets are `Unsupported`; external DTDs
  and entities are never fetched, custom/general entity declarations are not
  implemented. This is not a complete validating or DTD-processing XML engine.
  Scripting and `document.write` in XML are out of scope; parser-created script
  elements carry text only. Dispatch owner must not execute XML inline scripts.
- Normative references: [XML 1.0 Fifth Edition](https://www.w3.org/TR/xml/)
  and [Namespaces in XML 1.0 Third Edition](https://www.w3.org/TR/xml-names/).

### Evidence so far

On win-001 at `381c73a`, rebased onto namespace merge `c8c8264`, refreshed
and re-synced HTML tree: two runs each **82 unit passed / 0 failed / 0 ignored**
and **1 doctest passed / 0 failed / 0 ignored**. All 19 XML tests now assert the
real DOM rather than a recording sink. The new DOM identity test proves null
namespace, arbitrary URI, prefix, local name, and qualified name preservation.
All 23 HTML/DOM crate source-file hashes matched the Mac. Canonical results and
source hashes are in `tests/wpt-html/unit-results.json`. The workspace and
render integration gates remain pending.

Parser-only cascade XML diagnostic: 12 parsed / 0 error / 0 timeout /
2 unsupported out of 14 distinct documents, two identical runs. Per-source
and DOM-dump SHA256s are in `tests/wpt-html/parser-cascade-results.json`.
The two color tests require entities from an external DTD. This is separate
from pixel rendering; it does not establish a cascade pass-rate improvement.

Runnable artifact: `cargo run -p amazing-html --example xml_parse --locked --
tests/wpt-html/xhtml-smoke.xhtml` on win-001. The companion `xhtml-fatal.xhtml`
must fail, not produce a recovered HTML document.

Cascade selection: `tests/wpt-render/cascade-manifest.json`, WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Brief's historical baseline:
35 pass / 24 mismatch / 13 unsupported. Fresh before/after measurements are
pending driver dispatch and harness routing; no measured improvement claimed.

### Exit checks

- [x] Fatal well-formedness/entity/CDATA/namespace tests on Windows.
- [x] DOM-owned arbitrary/null element namespaces and prefix/local identity.
- [ ] Qualified attribute-prefix preservation follow-up (URI/local lookup works).
- [ ] Driver XML dispatch and removal of WPT XML exclusions (owner changes).
- [ ] Two stable cascade runs, before/after counts and hashes.
- [ ] Fresh full workspace gate, no regressions.
- [ ] Final report and HTML-2 proposal grounded in measured limitations.

## Proposed HTML-2 — strict XML byte input

Add BOM/encoding detection and strict UTF-8/UTF-16 decoding with declaration
consistency tests. The existing declaration test accepts decoded text declaring
UTF-16 while the runnable CLI only reads UTF-8: this concrete API boundary
motivates decoding before streaming. Incremental parsing should follow with
chunk-split tests once decoder ownership is settled.

The pinned diagnostic has two actual external-DTD entity blockers and no UTF-8
input failures among 14 files. Route a bounded XHTML entity policy separately;
do not silently use the HTML entity table or fetch arbitrary external resources.
No HTML-2 work started; rendering impact still requires the integration gate.

## DOM integration checkpoint

Rebased onto verified namespace merge `c8c8264`. The earlier `d835fb3` merge
message claimed the seam but its source lacked namespace.rs; `c8c8264` corrected
that mismatch. The HTML scope arm is now upstream.

The temporary `XmlTreeSink` and `parse_xml_into` are removed. Direct DOM
construction via `create_element_ns` passes **82 unit tests / 0 failures / 0
ignored, plus 1 doctest** on win-001, including 19 XML tests that assert actual
DOM identity. Two identical runs and current source hashes are recorded in
`tests/wpt-html/unit-results.json`; the parser-only corpus also repeated
identically with the same 12/0/0/2 counts and stable projection hash.

Remaining routed work: qualified attribute-prefix preservation, parser-safe
creation of a valid unprefixed XML element named xmlns, driver XML dispatch,
and owner-operated render harness changes. **HTML-1 is incomplete** until
these and final render/workspace gates land. No push/PR.
