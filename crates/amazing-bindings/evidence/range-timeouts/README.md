# Deferred iframe-dependent Range cluster (2026-09-26)

Source: `6a029fa3` (lane merged with local dev). WPT revision:
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Linux Boat iteration,
800×600, fresh `AMAZING_STORAGE_DIR` for each invocation, 120-second deadline.

| Pinned Range fixture | File result | Completed passing assertions |
| --- | --- | ---: |
| cloneContents | timeout | 1 |
| extractContents | timeout | 1 |
| deleteContents | timeout | 1 |
| insertNode | timeout | 0 |
| surroundContents | timeout | 0 |

The three completed assertions exercise a detached Range. The bulk loops wait
for two iframe loads and child scripts (`Range-test-iframe.html`, `dom/common.js`).
The separate three-second scaffold diagnostic passes direct clone/extract,
fails inserted iframe `contentDocument` and `contentWindow` presence, and never
completes the frame load callback. HTMLIFrameElement currently installs only
`sandbox`; bindings/driver have no contentDocument/contentWindow implementation.
This diagnoses unavailable setup, not a Range algorithm failure.

`range-*.json` are raw Linux results; `range-probe.json` is explicitly
scaffolding-only. The probe copies the original harness and child fixtures into
a temporary diagnostic directory; it does not modify the pinned checkout.
Run remotely with `python3 crates/amazing-bindings/evidence/range-timeouts/run-probe.py`
(optional arguments: WPT root and output JSON path).

Per the user's routing decision, these fixtures are **deferred**, not blocked:
JSRT-9 child realms after the await fix, then HTML-25 part 2. Resume this corpus
when that dependency lands. No native Range change is justified by this evidence.
Windows upload failed with `lost connection`; there is no Windows result or
new conformance/gate claim in this diagnostic-only commit.
