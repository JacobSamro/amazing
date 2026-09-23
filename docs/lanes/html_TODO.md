# HTML lane decision map

## HTML-1 — checkpointed for daily stand-down; render integration pending

Parser source `868d296` and evidence `8f15980` landed through `c4b6757`;
Mac lane ancestry was verified and lane/html rebased cleanly to that merge.
IA-1 was subsequently reported merged as `9bf1224`, but its authoritative
checks are **UNRUN**; no gate was active. IA XML dispatch and the integrated
cascade boundary remain pending owner confirmation. No new parser ticket.

Daily stand-down: no SSH/SCP until the primary recovery broadcast and no lane
work until resumed. HTML Windows source tree AND target reservation have been
explicitly released; no HTML REFRESH-HOLD remains. Coordinate a new reservation
before any future Windows gate. The exact restart procedure is recorded in the
shared HTML report; first read current primary/co-integrator/IA status locally.

Implementation gate revision: `868d296`, based on `origin/dev` `11dd8a8`.
DOM expanded-name APIs were verified in `74e3c50` before adoption. The earlier
`d835fb3` merge lacked the initial namespace seam; `c8c8264` corrected it.
Both namespace merge issues and all DOM seam requests are now resolved.

### Design

- `parse_xml(&str) -> Result<Dom, XmlError>` constructs and owns the real DOM;
  no partial document escapes a fatal error. No HTML recovery or fallback.
- The temporary `XmlTreeSink` and `parse_xml_into` are retired. The metadata
  entry point is `xml::parse_xml_with_declaration`, returning DOM plus optional
  XML declaration encoding/standalone information. Input is decoded Unicode;
  this is not byte decoding or encoding sniffing.
- The parser validates XML QName grammar, namespace binding/reserved names, and
  duplicate expanded attributes before calling DOM's `create_element_expanded`
  and `set_attribute_expanded`. DOM owns namespace identity and prefix storage.
  This preserves attribute prefixes and accepts valid unprefixed xmlns element
  names without misapplying DOM API QName restrictions. No parallel namespace
  enum, local DOM shim, extra dependency, or sibling-crate modification.
- Namespace scopes use per-element undo records. Default namespaces do not
  apply to unprefixed attributes. HTML/SVG/MathML retain existing enum identity;
  null/custom namespaces, prefix and local name are represented by DOM APIs.
- Iterative parsing, 16 MiB input, 512 open elements, 1024 attributes/element.
  XML character validation, BOM removal and CR/CRLF normalization precede tree
  construction. Character references preserve referenced attribute whitespace.
- Supports XML 1.0 declaration, elements, attributes, five predefined entities,
  numeric references, CDATA, comments, PIs and external PUBLIC/SYSTEM doctype
  identifiers without loading them. XML version 1.1 and DTD internal subsets
  are Unsupported. External-DTD entities remain Unsupported unless standalone
  rules require a malformed-document error. No external resource loading.
- XML scripting/document.write, validating DTD processing, byte decoding and
  incremental input remain out of scope. Dispatch must not execute XML scripts.
- References: [XML 1.0 Fifth Edition](https://www.w3.org/TR/xml/) and
  [Namespaces in XML 1.0 Third Edition](https://www.w3.org/TR/xml-names/).

### Historical Windows evidence — predates the SSH outage

No build or test was run by this resumed HTML agent. The measurements below
predate the fleet SSH outage and are not current integrated-head evidence.
Co-integrator separately reported authoritative `c4b6757` workspace nextest
**1229 passed / 0 failed / 3 skipped**, explicit workspace doctests **2 passed /
0 failed**, and **193 tracked Rust/manifest/lock hashes** matching before and
after, with incremental disabled after the HTML-only refresh. That reported
pre-outage gate is distinct from the source-checkpoint gate below and does not
validate `9bf1224` or subsequent XML dispatch.

After rebase and source re-sync following the integrator refresh, **370 tracked
crate/workspace files matched SHA256 before and after the successful gate**.
An earlier cargo-test attempt interrupted by a refresh is not credited.

- `cargo nextest run --workspace --locked`: **1229 passed / 0 failed / 3 skipped**;
  39.330 seconds test execution. Recorded in tests/wpt-html/workspace-results.json.
- `cargo test --workspace --locked --doc`: **2 passed / 0 failed / 0 ignored**,
  HTML 1 and JS 1. Nextest does not run these tests.
- HTML nextest subset and package repeat: **84 passed / 0 failed / 0 skipped**,
  identical test-name projections. HTML doctest passed separately twice.
  Includes 21 XML tests against the real DOM, including qualified attributes
  and valid xmlns element names. See tests/wpt-html/unit-results.json.
- Pinned parser-only diagnostic: **12 parsed / 0 error / 0 timeout / 2
  unsupported**, 14 distinct XML documents, two identical runs. WPT revision
  `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`; per-source and DOM-output hashes in
  tests/wpt-html/parser-cascade-results.json. Canonical projection remains
  `193d33ee4941222c4c7a6a77ac4726ea9b41c54239c05d2668024a8ef417dee2`.

The two colour cases t421-rgb-clip-outside-gamut-b.xht and
t425-hsla-values-b.xht require external-DTD entities and remain Unsupported.
These parser acceptance counts are not render passes or a WPT conformance rate.
The brief's 35 pass / 24 mismatch / 13 unsupported cascade baseline has not
been freshly reproduced, and no render improvement is claimed.

Runnable artifact: build `cargo build -p amazing-html --example xml_parse
--locked` on win-001 and invoke it on tests/wpt-html/xhtml-smoke.xhtml. The
companion xhtml-fatal.xhtml must return failure, never a recovered HTML DOM.

### Remaining integration and gates

- [x] Fatal well-formedness, entities, CDATA, namespaces and limits tested.
- [x] DOM-owned null/custom namespaces and element/attribute prefixes.
- [x] Valid xmlns element names through parser-safe DOM creation.
- [x] Refreshed workspace gate and explicit doctests; no test failures.
- [ ] Driver XML/XHTML dispatch, no HTML fallback or XML scripting.
- [ ] Git-free render runner/root mapping and removal of blanket XML exclusions.
- [ ] Two stable actual cascade before/after render runs and final integrated gate.

Co-integrator owns integration. The parser dependency has landed; do not request
its merge again or redo parser implementation. IA-XML was activated after that
merge and must preserve IA-1 frozen sweeps. Verify the owner's actual landed
symbols and exact integration revision on resume before claiming dispatch or
runner completion. After recovery and owner coordination, remaining evidence is
actual cascade before/after measurement, twice-stable pinned classifications and
pixel hashes, and final integrated workspace nextest plus separate doctests.
No server Git, no push/PR, and no next ticket. Historical parser counts are not
render passes; both external-DTD Unsupported limitations remain.

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

The pinned sample has two measured DTD-entity blockers and no UTF-8 input
failures among 14 files. A bounded XHTML entity policy is a separate decision;
do not silently use HTML's entity table or fetch arbitrary external resources.
