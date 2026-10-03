# HTML63 — strict XML response byte decoding

Fixed before READY62 `15291bf78bd205c3ae633e1fe0f033ed4b23ecc1`; final after/source chain in gate.json. XML response loading now selects BOM, actual transport charset, XML declaration, then UTF-8; fatal codecs replace HTML meta sniffing/Windows-1252 recovery. Existing XML declaration grammar is reused. Byte-decoded parsing preserves a second BOM as content rather than stripping it again; direct Unicode parser entry is unchanged.

Complete supported XML byte detector adapted from quick-xml src/encoding.rs at `2eaa844b4b40117fb7ed4d1f512ede918125aeab`, MIT (2016 Johann Tuffe), verified pinned license, full license adjacent and notice appended. Our std-only codecs replace encoding_rs. RFC7303 §3.2/3.3 and XML1.0 §4.3.3/Appendix F are primary policy references. UTF-32/EBCDIC remain explicit unsupported codecs; failed active XML document presentation remains the existing null-child boundary.

Linux native679 passed/6skip (before677); workspace4116 passed/17skip (before4114); four docs; CLI1894 trees and expanded1930/6unsupported retained. Whole13 browser directories plus111modules/220wholeHTMLsyntax/192CSSOM/123Selection retain every exact passing assertion multiplicity and passing file, with matching fixture hashes/selection pins. No corpus gains credited. Comparison and compact name-hash records preserve measured identities.

Initial511 full proof also had zero losses/gains. Subsequent4a6 native-only failure was canonical fixture ordering: initial URL commit requested child before loader installation, so the new metadata assertion encountered cached legacy-network failure. Diagnostic raw log is retained in MAIN html63-failed-final. Test-only964 installs loader before actual initial navigation and retains all assertions. Corrected whole batch46168 passed; compressed raw reports fetched promptly under MAIN html63-corrected.

Primary sources: https://www.rfc-editor.org/rfc/rfc7303.html and https://www.w3.org/TR/REC-xml/ (checked2026-10-03). No fixture/comparator correction or gain is attributed to XML engine behavior.
