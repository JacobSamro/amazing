# Integrator assist decision map

## IA-1 — Split the driver and migrate natural image dimensions

Original evidence base: `ea57978ee5e6a776ff9e278e4973df93471988ae`.
Expanded routing rebased onto `784dc59aec23e83ac04114a2830f19731e74d902`.

The driver dispatch now lives in `crates/amazing/src/main.rs`; command families
live in `cmd/parse.rs`, `render.rs`, `wpt.rs`, `js.rs`, `net.rs`, `webrtc.rs`, and
`gpu.rs`. `cmd/launch.rs` retains the Windows default-launch policy.
`cmd/wpt_harness.rs` is unchanged and registered as a normal module.
`dom_host.rs` owns the shared DOM adapter; `page.rs` owns resource loading and
stylesheet collection. Platform implementations in `gui.rs`, `gpu.rs`, and
`browser_peer.rs` retain their existing boundaries.

The extraction preserves command bodies, input handling, output strings, child
process dispatch, and exit-code handling. Visibility changes expose only the
shared helpers that crossed the new module boundaries. The embedded VP8 fixture
and Chromium-offer test paths gain one `../` for their new directory.

### Correctness change, separate from extraction

`PageMedia::intrinsic_size` previously returned `PageImage::used_size`, computed
from the image's HTML width/height attributes. It now returns decoded pixel
width/height. The redundant adjusted-size field and attribute arithmetic are
removed. Layout consumes CSS used sizing and must receive the natural aspect
ratio. HTML width/height presentational hints are currently ignored by CSS; the
old adapter's accidental attribute sizing disappears with this contract fix.
Proper low-priority hints are routed to the CSS/layout owners, not recreated in
the intrinsic-size provider. The `attribute-size.html` diagnostic measures this
explicitly: a natural 2×1 image with `width=8` changes from 8×4 (32 colored pixels)
to 2×1 (2 colored pixels). This is an outstanding platform limitation, not a
behavior-preserving result claimed for the intrinsic-size fix. A unit test covers absent, single, paired, and CSS-overridden dimensions;
a CLI pixel comparison demonstrates the former distorted ratio: 4×6 becomes
4×2, with 24 versus 8 colored pixels, and the fixed image exactly matches its
explicit 4×2 reference.

The rebased DOM adapter already handles `DocumentFragment`, `DocumentType`, and
`ProcessingInstruction` in serialization and cloning. The requested GUI and
phase2 style call sites already use `Dom::name` / `if let Text`, so they have no
missing exhaustive arms. No duplicate compatibility arms were added. The granted
shell files contain no remaining exhaustive matches needing these variants or
intrinsic-size adapters. The integrator subsequently lifted the exclusion for
`engine_viewport.rs`: its `PageResources::intrinsic_size` now returns decoded
natural dimensions, its adjusted-size storage/arithmetic is removed, and the
former 8×4 assertion now verifies the natural 2×1 dimensions.

### Original-scope validation

All builds and execution occur on win-001 in the integrator-assist source and
target directories. The tree was refreshed after rebase, then refreshed and
re-synced again before the workspace gate. Build jobs are capped at four;
incremental compilation is disabled.

- Driver: 98 passed / 0 failed / 3 ignored.
- Workspace, twice: 1024 passed / 0 failed / 3 ignored, 53 target/doc-test
  summaries per run, exit 0, no compiler warnings.
- All 217 final source/input files matched Windows; SHA-256 of the source
  manifest: `54d50c5240ff2e1e9653ec1adca682014b6f979e62e5349762de9cee8fcfd7b0`.
  Rust inputs are unchanged since the workspace tests.
- Original `main.rs`: 3519 lines; dispatch after extraction: 131 lines.
  Largest driver source afterward: `cmd/webrtc.rs`, 1737 lines.
- CLI: 36 cases cover all named dispatch commands; 33 compare byte-for-byte,
  three transport probes differ only in OS-assigned localhost ports. Pure
  extraction and final candidate each have two matching captures, normalized
  SHA-256 `bf13d50a3f7c2eedd73d7cb97c2863190f2ab13770fb4fa2bbc3933611f23852`.
  Exit codes, stderr, and BMP/PPM artifact hashes are included. GUI is only a
  bounded lifecycle capture, not visual proof; some interactive commands use
  error/usage inputs. Default no-argument GPUI launch is not exercised.
- WPT harness scaffolding: 14 cases, two stable runs, SHA-256
  `f8db03a6109855078313208ac15dc229b663729f9f8b7c2a970c46daa5475519`.
  These synthetic cases earn zero WPT credit.
- Original-scope pinned render comparison is complete: 16 selections / 654
  overlapping entries at WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
  One baseline and two candidate runs; all normalized result/pixel hashes are
  identical. Full counts and hashes are in
  `tests/wpt-integrator-assist/evidence/gate-evidence/summary.json`.

### Additional routing

The render runner now resolves `/css/reference/blank.html` against `WptRoot`
and supplies `AMAZING_WPT_ROOT` only to its renderer child. Driver root-relative
scripts (including `/common/reftest-wait.js`), images, stylesheets and fonts use
that explicit root for documents inside it. Ordinary relative resources retain
their existing resolution. Query/fragment suffixes and percent-encoded names
are handled; unsupported remote and escaping references remain classified.

The runner never invokes Git, with or without `-SkipFetch`. Provision the pinned
checkout on the editing host and transfer it. A detached `.git/HEAD` is the
default revision marker; an exported tree can use `-RevisionMarker` with a file
written from the verified editing-host revision. The full 40-digit marker must
match the manifest pin. Missing, symbolic, or mismatched markers fail clearly.
The marker records provenance; it is not a hash of every checkout file.
Artifact directories use short hashes to avoid Windows MAX_PATH while reports
retain original test IDs.

`verify-root-mapping.ps1` uses an explicitly synthetic revision and earns zero
WPT credit. It exercises root-relative, ordinary-relative, and encoded paths;
rejects escaping/remote references; checks missing/wrong markers; and installs a
throwing Git sentinel. Both runs pass 3, classify 2 unsupported, reject two bad
markers and record zero Git invocations; these fixtures earn zero WPT credit.
CSS owns the HTML presentational-hint follow-up; adjusted intrinsic sizes must
not be restored.

HTML-1 remains incomplete: XML dispatch, mapping `XmlErrorKind::Unsupported` to
the render result, and removing blanket XHTML test/reference exclusions are
pending together. Existing generic unsupported reporting and XHTML exclusions
are retained. The parser must land before the separately queued dispatch work;
that work must not fall back to HTML on XML errors. IA-1 claims no XML
conformance.

### Completed expanded IA-1 measurements

Validated source commit `5648fb9fa19773ce4377540cf14b7aec714bb3bc`, integration
base `784dc59aec23e83ac04114a2830f19731e74d902`. Windows workspace twice: **1134
native passed / 0 failed / 3 ignored**, plus **2 doctests passed / 0 failed** per
run (`cargo test --workspace --locked`). Shell viewport checks: **16 passed**.
The new fleet standard for future runs is nextest plus explicit doctests.

At WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, all **17 selections / 678
overlapping entries** completed for one same-base baseline and two candidate
runs. Candidate normalized result/pixel hashes match in every selection.
Totals: baseline **362 pass / 280 mismatch / 0 timeout / 19 unsupported / 17
crash**; each candidate **363 / 280 / 0 / 17 / 18**. This is not a unique-test
pass rate. Every result except these two cases is byte-identical after projection:

- `input-checkbox-zero-size.html`: unsupported (wrong reference location) →
  pass, exact pixel match. Controls selection 21 pass/2 mismatch/1 unsupported →
  22 pass/2 mismatch/0 unsupported.
- `use-descendant-combinator-003.html`: unsupported → crash. Correct reference
  mapping exposes an existing page-script `TypeError: cannot access a property
  on nullish value`; it is not counted as a pass. Replaced-content manifest
  remains 26 pass/4 mismatch, with 1 unsupported/17 crash → 0 unsupported/18 crash.

| Selection | Pass | Mismatch | Timeout | Unsupported | Crash |
|---|---:|---:|---:|---:|---:|
| block-collapse-manifest | 9 | 0 | 0 | 0 | 0 |
| cascade-manifest | 35 | 24 | 0 | 13 | 0 |
| computed-values-manifest | 20 | 0 | 0 | 4 | 0 |
| controls-manifest | 22 | 2 | 0 | 0 | 0 |
| flex-grid-broad-manifest | 61 | 139 | 0 | 0 | 0 |
| flex-grid-manifest | 24 | 0 | 0 | 0 | 0 |
| geometry-manifest | 31 | 30 | 0 | 0 | 0 |
| image-manifest | 12 | 0 | 0 | 0 | 0 |
| manifest | 26 | 4 | 0 | 0 | 18 |
| paint-compositing-broad-manifest | 24 | 36 | 0 | 0 | 0 |
| paint-compositing-manifest | 24 | 0 | 0 | 0 | 0 |
| position-manifest | 9 | 3 | 0 | 0 | 0 |
| svg-manifest | 12 | 0 | 0 | 0 | 0 |
| text-flow-manifest | 14 | 2 | 0 | 0 | 0 |
| text-manifest | 17 | 31 | 0 | 0 | 0 |
| text-shaping-manifest | 12 | 0 | 0 | 0 | 0 |
| vertical-ruby-manifest | 11 | 9 | 0 | 0 | 0 |

Evidence: `tests/wpt-integrator-assist/evidence/expanded/`. All 258 recorded
source/input files matched Windows before a subsequent fleet refresh. During
that refresh, all 17 manifest JSON values and both frozen executable hashes
remained unchanged, preserving the sweeps. These are measurements of the stated
build base, not a gate of subsequently advanced dev. No shell screenshot claim.
The earlier original-scope evidence remains separate; it used 16 selections,
and its text16/32 differs from the newer base's text17/31 even before root fixes.

The natural-size viewport rewrite now has the exclusive transfer `5005c1c` and
REVIEW classification `b5d1b99`. This grants no XML shell dispatch. IA-XML is
explicitly activated as the next scoped work; IA-1 is checkpointed before that
implementation begins.

## IA-XML — driver document dispatch

The driver selects XML parsing for local `.xml`, `.xht`, `.xhtml`, and `.svg`
files and for HTTP `application/xml`, `text/xml`, or `+xml` MIME types. An HTTP
`text/html` response remains HTML regardless of its URL suffix. XML parser
errors are fatal and never fall back to HTML. `XmlErrorKind::Unsupported`
propagates separately from malformed XML and ordinary rendering/JS failures:
exit 3 maps to `unsupported`, exit 2 to `error`, and exit 1 to `crash` in the
render runner. Static XML can render, but XML script elements and inline event
handlers are refused as Unsupported, and `domrun` refuses XML scripting. The
runner now renders XHTML tests and references instead of excluding them by
extension; when a reference fails, it retains the actual image hash.

The frozen BEFORE is driver `aa9cb9be3ca79425bb979519c75b442646b37028`,
executable SHA-256 `28b53f5950a99b972854ba3095476a77b5b7a91443ce40486a1d4d709a248ec1`.
At pinned WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, both 72-case
cascade runs had **35 pass / 24 mismatch / 0 error / 0 timeout / 13 unsupported /
0 crash**, projection SHA-256 `643e965ccd8783a3c1c94f395a5812c286286171ac4b0ac02de0dbe5e71d1f86`.
This is the old revision's baseline, not a current-head control.

After rebase onto `ffb143b`, driver revision
`8a7b5c453d45629c94300add25988488601db2fe` built executable SHA-256
`950964fe1de5296b5ba9b1a730470fe6cad0d9f58a7bf53a95da5af43c443055`.
Both current 72-case runs had **38 pass / 32 mismatch / 0 error / 0 timeout /
2 unsupported / 0 crash**, stable result/pixel/detail projection SHA-256
`8aca2064b4c6309679c4a4b1508a163c4c3be720dbc597a2eb6f59c42d7a5c6f`.
Of the 13 former unsupported cases, 3 became pass and 8 became mismatch. The
other 2 remain Unsupported because they reference entities requiring external
DTDs; they receive zero pass credit. All 61 cases with unchanged classifications
also retained their recorded pixel hashes. These are overlapping selection
results, not a browser-wide WPT pass rate. No shell XML dispatch is claimed.

The 7 CLI integration tests exercise static XML pixels, case/structure,
fatal malformed XML, external-DTD Unsupported, refusal of XML scripting,
HTML script behavior, and HTTP MIME selection. Scoped driver nextest passed
107/107 with 3 skipped on the refreshed Windows tree; the binary package has
no library target for doctests. All 231 recorded source/input paths matched
Windows hashes after own-tree refresh and re-sync. The synthetic four-case
runner fixture classified 2 `error`, 1 `unsupported`, and 1 `crash`, with zero
passes and a retained actual pixel hash when the XML reference failed. It
earns zero WPT credit. Final win-001 workspace nextest passed **1270/1270**
with **3 skipped** across 43 binaries; separate doctests passed **2/2** with
0 failures across 19 summaries. The gate used the primary shared build slot
and stayed under `REFRESH-HOLD` until both commands finished. Counts and
artifact identity are recorded in the ticket report and `xml-evidence/`.

## IA-HTML2 — driver document byte decoding

On fixed dev base `2021707`, the driver now passes local document bytes and
HTTP response body bytes to merged `amazing_html::parse_bytes`, with the actual
HTTP Content-Type charset label. XML selection happens first and still feeds
caller-decoded Unicode to `parse_xml`. XML parse failures and Unsupported stay
fatal, with no HTML fallback or XML script execution. Native GUI HTML loading
also takes response bytes; local file metadata synthesized by amazing-net is
not treated as an HTTP charset. Scripts, stylesheets, and fragments retain their
existing text paths. This adopts HTML-2's four-decoder subset, not full WHATWG
Encoding coverage.

The pinned 12-case raw-byte driver selection compares revision `2021707`
(`before.exe` SHA-256 `5ced7c78b96e08cc6ac3b55fb60dbd849374bc80e4051e72604f309a476fb732`)
with `43c83d40bf07b2468cc5ec9c2aee5c722ded49ff` (`after.exe` SHA-256
`7c65ce68a30c1fde27f8ece0e77d6cc5dcababfdc427691b21eb3fe7636f18da`).
Both runs of each executable agree exactly: BEFORE projection SHA-256
`fdb548e3fc3973ab31cda1eac2fccc1ef92ccb4a359869d35a866c6fc3df1461`,
AFTER projection SHA-256
`fcf731df6b539e06f7593b22dab3cd6264a949f96906e7cd07f9c8c49fa79f01`.
Seven HTML outputs change: BOM removal, meta and default windows-1252,
UTF-16LE/BE, HTTP transport label over meta, and BOM over HTTP transport.
Malformed UTF-8 replacement and four XML controls remain byte-identical in
their output, with zero changed exit classifications across all 12 cases.
The selection contains no Unsupported case; the separate scoped XML dispatch
tests exercise exit 3 for an external-DTD entity. This is a fixed-base lane
gate, not evidence for later dev merges or a browser-wide conformance rate.
Raw reports, input hashes, actual transport labels, and comparison are in
`tests/wpt-integrator-assist/html2-evidence/`.

Scoped win-001 nextest passed **112/112** with 3 skipped; the separate scoped
doctest invocation found no library target in the binary `amazing` package.
Final fixed-base workspace nextest passed **1322/1322** with 3 skipped, and
separate workspace doctests passed **2/2** with no failures. Both workspace
commands used the primary shared build-slot wrapper under `REFRESH-HOLD`, then
the hold was released. All 944 tracked HEAD source/input paths in the committed
manifest matched SHA-256 on the Windows lane tree after the gates. These
measurements do not substitute for co-integrator merged-head preflight.

## IA-2 proposal — shell/driver URL migration

Follow the caller inventory and persisted-key concerns in
[net_TODO.md](net_TODO.md). Replace legacy `Url` use in driver resource resolution,
DOM URL completion, and shell navigation with the canonical network URL API.
Then migrate cookie/cache keys, storage origins, and persisted session state with
an explicit old-key policy. Require grants for each shell file first, especially
`engine_viewport.rs`; the IA-1 grant covers only its intrinsic-size migration.

Gate the migration with relative-resource/navigation fixtures, canonical-origin
and default-port cases, persistence round trips, the pinned URL selection, and
actual Windows shell screenshot proof. Keep it a separate ticket: URL behavior
must not be folded into this command extraction.

## DOM-5 driver adoption — mapping checkpoint

Mapped alongside IA-XML, but checkpointed separately. Inspected the actual
`BindingHost`, `InterfaceDefinition`, `Member`, `Operation`, `install/INTERFACES`
and `invoke` seam at DOM checkpoint `2b917f8` (including `afa9ad5`), read-only in
the DOM lane. The final ownership decision is a backend-neutral
`amazing_bindings::host::BindingHost` trait drafted by dom-bindings and reviewed
by js-runtime, with a driver-local `RuntimeBindingHost` in `crates/amazing`.
The driver must not create a parallel trait or wrapper cache. Begin only after
js-runtime accepts the final trait text. The latest Stage-1 priority places
this after JS-4 scheduling and before streamed document loading.

Driver changes required once the runtime adapter is supplied:

- `dom_host.rs`: lend mutable access to `BrowserDomHost.dom` through the runtime's
  actual arena hook. This must be the same arena parsed for the page, not a cloned
  DOM. Current `amazing_js::DomHost` exposes individual operations but has no
  arena-access callback; that contract belongs to the runtime lane.
- `cmd/wpt_harness.rs::execute`: install the runtime's standard interfaces after
  attaching the same host and before upstream testharness/page scripts execute.
  `cmd/js.rs` installs that host in inline-script and run-dom paths; native-driver
  GUI is another existing consumer of the shared host. No XML shell dispatch is
  authorized by this adoption map.
- Keep node brands private. Runtime factories, query results and new binding
  operations must share its canonical wrapper identity. `childNodes` must be
  live and same-object. UTF-16 conversion/coercion stays outside DOM borrows;
  constructors, methods and accessors forward the supplied `Operation`.

The driver currently depends on `amazing-js`; its direct `amazing-bindings`
dependency and installer belong to the future ticket. This map does not
implement or validate that installer. Runtime string methods still wait for
the JsString migration.

Validation after dependencies land must show an API is visible in actual driver
WPT execution, mutations reach the parsed arena, factory/query/new-interface
node identity agrees, and a retained `childNodes` object updates after mutation.
Use a newly installed API (for example Text construction or append) and report
measured before/after separately from native DOM/binding counts. This map earns
no WPT credit; the completed XML no-fallback/no-scripting behavior remains a
regression requirement.

## IA-JS4 — driver EventLoop scheduling adapter

Fixed dev base `c8c3c21` includes DOM-6's landed reference proof. Driver
implementation `1f16fec356c957922fe5428a70fcf3f521268787` constructs each
interpreter with `new_with_external_scheduler(4096)`, attaches the same parsed
DOM, and registers one EventLoop global. Every page-facing entry in CLI,
classic-script renderer, WPT worker, and native GUI goes through the shared
driver scheduler: script, inline-handler installation, DOM event dispatch,
Promise/queueMicrotask job, timer/frame callback. A taken outbox request remains
driver-owned in FIFO until admitted; a downstream error blocks later JS entry
without dropping it. Timer/frame maps retain core IDs and opaque JS callback
tokens; callbacks receive Window and frames receive the core timestamp.

BrowserDomHost forwards one pending native mutation request after release of
the DOM borrow; the runtime acknowledges only successful admission. The DOM
pending flag remains until EventLoop's mutation checkpoint, and re-arms after
delivery. EventLoop steps drain newly emitted requests before `complete`, even
after a thrown callback. Driver clock advance and rendering opportunities are
explicit. The adapter is single-threaded and does not add BindingHost or a JS
MutationObserver constructor; the native mutation proof uses a pre-registered
DOM observer and is synthetic, not WPT credit. The checkpoint consumes native
records; JS observer callback invocation remains for BindingHost installation.

Five driver adapter tests passed twice with the same eight named trace outcomes
and normalized SHA-256
`4c6d28065fe67184cef9a69456d45ab1f2d9be8ad10afddfe702a4c84904eb59`.
The DOM-6 reference proof passed 5/5. Scoped driver nextest passed **122/122**
with 3 skipped; the binary package has no scoped library doctests. The scaffold
WPT worker retained its expected pass and fail classifications. Fixed-base
workspace nextest passed **1414/1414** with 3 skipped; separate workspace
doctests passed **2/2**. Both full commands used the primary shared slot under
REFRESH-HOLD, then the hold was released. All **1109/1109** tracked HEAD file
hashes matched win-001 after the gate. Trace reports, exact commands, source
manifest, and entry inventory are in `tests/wpt-integrator-assist/js4-evidence/`.
This lane gate does not prove later dev merges; co-integrator owns merged-head
preflight. No GPUI or lower-crate source was changed.

## Queued after IA-JS4

1. Driver-local RuntimeBindingHost installation, after js-runtime accepts the
   final `amazing_bindings::host::BindingHost` trait text and string facade.
   Use the ownership decision above; this ticket has not begun.
2. Streamed document loading: NET-9 `fetch_stream` on dev `dc184d2` supplies
   decoded byte chunks and drop cancellation; HTML-3 `IncrementalParser` takes
   already-decoded Unicode via `push_chunk` and `finish`. Bridge the byte and
   encoding boundaries under HTML-2's four-decoder limit, without a full-body
   buffer or lower-crate edits. Full queued brief is in
   `.agents/co-integrator/ia-streaming-queued-20260923.md`.

## IA-STREAM — pull fetch bodies into the driver parser

The driver uses NET-9's `fetch_stream` for `fetch-parse` and native GUI page
loading. After response headers choose document mode and an HTTP(S) transport
charset, each HTML `next_chunk` result goes directly to HTML-4's
`IncrementalByteParser::push_chunk`; `finish` is called once at EOF. Local file
Content-Type is synthetic and supplies no charset. XML keeps the existing
whole-source `parse_xml` path with fatal malformed/Unsupported errors and no
HTML fallback. `fetch-parse` checks non-2xx status from the head before reading
its body. The driver holds one HTML chunk at a time rather than a full HTML
response. Scripts still run after EOF, and paint is not progressive.

Implementation `2fb0208adb7cd889798f2a8614b89f6c3abe82db` is on fixed
green dev base `133db582307bf14fd9cd8ece92a47c9620869f0d`. The delayed
HTTP fixture compares five exact DOM dumps and HTML modes with the equivalent
buffered parser. It withholds the rest of each body until the driver has pushed
the first part; BOM, meta, UTF-8 scalar and UTF-16 boundaries are split. The
100,007-byte case delivered 14 chunks with an observed 8,192-byte peak driver
byte buffer. Smaller case byte/chunk/peak counts are recorded in
`tests/wpt-integrator-assist/stream-evidence/gate-summary.json`; small chunk
counts can vary with TCP coalescing. XML valid DOM/mode and malformed error
text/exit match the buffered path. An incomplete HTTP 404 body is rejected
from headers without another network read. Existing XML dispatch and GUI file
decoding tests pass. These are controlled fixtures, not WPT credit.

On win-001, the refreshed and re-synced lane tree matched **719/719** tracked
Cargo/source/script/non-evidence test input SHA-256 hashes before and after the
gate, with zero missing or mismatched paths. Scoped nextest passed **125/125**
with 3 skipped; the binary package has no library doctest target. Under the
shared build slot, workspace nextest passed **1439/1439** with 3 skipped and
separate workspace doctests passed **2/2** across 19 summaries. Commands, log
hashes, source manifest and fixture measurements are in
`tests/wpt-integrator-assist/stream-evidence/`. No net, HTML, shell, root
manifest or lockfile sources were edited. The four-decoder HTML-2 limit remains.

RuntimeBindingHost is the next separate driver ticket after both DOM-7 and
JSRT-3 land with the final accepted trait text. It was not started in
IA-STREAM.

## IA-CACHE — GPUI shell adoption of NET-10 cache policy

Fixed base `a858fd08b66c34ec3d9ef47f06be115680ae3d77` includes the
validated public `amazing_net::Headers::append` needed to return cached Age
and merged 304 headers. The shell now delegates freshness, Vary, validation,
304 merge, and unsafe-method invalidation to the private `amazing_fetch`
`CacheStore`. It keeps the existing request-plus-Cookie key, final response URL
and reason, and hit promotion. Limits are 64 entries, 16 MiB aggregate, and
8 MiB per body; NET-10 also charges metadata against the aggregate limit, so
effective capacity is slightly stricter. Cache entries remain memory-only.
`session-state-v1` retains exact `C/L/H/B/D` records.

Before the adapter, shell nextest passed **43/43** but the controlled ETag/304
proof failed **0/1**: a stale `no-cache` response was reused without sending
`If-None-Match`. After the adapter, shell nextest passed **46/46**. The four
focused cache and persistence tests then passed **4/4 twice**, and the
controlled 304 proof passed **1/1 twice**: it observed the conditional request,
original body and status, merged `Cache-Control: max-age=60`, and a third fresh
hit without a connection. A repeated run exposed an overly strict `Age: 0`
assertion; it now accepts a fresh age below the 60-second lifetime. This was a
test timing correction, not a cache policy change.

On win-001, the locked standalone shell workspace nextest passed **65/65**,
0 skipped; its separate doctest command passed with **0 doctests**. The locked
root workspace nextest passed **1448/1448**, 3 skipped; separate root doctests
passed **2/2**. Both workspace gates ran under the shared build slot with
`REFRESH-HOLD`. All **1192/1192** tracked file SHA-256 hashes matched the
Windows lane tree before and after the gates. The generated shell lockfile
adds only `amazing-fetch` and its shell dependency edge. Logs and the pinned
manifest are in `C:/Users/server/target/integrator-assist/diagnostics/ia-cache/`.
These fixed-base gates are lane evidence; merged-head preflight belongs to the
integrator. No WPT result is claimed for this shell-only change.

## IA-SVG — driver adoption of the RC-13 SVG renderer

Fixed pushed base `582a6af` contains RC-13, HTML-15, CSS-11, and NET-20.
The lane branch retargets the driver and static reftest adapter from the old
`amazing-image` re-export to root-workspace `amazing_svg::{from_dom, render_svg}`.
The driver's `data:` image and stylesheet loading uses NET-17's
`amazing_fetch::process_data_url`; other resources keep their file/network
paths. A focused Windows driver gate passed **130/130**, three skipped, and
the reftest data-image/data-stylesheet test passed **1/1**. The root driver SVG
manifest/lock edge was granted by the primary. Cargo's Windows metadata also
required one separate existing fetch lock edge and the standalone shell lock's
HTML-15 encoding edge. The shell lock change matches Cargo's generated output
exactly. All nine branch source/lock file hashes matched the Windows lane tree.

The unchanged RC-11 manifest SHA-256 is
`2fdbe12184173e8808ad2c09a6dc2ed41254affaa4bd1d84054e1880e110cfb0`,
at WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, viewport 800×600 and
DPR 1. The predecessor at `859bb53` repeated **22 pass / 13 mismatch** on all
35 cases, **21/11** on the 32 inline cases, and **11/7** on the 18 inline cases
that both Chrome and Safari pass. The IA adapter on RC-13 repeated **26/9**,
**25/7**, and **14/4** respectively. The full 20-case both-browser consensus
moves **11/9 → 14/6**. All four gains are
`pathlength-005`, `polygon-with-filtered-marker`, `marker-implicit-subpaths`,
and `will-change-under-mask`; no raw pass became a mismatch. Both AFTER runs
have **23 nonblank passes** and identical per-case classifications, differing
pixel counts, actual/reference/diff hashes, and nonwhite pixel counts. The
AFTER projection SHA-256 is
`a3302c905f0c7f2da50c91bd6b671125f1b78fbdfc4508b69eb1d32ff0821b99`
in both rounds. It matches RC-13's after-move report per case.

Three AFTER raw passes are blank actual/reference matches and get no paint
credit: `ellipse-auto-rx-textpath`, `pathlength-005`, and
`non-scaling-stroke-007`. The first and third were nonblank passes in the
pre-RC-12 baseline, so **zero raw pass losses does not mean zero nonblank
regressions**; those two cases lose paint evidence. RC-13 owns the renderer
subset and this lane does not edit it. The first attempted sweep was denied by
PowerShell execution policy before any case ran; the measured pair used
process-local `ExecutionPolicy Bypass`. Raw reports, pixel counts, and
comparison helpers are under `.agents/integrator-assist/ia-svg-final-{1,2}/`.

Final fixed-base Windows gate under the shared build slot: root workspace
`cargo nextest run --workspace --locked` passed **1,719/1,719**, three skipped;
separate `cargo test --workspace --locked --doc` passed **2/2** across the
workspace. The gate logs have SHA-256
`9ca966780deb7b9eb000cc48cddd910c7d9b2f6524cfbdf85f16d717e9dfe72a`
(nextest stderr, including summary) and
`7e375c6ecdf2b3834b1c8e3f3199e7e9708fa14b554d9b8dd75284acdbedf0ac`
(doctest stdout). All 11 files changed between the originally refreshed
`6dff430` tree and the final rebased branch matched Mac/Windows SHA-256 after
the gate, including the two pushed-base scripts. `REFRESH-HOLD` was released.
The pushed base `582a6af` differs from `6dff430` only in those scripts. No
push or PR was made by this lane.

## IA-ADOPT — DOM-15/14/13/10 driver adoption (2026-09-23)

On the IA branch, `RuntimeBindingHost` now chooses `HTMLFormControlsCollection`,
`HTMLOptionsCollection`, `RadioNodeList`, `ValidityState`, and
`CustomElementRegistry` prototypes for their native brands. Live form/select/radio
index counts and supported names read the same DOM arena as page scripts; the
options indexed setter calls the binding operation. Date values use the realm's
Date constructor and the runtime's existing date value. Focused Windows driver
tests exercise collection identity and indexing, radio value, options indexed
replacement, constraint validity, Date conversion, document metadata/URL
reflection, registry identity, definition lookup, and `whenDefined` promise
resolution. Existing tests cover ShadowRoot interface selection and slotchange
checkpoint delivery, plus abort timeout delivery through the driver timer queue.

`window.customElements` uses the current document's canonical registry wrapper.
The driver supplies constructor checks, object/sequence conversion, Promise
resolve/reject, DOMException values, and thrown reaction reporting. It drains
custom element reactions at each driver event-loop entry after script execution
and before outbox admission. The JS runtime currently constructs a derived
class with an ordinary `this` and calls `super()` as an ordinary call. Therefore
JS custom-element upgrades cannot finish with the canonical host wrapper;
JSRT-7 owns that runtime/host-super facade. HTML-14 parser construction remains
a separate document-loading integration. The registry surface is installed,
but no class-upgrade WPT conformance is claimed.

The DOM-15 `pattern` hook now uses JS-9's first-party `amazing-regex` v-mode on
exact UTF-16 code units. It checks standalone pattern validity before compiling
the anchored expression, returns no compiled pattern for invalid syntax, and
propagates resource limits. The primary granted the driver dependency and root
lockfile edge. DOM-10 abort timeout,
DocumentHost, and URL reflection were already installed on this base. There is
no existing fetch/XHR global abort callback to connect before IA-NET; the
unsupported abort algorithm remains explicit. NET-16 transport/security adoption
and Fetch/XHR globals stay on IA-NET.

The frozen DOM WPT selection is 627 tracked sources at revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, selection SHA-256
`9787de028efe5dc83f491ce2a818e2a549b4a07a236e6538c1e6165bb437b0b5`.
On old fixed base `d2a610e`, the twice-repeated BEFORE was 77 passing files,
1,775 passing subtests (raw SHA-256 `f14b6c721312fde5221cb06ba7d845e0102457e5fff5af47d46bfcbed456ce5d`);
the IA AFTER was also 77/1,775 (raw `704c3857f2d4be882ed6e3f3845d4f76fb3ce5b2297e998ab04e39bb14def0a2`).
There were zero file pass gains/losses and zero subtest pass gains/losses.
`dom/nodes/moveBefore/slotchange-events.html` changed from harness error to file
failure with four failing subtests; the first reports `cannot convert object to
primitive`. It exercises a derived custom element class and earns no pass credit.
Both old-base pairs were stable and their raw reports are retained in the IA
orchestration evidence. The IA branch was then rebased onto JSRT-5 dev
`cb3064b` as `8690864`; the final rebased WPT and workspace gate are measured
separately below.

The first rebased AFTER attempt used three WPT workers. Both rounds retained
77 passing files and 1,775 passing subtests, but the pair was **unstable**:
`Document-createElementNS.html` crossed the unchanged 10-second deadline after
596 versus 464 failing assertions. Its file status was timeout in both rounds,
yet the raw and outcome hashes differed, so this attempt is diagnostic evidence
only. Three standalone executions of that pinned source on the same executable
completed all 596 assertions as file failures in 9.47, 7.95, and 8.58 seconds.
The subsequent serial, unchanged-deadline run was stopped by primary checkpoint
direction during round 1 after its last logged 225/627 sources. It is partial
and carries no baseline or final pass claim. A complete stable pair is still
required before final WPT stability can be claimed.

## Queued after IA-ADOPT — font loader adoption

Net's `FontRequest` and `NetworkService::start_font` are ready for the driver.
Next, route `@font-face` resource loading through that mode and register decoded
faces with the page `TextProvider`. Keep its corpus and final Windows gate as a
separate ticket after IA-ADOPT.

## IA-ADOPT fixed-base completion evidence

Commit `1d1066b` rebases the driver adoption onto fixed `dev` base `f8bdb33`,
which includes IA-SVG, JS-9, and NET-22. The primary granted the one
`amazing-regex` driver manifest line and one `amazing` package lockfile edge;
both are in the same commit. `BindingHost::form_pattern_matches` compiles the
standalone UTF-16 pattern with the JS-9 `v` engine, then compiles the anchored
full-string form; syntax errors leave the pattern uncompiled, and resource
errors propagate. The focused driver test passes all four assertions. The
`HTMLInputElement.pattern` IDL reflection is still absent in DOM-15, so that
test changes the DOM attribute with `setAttribute`; a request is in the lane
questions file for dom-bindings. JSRT-7's derived custom-element `super()`
facade remains unsupported.

The pinned 627-source WPT `dom/` selection at revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` and selection SHA-256
`9787de028efe5dc83f491ce2a818e2a549b4a07a236e6538c1e6165bb437b0b5`
ran with three workers, 800×600 viewport, 20-second normal and 120-second
long deadlines. Linux BEFORE on venus-001 repeated **87 passing files / 2,252
passing subtests**, with identical raw SHA-256
`2facb5712486dbe746625b7f5553332bf825774144cbfdba168b5af5ff5bdb3e`
and outcome SHA-256
`d4d6adb50c3aab642de5bdaa5a04baa87965268f9f1b37c793e66b19b9ee34a1`.
Linux AFTER on venus-003 repeated **87 / 2,252**, with identical raw SHA-256
`2915428596cb6509445060e4ff67e7b1928cc4cc00266bbe7cce85d3063c3e3c`
and outcome SHA-256
`77c2b8768596707f9ab77d8702cfc448f4791cdde7a8667439be28519dd13362`.
Exact named pass comparison found **zero file pass gains, zero file pass losses,
zero subtest pass gains, and zero subtest pass losses**. The sole file status
change is `dom/nodes/moveBefore/slotchange-events.html` from harness error to
failure with four executed failing assertions; no pass credit is claimed.
The frozen upstream harness proof passed twice in each pair. Raw reports are
retained in the lane's `.agents` evidence folders.

Linux `cargo nextest run --workspace --locked` and separate
`cargo test --workspace --locked --doc` both passed on `1d1066b`. The final
Windows fixed-base before/after corpus and scoped/workspace/doctest gates are
measured separately below.

On win-001, the frozen BEFORE WPT pair also completed stably twice at **87
passing files / 2,252 passing subtests**, with raw SHA-256
`377b86b80aa6b891174bde07e54d477d0a8d8f071dc580f4c9b4a8de47fc3e1c`
and the same outcome SHA-256 as Linux BEFORE. The host's first pair was
stopped after that WPT result when the primary introduced win-002 for plain
Windows gates. The scoped nextest then passed **134/134**, three skipped, on
win-002. `amazing` has no library target, so crate doctests are N/A; the
workspace nextest passed all **1,790 runnable tests**, and separate doctests
passed **2/2** on win-002. Nextest's locked workspace discovery on that same
tree lists **1,794 tests**, including four ignored (three in `amazing`, one in
`amazing-text-provider`). Five changed code/lock files matched Mac and win-002
hashes.

The win-001 AFTER WPT pair completed stably twice at **87 passing files /
2,252 passing subtests**, with raw SHA-256
`a0597234a208dd0fc2df733d63893773080353096b856063335c2af36dbf8a1b`
and the same outcome SHA-256 as Linux AFTER. Both Windows pairs had exact
matching raw record bytes across rounds. Exact named-pass comparison found
**zero file gains/losses and zero subtest gains/losses**. The same sole
`slotchange-events.html` error-to-failure transition exposed four failed
assertions and earns no conformance credit. Pinned WPT proof and selection
hashes matched the Linux runs. Full Windows raw reports are retained in the
lane's `.agents` evidence folders.

## IA-COMPOSITOR — retained live GPUI page frames (2026-09-24)

Fixed pushed base: `82d273a` (including COMP-5, RC-17a, the raw SolidColor
direct-path fix, and TEXT-6). The live shell creates a `DisplayListView` from
the laid-out page, its single registered `TextProvider`, styled images and
controls, then retains a compositor and GPUI presenter per viewport. The
presenter receives each frame before the borrowed view is dropped. The GPUI
viewport displays `GpuiFrame::element()` with the existing link/control hit
overlays. A scene rebuild makes a new presenter; property updates preserve
the page inputs and notify the caller only when the frame has damage.

The full-paint reference and layout use the same page `TextProvider`; system
faces and CSS `@font-face` registrations follow the driver loader. The old
font/outline path is no longer used for live page layout or painting. The
isolated shell manifest adds `amazing-text` and `amazing-text-provider`. Its
Cargo-generated lockfile adds provider dependencies and unifies `flate2`
from 1.1.10 to the provider's reviewed 1.1.9 pin, removing unused
`miniz_oxide` 0.9.1 and `zlib-rs` 0.6.8. The primary explicitly granted this
resolver change.

Focused 96×64 page evidence on win-002: an initial `RawLocal` direct frame
rasterizes/uploads one tile for 6,144 pixels; an unchanged frame has zero
damage, raster, and uploads. Translation and opacity updates retain two tile
resource IDs, rasterize/upload zero tiles, and match the full shaped painter
pixel for pixel. Their damage is 5,888 and 5,440 pixels respectively versus
6,144 pixels for a full repaint. An 8-pixel scroll retains resource IDs and
has zero raster/uploads; its pixels match a freshly composed full frame at
the same scroll offset. Explicit item invalidation rasterizes one tile.
Scene rebuild produces a fresh full-resource frame; controls select
`BakedFallback`, and a rotated page uses the CPU-composed presenter path.
The final win-002 gate on the changed source passed isolated shell nextest
**60/60**, root workspace nextest **1,960 passed / 5 skipped**, and separate
root workspace doctests **2/2**. The five changed shell file hashes matched
before and after that gate. The isolated shell has no library target, so its
separate `--doc` invocation reports N/A.

The win-001 IA tree was refreshed to `82d273a`, then its five changed shell
source/manifest/lock files were re-synced; all five SHA-256 values matched
the Mac files. A before/after actual GPUI window screenshot remains pending
the user's win-001 RDP reconnection. No visual-regression claim is made from
the automated pixel tests.

## IA-REFTEST-PROTOCOL — TestRendered capture boundary (2026-09-24)

This independent driver ticket starts from fixed pushed `origin/dev`
`82d273a`, without the unpushed IA-COMPOSITOR shell files. The win-001 IA
tree was refreshed once at that base. Ordinary `render` retains its existing
script/checkpoint/capture behavior. `render-wpt` keeps the JS-4 scheduler
alive across an actual first style/layout/paint, dispatches one
`TestRendered` event, drains its checkpoint, and advances a bounded virtual
frame clock while the document root has `reftest-wait`. It then rebuilds CSS,
fonts, media, layout, and paint from the current DOM. A wait that survives
120 checkpoints exits with explicit unsupported status and writes no PPM;
script errors also write no PPM.

The frozen four-pair `css/css-lists` selection at upstream revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` has selection SHA-256
`90182d9a04ea9e052b2e50b9548ac22eb2d5e09c3ea31530ed248313b94c9129`.
Its 800×600 manifest records exact Git blob and SHA-256 values for all four
test sources, four references, and `/common/reftest-wait.js`; all nine staged
win-001 WPT files matched. Two BEFORE runs were identical at **1 pass, 2
mismatches, 1 crash**. Two AFTER runs were identical at **1 pass, 1 mismatch,
1 crash, 1 unsupported**. The only gain is
`list-item-dynamic-padding` mismatch→pass, with actual image SHA-256 changing
from `c86a97b952e37dd72d00ce6a77ae4d8090f03c0fc10438fc247c311847de10f4`
to the reference hash
`5f14e0d16560e649190a55fb194432f91f68517bb3c1259e128853a980d43269`.
`marker-dynamic-content-change` pass→unsupported is an exposed previous
unchanged-image false pass: the event handler's `classList = "tweak"`
assignment fails on a read-only property, so the wait class stays. The
gradient case remains a page-script crash; static `marker-counter` retains
its exact actual/reference image hashes and mismatch classification. No
net conformance gain is claimed. The exact reports, hashes, and transitions
are under `crates/amazing/tests/reftest_protocol/evidence/`.

The marker page's Element.classList `[PutForwards=value]` setter and the
gradient page's bare global `addEventListener` receiver are lower-crate seams
routed in the lane questions file. Neither behavior was simulated in the
driver. Owned fixtures cover first-paint ordering, one event, synchronous,
timer and animation-frame class removal, fresh layout/paint, wait timeout,
thrown scripts, and static output parity; they earn no WPT credit.

Final locked win-002 gates on the fixed-base tree passed: `amazing` nextest
**147 passed / 3 skipped**, root workspace nextest **1,967 passed / 5
skipped**, and separate root workspace doctests **2/2**. The `amazing`
package has no library target, so its separate doctest command is N/A. The
five changed Rust source/test files, frozen selection, and unchanged root
`Cargo.lock` matched Mac SHA-256 before and after the final gate; the audit
is in `evidence/gate-summary.json`.


## IA-ANIMATIONS — driver sampling and event dispatch (2026-09-24)

On fixed base `ba83010663168cf959f2d02b20a3e3dcaadd790f`, the driver now
feeds CSS computed animation and transition longhands and parsed keyframes to
`CssAnimationEngine`, retains its timeline in the GUI and WPT renderer, samples
at each frame, applies supported width, color, and opacity values before paint,
and dispatches animation/transition events into the DOM scheduler. The root
`Cargo.lock` has the cargo-generated compositor dependency addition. Owned
fixtures cover sampled opacity, a width transition, animation/transition end
events, and final width/color/opacity pixels. CSS animation-origin cascade
integration is routed to CSS-16; styled document relayout is routed to layout.

The pinned three-pair WPT selection (upstream
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 800×600) was stable across
two BEFORE and two AFTER runs: **2 pass / 1 mismatch / 0 errors, timeouts,
unsupported, or crashes**. All actual image hashes were unchanged, so there is
no WPT gain. The `background-color-animation-in-body` pass has white-only
actual and reference images and receives no conformance credit. Selection,
source hashes, per-case hashes, and comparisons are in
`crates/amazing/tests/animations/evidence/`.

The locked win-002 nextest gates passed on the fixed base: `amazing` **153
passed / 3 skipped** and workspace **2,026 passed / 5 skipped**. The driver
package has no library target, so its separate doctest command is N/A.
Workspace doctests passed **2/2** on win-002.

## IA-GENERATED-IMAGES — decoded URL cache (2026-09-24)

On fixed local `dev` base `c9947a0` containing RC-17b, the page adapter now
loads CSS generated `content: url(...)` and marker images once per resolved URL.
`PageMedia` serves the same decoded image to layout's intrinsic-size query and
paint's URL query, while `<img>` keeps its NodeIdx lookup. GUI layout, media
loading, and paint use stylesheets with their source URL so the generated key
stays resolved across all three stages. Owned tests cover shared cache identity
with an `<img>` and final rendered pixels for generated content.

The pinned RC-17b seven-case generated-content selection at upstream
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 800×600, stayed **2 pass /
5 mismatch** in both after sweeps. All classifications and actual image hashes
matched RC-17b's prior after report, so this adoption claims **zero WPT gains
and zero losses**. Per-case reports and comparison are under
`crates/amazing/tests/generated_images/evidence/`.

Final locked win-002 gates passed: `amazing` nextest **155/155** (3 skipped),
workspace nextest **2,097/2,097** (5 skipped), and workspace doctests **2/2**.
The three source hashes matched the win-002 gate tree. The driver package has
no library target for a separate doctest run.

## IA-INVALIDATION-ORACLE — diagnostic coordinator (2026-09-24)

Fixed base: local `dev` `0075614` (DOM-21 render journal and CSS-15 invalidator). `AMAZING_INVALIDATION_ORACLE=1` keeps style state across WPT rendering updates, drains the DOM journal at a frozen epoch, validates node generations, and feeds attribute/state changes to `StyleInvalidator`. Stylesheet, viewport, structural, text and unknown mutations conservatively request a full style result. The candidate style is compared with a fresh computation before layout. The coordinator also rebuilds a separate full layout, display-list view and full-painter surface; it compares styles, layout geometry/fragments, paint item order/property trees and final RGBA bytes before presenting the compositor candidate. Any difference presents the full result, records the first divergence and writes a PPM pixel diff when candidate pixels exist.

Current lower-crate limits: layout and paint have no retained `GeometryDelta`/display-list update seam. The coordinator therefore runs full layout and paint on every update and does not claim the ADR's frame-time or property-only zero-work targets. A request for those additive seams is in `.agents/integrator-assist/questions.md`.

Owned diagnostics cover a TestRendered class mutation and COMP-7's effects scroller at 512×512; both preserve the default PPM bytes and report compositor/full parity. A direct journal unit test proves one stable attribute mutation avoids a full style fallback, then a structural mutation selects full style. Linux locked crate gate with isolated storage: 169 passed, 1 skipped; this is iteration evidence, not the Windows final gate.

Windows diagnostic finding: the 512×512 effects scroller has a one-channel-level compositor/full difference at (38,178), candidate `[95,95,95,255]` versus full `[96,96,96,255]` on both frames. Linux matches exactly. The oracle falls back to the full painter and writes a PPM diff. This is routed to compositor/replaced-content; the effects case has no exact-pixel credit until fixed.

## IA-ORACLE-LAYOUT + IA-ORACLE-PAINT — retained candidate (2026-09-24)

On fixed local `dev` base `93bb98b`, the invalidation coordinator now passes CSS changed properties and DOM text/replaced inputs to `LayoutTree::update`, compares its retained root with an independent full layout, and paints from the retained root. It compares stable keyed paint snapshots, translates safe `DisplayListView::update` results into compositor paint deltas, and retains the compositor/presenter across frames. A parity loss presents the full painter and resets the scene. The default renderer remains unchanged.

The Win-001 release mutation trace passed its direct-render byte comparisons, with full-painter fallback for 21 known one-channel effects differences. Article and effects p95 were **266.614 ms** and **248.500 ms**, missing ADR 0010's 20/31 ms targets. Opacity p95 was **110.244 ms**, with 20/20/20 style/layout/paint-view calls over 20 mutations and zero tile uploads; property-only zero work is not met. The driver trace includes a full oracle and resource work, unlike the ADR's compositor-only baseline. Exact counts and command are in `crates/amazing/tests/invalidation_oracle/evidence/layout_paint_win001_2026-09-24.md`. Layout still rebuilds width changes and paint still compares complete views; follow-up owner requests are in `.agents/integrator-assist/questions.md`.

## IA-COMP-ADOPT — rendering-update property scene (2026-09-24)

The rendering-update coordinator creates a generation-aware retained property scene from `PaintViewSource::property_bindings()`. CSS attribute classification runs before style, layout, and paint; a validated compositor-only class or inline-style delta updates the scene and cached CSS values with zero stage rebuilds. COMP-12's `scene.on_vsync` is called at the WPT frame checkpoint for active CSS handoffs. RC-20's retained paint view is seeded with DOM generations and accepts LAYOUT-17 keyed damage when its mapping is unambiguous; `AmbiguousGeneration` requests full paint. Unsupported RC-20 seeds keep the RC-19 paint-delta path. Any validation or parity loss uses the full painter.

The Win-001 release oracle trace on `58df3b15` passed. The document `<style>` opacity class fixture took **0/0/0** style/layout/paint-view calls and zero uploads over 20 frames. Effects had **40 layer-rounding pixels** (two per frame), zero parity losses, and zero fallbacks. The article/effects diagnostic p95 of **149.999/120.875 ms** includes the independent full recomputation and must not be compared with ADR 0010's frame-time targets. Win-002 locked gates on the same production code passed: `amazing` 184 tests, workspace 2,377 tests, and two doctests. Evidence and direct-full baselines are in `crates/amazing/tests/invalidation_oracle/evidence/comp_adopt_win001_2026-09-24.md`.

An opt-in `AMAZING_INCREMENTAL_PRODUCTION=1` candidate path skips the full oracle; the timing trace also keeps pixel classification outside both timers. On Win-001, candidate p50/p95 was **71.364/77.389 ms article**, **88.560/97.082 ms effects**, and **37.759/40.775 ms opacity**. Direct full p50/p95 on the same mutations was **31.417/32.705**, **28.006/31.101**, and **16.301/19.246 ms**. Article/effects remain above the ADR's 20/31 ms candidate targets because their production path still rebuilds paint; opacity keeps 0/0/0 and zero uploads. Full data and Windows gate counts are in `crates/amazing/tests/invalidation_oracle/evidence/production_timing_win001_2026-09-24.md`.

## IA-GENERATED-IMAGES follow-up — data SVG (2026-09-24)

On fixed local `dev` base `544d7e8`, the generated-image URL cache now decodes
`data:image/svg+xml` with the bounded SVG decoder, including percent-encoded
and base64 data URL bodies handled by Fetch. Raster data URLs continue through
the media decoder. A page test proves a data-SVG `list-style-image` enters the
URL cache, exposes its intrinsic size, and paints red pixels.

## IA driver render test isolation (2026-09-24)

The `css3_stylesheets` and `render_wpt` integration fixtures now give every
renderer subprocess a unique storage profile. Their temp/output paths also
include a process ID and timestamp to avoid reuse across runs. The earlier
IPC 50-cycle stress logs showed both named tests failing in all 50 runs when
the shared profile was corrupt. On fixed local `dev` base `544d7e8`, 50 new
full-workspace parallel Linux cycles selected and passed both tests, and each
workspace run exited 0 with **2,154 passed / 5 skipped**. The per-cycle summary
is under `crates/amazing/tests/driver_flakes/evidence/`.

## IA-STORAGE-EVENTS — cross-document localStorage delivery (2026-09-24)

On fixed local `dev` base `f676798`, a successful localStorage mutation queues
one `storage` task on each other live document in the same profile and storage
key. The receiving scheduler admits its inbox after pre-existing JS requests,
and delivers a `storage` event to window listeners and `onstorage` with exact
UTF-16 key and values, source URL, and the receiving document's Storage
wrapper. Equal-value writes and the source document emit no event. A driver
test covers set, remove, clear, same-origin filtering, task timing, and listener
removal. Session storage remains per arena: a browsing-context-group seam from
storage is needed before sessionStorage can share data and notify related
documents.

## IA-ANIMATIONS-2 — CSS sampled style adoption (2026-09-24)

On fixed local `dev` base `544d7e8`, the animation scene now serializes
compositor samples into `AnimatedValues`, preserves animation and transition
origins while a transition runs, and removes the transition origin after its
end/cancel event. The driver passes those values to
`compute_with_animations` for the rendering style and media/paint resources.
The existing stylesheet-based layout entry point and its sampled box-style
application remain until LAYOUT-14 item 8 provides `layout_styled_document`.
`PropertyUpdate` currently omits an origin field, so the driver classifies
samples by the compositor's documented animation-before-transition order and
transition lifecycle events; a compositor origin field would remove that
inference. Tests cover animation and transition cascade sources and retained
precedence.

## IA-ANIMATIONS-2 — styled layout adoption (2026-09-24)

On fixed local `dev` base `ac04822`, WPT rendering and the Windows GUI pass
their sampled `StyledDom` directly to `layout_styled_document`. Media, layout,
and paint therefore use the same animation and transition cascade result. The
old box-tree style mutation and second layout pass are removed. The
invalidation oracle keeps its plain-style candidate check and lays out its
independent reference from freshly computed sampled styles. A render test
checks that an important width declaration beats the sampled animation in
layout; existing tests cover animated width, color, opacity, and transition
events.

Locked win-002 `amazing`, workspace nextest, and workspace doctest gates passed.
The pinned Win-001 three-case animation render selection at WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` stayed **2 pass / 1 mismatch**
before and after. Every actual image hash and differing-pixel count matched;
the remaining `animation-important-002` mismatch is unchanged. Reports and
comparison are under `crates/amazing/tests/animations/evidence/layout-*.json`.

## 2026-09-24 pause handoff: production timing, viewport CSS, parallel tiles

Production timing is committed in the rebased series `625b4839` + `52b0f5fb` (original fixed-base Win-001 evidence at `71527381`). Oracle-free candidate p50/p95 ms versus direct full: article 71.364/77.389 vs 31.417/32.705; effects 88.560/97.082 vs 28.006/31.101; opacity 37.759/40.775 vs 16.301/19.246. Opacity had 0/0/0 style/layout/paint-view calls and zero parity losses; article/effects still miss ADR 0010 targets. See `crates/amazing/tests/invalidation_oracle/evidence/production_timing_win001_2026-09-24.md`.

Viewport CSS driver adoption is `7c83a744` on CSS-15. Render, layout dump, GUI repaint, and animation sampling pass the actual viewport to `compute_with_environment`. Its exact-code predecessor `6ce2a16e` passed Linux `amazing` 183/183 (3 skipped), including the two-width root `vw` pixel test, and the Win-002 `amazing` crate gate. The Win-002 workspace gate stopped on compositor-owned `paint_view_owner_mapping_binds_dom_properties_to_tree_nodes` (root scroll node increased owner bindings from 3 to 4); doctests did not run. Request is in `.agents/integrator-assist/questions.md`.

COMP-14 driver adoption is `5cf0e3f4` on owner API `e28f903a`. The retained scene owns one `TileRasterPool`, schedules from `Arc<RetainedPaintView>`, keeps the prior complete surface while workers run, and atomically presents ready tiles at vsync. If a paint delta requires a new scene, its owned snapshot can still be scheduled. Linux `amazing` compiled: 183 other tests passed and the focused parallel-vsync test passed after the fallback fix (3 ignored). A full combined Win-002 gate, Win-001 render trace, and production timings after this change remain for RESUME. The branch contains the compositor owner commit provisionally; rebase onto its eventual `dev` merge before final gate.

On RESUME, also adopt HTML's frame phases in GUI: its scheduler now orders resize/scroll, rAF, then a render hook. Move GUI frame cadence into that hook, call compositor `on_vsync` there, and retain the WPT harness's deterministic 16,667us steps.

### IA-COMP-14 completed — combined gate and post-adoption trace (2026-09-26)

Resumed on `lane/ia-comp-adopt` after the Mac restart. Merged local `dev`
`59492359`/`34ec9a9c` (the Boat→venus and win-002→win-001 fallbacks) into the
branch; HEAD `8dde93ee`. Linux venus-001 `amazing` + `amazing-compositor`
nextest: **246 passed / 3 skipped**. The former workspace blocker
`paint_view_owner_mapping_binds_dom_properties_to_tree_nodes` is cleared by
COMP-13b `6ca61021`: the document viewport scroll owner is a distinct node from
the nested element's transform/opacity/scroll bindings.

Win-002 was offline, so the combined gate ran on win-001 with the same
toolchain through `scripts/win-run.sh`: workspace nextest **2453 passed /
8 skipped** (124 binaries) and workspace doctests **2 passed / 0 failed**. The
win-001 release `incremental_production_mutation_trace` passed 1/1; its absolute
p50/p95 numbers are inflated because the host carried two other lane build
slots (full p95 reached 1.03 s), so the clean 2026-09-24 reference stands for
timing. Invariants held: opacity 0/0/0 with zero uploads; effects 40
layer-rounding pixels, zero losses. Full data and the exact commands are in
`crates/amazing/tests/invalidation_oracle/evidence/comp14_win001_2026-09-26.md`.
The article `RequiresFullRebuild` paint fallback and effects
`AmbiguousPropertyOwner` seed fallback remain routed to their owners.

Boat named snapshots are unavailable while the account has zero-data-retention
enabled, so `boat-lane.py ensure` cannot provision lane sandboxes; `linux-run.sh`
falls back to venus automatically. Routed in `.agents/integrator-assist/questions.md`.

### IA-FRAME-PHASES — done (2026-09-26)

The driver EventLoop emits `Action::Render` at `RenderPhase::BeforeAnimationFrames`
after resize/scroll and at `AfterAnimationFrames` after rAF. `DriverScheduler`
previously ignored those actions, so callers self-drove rendering outside the
loop. This ticket adds `DriverScheduler::pump_at_with_render` and
`pump_now_with_render`, which invoke a host hook for each render action while
`pump_at`/`pump_now` keep ignoring them (`|_, _| {}`). The WPT reftest loop now
ticks the retained compositor's `RenderCoordinator::on_vsync` from the
`AfterAnimationFrames` step, so the sample stays at the same frame boundary as
before (after rAF mutations, before the next loop step) but is sourced from the
render hook; the virtual 16 ms steps are unchanged. The driver Win32 GUI's
animation sample and event/update capture move into the same hook in
`pump_timers`, after rAF callbacks have run.

Commits on `origin/dev` `f5f97c39`: `939942bb` (hook + ordering test),
`2e80541c` (WPT `on_vsync` and GUI sample from the hook). Linux Boat `-p amazing`:
**191 passed / 4 skipped**, including `cmd::render::reftest_protocol_tests::*`,
the oracle traces, and `js_scheduler::tests::driver_render_hook_brackets_animation_frames`,
which asserts the hook sees `BeforeAnimationFrames` then `AfterAnimationFrames`
around the rAF callback. Win-001 combined gate (win-002 offline): workspace
nextest **2483 passed / 9 skipped** (124 binaries) and workspace doctests
**2 passed / 0 failed**.

The driver Win32 GUI has no retained compositor scene, so its hook samples
animations but cannot tick a compositor; the GPUI shell drives frames from
`amazing_js::Interpreter::pump_timers` and never calls a compositor `on_vsync`.
Both are routed in `questions.md` as follow-ups (shell grants / a retained GUI
scene). A first win-001 gate failed on `os error 112` (C: 0 GB free); the lane
purged its own `debug\incremental` and stale `ia-*` evidence folders, and the
orchestrator freed the rest. The retry gate is the measurement above.

## IA-REAL-PAGES — headless bulk browsing corpus (2026-09-26)

Built `tests/real-pages/`: 52 synthesized everyday pages (news, docs, wiki, shop,
web-app shells) plus 30 shared subresources, generated deterministically by
`tests/real-pages/generate.py`. Ten pages carry one unguarded `feature-*.js`
probe apiece so missing globals/interfaces surface per page. The corpus is run
headless by `crates/amazing/tests/real_pages.rs` through a new lenient
`amazing render-page` command (`crates/amazing/src/cmd/real_pages.rs`), which
runs every script, inline handler and bounded event-loop step, reports each
failure, and always paints a result. The harness spawns one child per page so a
crash is attributed, and fails on a crash, panic, timeout, blank render or
missing image.

Result on the merged head `5d484278` (includes local `dev` IPC-11 and the
fleet-expansion crates; Linux Boat, viewport 1024): **52/52 pages ok, 0
crashes/panics/timeouts, all non-blank**, 93-496 ms in-driver, 9,581 minimum
non-white pixels. Two consecutive runs were byte-identical in status, error
count, fallback count and non-white pixels. Six pages reported one
missing-feature error each. Evidence:
`crates/amazing/tests/real_pages/evidence/run.tsv` and `feature-matrix.md`; the
full per-page table is in `.agents/integrator-assist/report.md`.

Triage (routed to the owning lanes' worktree `questions.md`):

- js-lang: non-optional member access on `null`/`undefined` returns `undefined`
  instead of throwing (`interp.rs` `ExprKind::Member` → `get_prop_receiver`).
  This hides page bugs (`news-broken-script` reports zero errors, and `app-mail`
  surfaces as `value is not callable` instead of the real missing member).
- js-date + js-runtime: `Date.now` and `Date.parse` are undefined.
- dom-bindings: `navigator` is now present, but `navigator.clipboard`,
  `matchMedia`, `getComputedStyle`, `requestIdleCallback`, `WebSocket`,
  `IntersectionObserver` and `Element.prototype.animate` are missing; net and
  css notes attached for the transport and media/computed-value seams.

Driver-owned: the lenient headless path and the corpus harness; no driver crash
was found. Ordinary `render` and the strict reftest path keep abort-on-error.

## IA-LIVE-PAGES — live public-site headless corpus (2026-09-26)

Finished a prior agent's wip on `tests/live-pages/` (40 real public URLs;
`probe.js` surveys ~70 Window/interface globals) and
`crates/amazing/tests/live_pages.rs`. Fixed a real cross-lane hazard in both
this and the IA-REAL-PAGES harness: pooled Boat sandboxes share
`~/amazing/target`, so another lane's mid-run `cargo build -p amazing` can
swap `CARGO_BIN_EXE_amazing` under a running corpus loop; both harnesses now
copy the binary into a private per-run path before looping.

Result (Linux Boat, viewport 1024, 40 URLs): 0 crashes/panics; 37/40 `ok`, 1
stale-fixture 404 (fixed), 2 timeouts at the 150s cap on very JS-heavy real
sites (`docs.rs/serde`, `vuejs.org`). Full per-page/API/error/warning tables
regenerate via the command in the test's own doc comment (not checked in;
public-web evidence goes stale).

Driver-owned fixes: the binary-pinning bug above. Investigated two
aggregated warning clusters by reproducing them directly instead of guessing
from counts, and root-caused both to lower crates (not driver bugs): a CSS
`unicode-range` tokenizer/serializer round-trip bug (`amazing-css`) and a
sfnt/WOFF2 decode short-read (`amazing-text-provider`); filed with exact
repro to `.agents/css/questions.md` and `.agents/text/questions.md`. Plus
three js-lang regex/module findings from real shipped JS and a 17-API
confirmed-missing list to dom-bindings (see `.agents/<lane>/questions.md`).

Mid-ticket, a storage-reported cross-lane regression (real-timer hangs
through `wpt_harness.rs`'s `pump_now`) was relayed here; checked it against
this head (workspace nextest clean, 2747/11 skipped/0 failed) and against
this harness's own code path (`pump_at`, render-hook-disabled per
IA-FRAME-PHASES) and found neither implicated — filed as negative evidence
to `.agents/storage/questions.md`. Did not touch the render hook or task
queue ordering.

## IA-A11Y-ADOPT / IA-DEVTOOLS-ADOPT — accessibility tree and DevTools host seam (2026-09-26)

Adopted two crates the a11y and devtools lanes had asked the driver to wire
up, in the same series as IA-LIVE-PAGES.

**`amazing axtree <page.html|url> [viewport_width]`**
(`crates/amazing/src/cmd/axtree.rs`): fetches/parses a page exactly like
`render`/`render-page`, resolves real author stylesheets via
`page::collect_author_css` (inline `<style>` *and* `<link rel=stylesheet>`,
not just the a11y crate's own inline-only convenience entry), computes
style+layout with the fallback text system and `NoReplacedElements` (same
limitation as `amazing_a11y::build_document`), and prints
`amazing_a11y::build(&dom,&styled,&layout).snapshot().dump()`. No changes
needed in `amazing-a11y` — the crate anticipated this seam (`SchemaSnapshot::
dump()` doc comment: "used by tests and the CLI").

**`amazing devtools <url> [listen-addr]`**
(`crates/amazing/src/runtime_host_devtools.rs`): implements `DevToolsHost`/
`DevToolsSession` over a real fetched page — `Page.navigate`,
`Page.getFrameTree`, `Page.getLayoutMetrics`, `Runtime.evaluate` — and owns a
`CdpServer::bind(...)` on a background thread, per the devtools lane's exact
request in `.agents/integrator-assist/questions.md`. Key design point: the
session type is `Arc<Mutex<PageState>>` over a *plain* `amazing_dom::Dom`
(no `Rc<RefCell<BrowserDomHost>>`), because `DevToolsSession: Send` and `Rc`
is never `Send` for any `T` — the real `DriverScheduler`/JS interpreter (which
*is* `Rc`-based, per the single-threaded-execution locked decision) is built
fresh inside each `evaluate()` call instead of stored on the session, so it
never crosses the host→connection-thread boundary. Consequence, documented
rather than hidden: `Runtime.evaluate` gets a fresh JS realm per call — DOM
mutations persist across calls (plain owned data), JS globals do not. A
persistent realm needs a dedicated JS thread the session talks to over a
channel; noted as a follow-up, not attempted here.
`Page.captureScreenshot` stays the crate default (`Unsupported`): no
PNG/JPEG/WebP encoder exists in this driver yet, only BMP. DOM/CSS/Input
domains are left for the next slice, consistent with the crate's own
"adopt domains one at a time" design (every `DevToolsSession` method
defaults to `Unsupported`).

Both are additive dependencies (`amazing-a11y`, `amazing-devtools`, each
already std-only) in `crates/amazing/Cargo.toml`; `Cargo.lock` updated to
match. Smoke-tested against real pages/URLs (a real `<button>`/`<p>` tree;
`/json/version` and `/json/list` served correctly for `https://example.com/`
over a bound TCP port), plus 5 new unit/integration tests in
`runtime_host_devtools.rs`.

Gate (Linux Boat, head `134ccaf6` plus this series): `-p amazing` nextest 224
passed/5 skipped (no lib doctests — `amazing` is bin-only); workspace nextest
2747 passed/11 skipped/0 failed (85.5s); workspace doctests 4 passed/0
failed. Zero losses.

## IA-LIVE-2 — real-pages cluster table, docs.rs/vuejs.org timeout root cause (2026-09-26)

Merged `dev` (4bab124d: IA-LIVE-PAGES + a11y/devtools merged; brought in the
orchestrator's own main.rs conflict resolution). Two follow-ups from that
merge, then the ticket itself.

**Ownership fold.** a11y had already resolved A11Y-6 itself
(`runtime_host_a11y.rs`, `amazing a11y-tree`) rather than waiting on this
lane's earlier duplicate `amazing axtree` (`cmd/axtree.rs`). Per the
orchestrator: deleted `cmd/axtree.rs`, removed its `mod`/dispatch lines from
`main.rs`/`cmd/mod.rs`, and folded its one real addition — accepting an
`http(s)://` URL, not just a local file — into `runtime_host_a11y.rs::
ax_tree_for_page`/`cmd_a11y_tree` (plus the `DocumentKind::Xml` "unsupported"
guard `axtree.rs` had that the a11y version didn't). `runtime_host_devtools.rs`
ownership also moved to the devtools lane (CDP-16 continues it); this lane
made no further edits there.

**Cluster table.** Extended `crates/amazing/tests/real_pages.rs` (the local
52-fixture IA-REAL-PAGES corpus, no network) to capture renderer warnings —
it only captured script errors before — and print the four IA-LIVE-2 buckets:
JS exceptions by message, missing-API fallbacks, a CSS/parse (renderer)
warning histogram, and explicit panic/timeout counters, mirroring
`live_pages.rs`'s existing collapsing (`warning_kind`). Current numbers: 52
pages, 4 errors, 4 fallback pages, **0 crashes, 0 panics, 0 timeouts**; new
signal: **121x `UnsupportedSvg`** across the corpus (7 per page on every page
using the shared thumbnail/logo SVG fixtures) — routed to replaced-content
with an exact repro, plus a confirmed side-finding while reading
`amazing-svg`: `<text>` elements only ever render when `font-family` contains
the literal string `"Ahem"` (the WPT test font), so real-world SVG text is
silently a no-op today, independent of the `UnsupportedSvg` cause.

**docs.rs/serde and vuejs.org timeouts, root-caused.** IA-LIVE-PAGES's live
40-URL run had 2 timeouts (150s child-process ceiling each); the storage
event-loop regression that prompted holding off on a full live-corpus rerun
had *not* landed on `dev` yet (`git log dev -- crates/amazing/src/
js_scheduler.rs` showed nothing since the report), so this ticket worked the
two known timeouts directly instead of re-running the full live corpus.

- **docs.rs/serde (fixed, driver-owned):** `PageFontLoader` (`font_loader.rs`)
  had no per-page fetch cache, and a `PageFontLoader` is constructed fresh
  once *per `@font-face` rule* in `page.rs::load_page_text_from_sheets`. This
  page declares "Font Awesome 5 Free/Brands", "Font Awesome 6 Free/Brands"
  and a plain "FontAwesome" alias, all pointing at the same 4 URLs — 12
  redundant fetch attempts of those 4 URLs, plus 5 unique webfont URLs, each
  paying its own 20s `fetch_cors` deadline against an unreachable host (17
  sequential attempts, ~279s observed). Added a per-page `HashMap<resolved
  URL, Result<Vec<u8>, String>>` cache threaded through `PageFontLoader`
  (`cache` field) from a single map owned by `load_page_text_from_sheets`, so
  a repeated URL reuses the first result instead of re-fetching; still warns
  once per `@font-face` rule for triage parity. New regression test:
  `font_loader::tests::http_font_source_is_only_fetched_once_per_page_even_
  when_reused_by_multiple_font_face_rules` (counts real server connections).
  Measured: 279s -> 190s (32% faster) on the same Boat sandbox. Still exceeds
  the live corpus's 150s child-process ceiling — 9 *distinct* font URLs on
  this page are genuinely unreachable/slow from this network, each still
  paying its own 20s deadline (9 x ~20s ~= 180s); that residual is a
  legitimate "many distinct broken font sources on one page" cost, not a bug,
  and is noted as a follow-up (shorter default per-font-fetch deadline, or a
  larger live-corpus per-page ceiling) rather than fixed here.
- **vuejs.org (root-caused, not fixed — outside this lane's owned crates):**
  isolated with a minimal repro (a local page with nothing but `<link
  rel="stylesheet" href="https://vuejs.org/assets/style.BMnqHME7.css">` — no
  scripts, no other resources) that still hangs 60s+ with *zero* output.
  Ruled out reachability/DNS/TLS directly (every one of the 14 distinct
  same-origin font URLs that stylesheet's 28 `@font-face` rules resolve to,
  after the same dedup fix above, fetches in well under 1s via plain `amazing
  fetch`/`curl` from the same sandbox). Zero warnings during the hang means
  every fetch eventually *succeeds*, just far slower in aggregate than the
  plain blocking fetch path used everywhere else in the driver — the one
  difference is `font_loader.rs::fetch_cors`'s use of `amazing_fetch::
  service::NetworkService`/`FontRequest` (a 2ms-interval poll loop) instead of
  a single blocking call. Routed to net (`amazing-fetch`/`amazing-net` are
  net-owned) with the exact repro; not attempted here since the dispatcher
  internals are outside `crates/amazing`.
- Also added an overall wall-clock budget guard around the whole script phase
  in `execute_page_scripts_lenient` (`cmd/js.rs`) — handlers, scripts, the
  DOMContentLoaded/load dispatch and the 16-step event-loop pump now share one
  deadline (`AMAZING_SCRIPT_PHASE_BUDGET_SECS`, default 45s) instead of only
  having each individual sub-operation bounded; remaining work is skipped and
  reported (not silently eaten) once the budget is spent. This is a general
  defense for any future page whose *script* phase chains many blocking
  network primitives (each already capped at 10-15s in `runtime_host_net.rs`)
  faster than one could reasonably notice; it does not change behavior for
  any page that finishes within budget, and it did not end up being the
  vuejs.org fix (that hang is in the paint-phase font stylesheet, not script
  execution) but is kept as a real, tested safety net either way.

**Infra note:** the shared Boat sandbox (`bx_pvqc5usm`, pool `pool-dom`) went
from 3 GB to 0 GB free across this ticket's `linux-run.sh` calls; flagged to
the orchestrator in `.agents/integrator-assist/questions.md` since this lane
has no `boat` command access to investigate.

Gate (Linux Boat, `dev` head `99df2b68` plus this series): `-p amazing`
nextest 257 passed/6 skipped; workspace nextest 3021 passed/12 skipped/0
failed (71.7s); workspace doctests all passed (0 failures, `amazing_js`/
`amazing_html` included). Zero losses.

## IA-LIVE-3 (2026-09-26)

Took the 17 "missing on 100% of live pages" APIs filed in IA-LIVE-2 and
checked `.agents/dom-bindings/questions.md`: none of them had been picked up
there yet (dom-bindings' queue was WebSocket/IPC/event-handler-attribute work,
no overlap). Implemented the driver-owned subset — window/navigator/
document-level host APIs whose *value* comes from the driver, as opposed to
WebIDL interface scaffolding — in a new lane-owned file,
`crates/amazing/src/runtime_host_integrator_assist.rs` (this ticket's entire
footprint beyond a handful of one-line hooks in the shared
`runtime_binding_host.rs`, per the driver-adapter-per-lane rule):

- `navigator.userAgent/language/languages/platform/onLine/cookieEnabled/
  appVersion/hardwareConcurrency` — real driver/OS values (`std::env::consts::
  OS` mapped to Win32/MacIntel/Linux x86_64).
- `performance.now()`/`performance.timeOrigin` — a real per-page
  `Instant`-based clock (`IaRuntime::created_at`), not a stub.
- `window.matchMedia(query)` — evaluates the query against the driver's real
  per-page viewport by reusing `amazing_css::MediaQueryList::parse`/
  `.evaluate(&MediaEnvironment)` (no new CSS media-query grammar). Wired the
  real viewport through: `cmd_render_page` (`real_pages.rs`) already has
  `width`/`fixed_height` in scope, so it now passes `(width, viewport_height)`
  into `execute_page_scripts_lenient` (`cmd/js.rs`, new 4th parameter) which
  calls the new `DriverScheduler::set_viewport` (`js_scheduler.rs`, one
  additive method, same shape as the existing `set_location_url` — did not
  touch the render-hook/event-loop code storage is working on there) ->
  `RuntimeBindings::set_viewport` -> `BindingState.integrator_assist`. Falls
  back to `MediaEnvironment::default()`'s viewport when no `fixed_height` is
  given. `addListener`/`addEventListener`/`removeListener`/
  `removeEventListener` on the returned object are defensive no-ops (nothing
  ever fires a `change` event yet) so page code that defensively registers a
  listener doesn't throw; a live-updating `MediaQueryList` interface is left
  to dom-bindings.
- `window.requestIdleCallback`/`cancelIdleCallback` — reuses the existing
  `Interpreter::schedule_timer`/`cancel_timer` driver timer queue; the
  callback receives an `IdleDeadline`-shaped object (`timeRemaining()`
  always 0, `didTimeout` always `false` — this driver has no real idle
  detection, so it never claims a timeout it can't back up).
- Explicitly NOT attempted (left to dom-bindings, per the ticket): a live
  `MediaQueryList` interface with real `change` events, a real `IdleDeadline`
  interface/type, `PerformanceObserver`/marks/measures/entries.

**Bug found and fixed while gating:** the first `RequestIdleCallback` cut
defaulted its timer delay to 1ms when no `timeout` option was given. The
timer queue backing it (`amazing-js/src/interp.rs`) compares against a real
`Instant::now()`, not a simulated clock, so a nonzero real delay makes firing
depend on genuine wall-clock progress between scheduling and the next pump —
exactly the flake this codebase's own convention avoids elsewhere
(`AbortSignal.timeout(0)` in `js_scheduler.rs` uses delay 0 for the same
reason). It passed once on a slower/contended sandbox, then failed
deterministically (5/5 reruns, isolated) on a fresh, faster one. Changed the
default to `0` (fire as soon as the loop is next pumped, matching the
existing convention); reran the specific test 5/5 clean before and after the
full workspace gate.

Real-pages corpus (52 pages), before -> after this ticket (Linux Boat,
`cargo test -p amazing --locked --test real_pages -- --nocapture`, code
before/after compared via `git stash`/`stash pop` on the same sandbox): 4 ->
3 page-level errors, 3 -> 2 fallback pages. `app-dashboard.html`'s `page
script: ReferenceError: matchMedia is not defined` is gone entirely (0 errors,
0 fallbacks on that page now). The remaining 3 errors (`app-chat.html`
"Invalid document URL", `app-kanban.html`/`app-mail.html` "value is not
callable") are unrelated to this ticket's APIs.

Live corpus (40 URLs, `cargo test -p amazing --locked --test live_pages --
--ignored --nocapture`, 2315s): none of `matchMedia`, `navigator.clipboard`,
`getComputedStyle`, `requestIdleCallback`, `cancelIdleCallback`, or
`WebSocket` — all part of the originally filed 17-API list — appear in the
after-run's top-20 missing-API table any more (0/36 missing, down from
100/100% before this ticket, per the same table this ticket's filing was
based on). The after-run's top-20 is now entirely interface-level APIs
(`MediaQueryList`, `IntersectionObserver`, `Element.prototype.animate`,
`ResizeObserver`, `PerformanceObserver`, `Permissions`, `Notification`,
`EventSource`, `CSS`, `RTCPeerConnection`, `SharedWorker`, `TextEncoder`/
`TextDecoder`, `URLPattern`, `WebAssembly`, `WebGLRenderingContext`, `Worker`,
`XMLSerializer`, `AudioContext`, `OffscreenCanvas`) — all correctly out of
this driver-owned ticket's scope.

**Infra note:** a background gate run duplicated itself when a "rerun"
request landed before the first attempt was actually stopped, leaving two
identical `cargo nextest run --workspace` invocations serialized against
each other on the same lock (both then reported empty output, which does not
mean success). Caught by checking `ps` directly against the claimed
completion before trusting it, rather than committing on an unverified
"finished" report.

Gate (Linux Boat, Boat sandbox `bx_e3cjmk4g`/pool-ia, this series):
`-p amazing` nextest: `request_idle_callback_runs_the_callback_with_an_idle_
deadline_shape` + the other 4 new tests, 5/5 clean reruns after the fix.
Workspace nextest: 3053 passed/13 skipped/0 failed (64.1s) -- up from
3052 passed/1 failed before the fix (zero losses, one flake resolved).
Workspace doctests: all passed (0 failures).
