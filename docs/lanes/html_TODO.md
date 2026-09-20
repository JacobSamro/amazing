# HTML lane decision map

## HTML-1 — XML/XHTML (in progress)

- Base: `d4613a9` (`origin/dev` at initial rebase).
- Owned production code: `crates/amazing-html`; integration needs go through
  DOM/driver owners. No changes to the HTML recovery parser.
- Entry point: `amazing_html::parse_xml(&str) -> Result<Dom, XmlError>`.
  The caller chooses XML by media type / file extension. An error is fatal;
  callers must never fall back to HTML. Input is decoded Unicode, with leading
  BOM removal and XML end-of-line normalization. Encoding names from the XML
  declaration are validated and available from `parse_xml_into`; this is not
  byte-level encoding sniffing or transcoding.
- Parsing is iterative. Namespace bindings are pushed/restored per element;
  attributes resolve after all declarations on the start tag are known.
  Default namespaces never apply to unprefixed attributes. Duplicate expanded
  attribute names, reserved namespace bindings, and undeclared prefixes fail.
  HTML/SVG/MathML use the existing DOM enum. Null/arbitrary URI support and
  qualified attribute preservation await the DOM-owned creation seam.
- `XmlTreeSink` is the lower-crate namespace-aware creation contract, not a
  parallel namespace enum or DOM. It receives URI/local/prefix/qualified names.
  Consumers discard partial state on error; `parse_xml` enforces that by owning
  its DOM until success.
- Limits: 16 MiB source, depth 512, 1024 attributes per element. No recursive
  descent stack. Invalid XML characters are checked before tree callbacks.
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

On win-001, refreshed HTML lane, `cargo test -p amazing-html --locked`:
81 unit passed / 0 failed / 0 ignored and 1 doctest passed. This includes 18
new focused XML tests (hand-authored, **not** a WPT pass rate). Final repeated gate, DOM-seam tests, and workspace gate pending.

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
- [ ] DOM-owned arbitrary/null namespaces and qualified attributes integrated.
- [ ] Driver XML dispatch and removal of WPT XML exclusions (owner changes).
- [ ] Two stable cascade runs, before/after counts and hashes.
- [ ] Fresh full workspace gate, no regressions.
- [ ] Final report and HTML-2 proposal grounded in measured limitations.

## Proposed next investigation

HTML-2 candidates are strict byte decoding / encoding sniffing and incremental
input. The current parser accepts a complete `&str`, so it cannot validate an
encoding declaration against source bytes or consume split byte sequences.
Prioritize using actual rejected pinned fixtures after the HTML-1 corpus run;
no new ticket is started here.
