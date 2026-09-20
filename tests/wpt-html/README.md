# XML/XHTML parser diagnostic

Run builds and tests only on win-001, in the HTML lane tree with
`CARGO_TARGET_DIR=C:\Users\server\target\html`.

```powershell
cargo nextest run -p amazing-html --locked
cargo test -p amazing-html --locked --doc
cargo build -p amazing-html --example xml_parse --locked
C:\Users\server\target\html\debug\examples\xml_parse.exe tests\wpt-html\xhtml-smoke.xhtml
C:\Users\server\target\html\debug\examples\xml_parse.exe tests\wpt-html\xhtml-fatal.xhtml
```

The smoke fixture produces a DOM with HTML namespaces and literal CDATA CSS.
The fatal fixture must exit nonzero without returning a recovered DOM. This
example reads strict UTF-8, not lossy bytes or guessed encodings.

`run-xml-corpus.ps1` measures parser acceptance of the distinct `.xht`/`.xhtml`
tests and references reached from the existing cascade manifest. It uses the
pinned Windows checkout, reads its detached HEAD marker without invoking Git,
and records SHA256 for each source and DOM dump. It does not build, fetch,
modify WPT source, render pixels, or claim a WPT conformance pass rate.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tests\wpt-html\run-xml-corpus.ps1 `
  -WptRoot C:\Users\server\target\css\wpt `
  -ExecutablePath C:\Users\server\target\html\debug\examples\xml_parse.exe `
  -OutputDirectory C:\Users\server\target\html\xml-corpus-1
```

Repeat with a second output directory; compare the decoded JSON reports. The
canonical projection hash uses UTF-8 JSON, keys sorted, separators `,` / `:`
with no spaces, over the original report (before repeat/provenance metadata is added).
`parser-cascade-results.json` retains two identical runs at WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`: **12 parsed / 0 error / 0 timeout /
2 unsupported** out of 14 distinct XML documents. Two color tests require
external-DTD entities, which this ticket deliberately does not load. This
parser-only selection is separate from the 72-case cascade render gate.

Latest source gate: `868d296`, based on `11dd8a8`. HTML nextest results
repeated identically: 84 passed / 0 failed / 0 skipped; the separately run
HTML doctest passed twice. Full workspace nextest: 1229 passed / 0 failed /
3 skipped; separate workspace doctests: 2 passed / 0 failed / 0 ignored
(HTML and JS). See unit-results.json and workspace-results.json. None of these
counts establishes a cascade render improvement.
