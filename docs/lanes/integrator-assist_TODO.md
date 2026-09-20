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

Authorized alongside IA-XML, but checkpointed separately. Inspected the actual
`BindingHost`, `InterfaceDefinition`, `Member`, `Operation`, `install/INTERFACES`
and `invoke` seam at DOM checkpoint `2b917f8` (including `afa9ad5`), read-only in
the DOM lane. The runtime owns the `BindingHost` implementation; the driver must
not create a parallel binding trait or wrapper cache.

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

Requested dependency merge through co-integrator, without blocking acceptance
of native-only lower-crate evidence on later driver WPT installation. The current
runtime source has no `BindingHost` adapter/installer. The existing driver →
`amazing-js` dependency may suffice if the runtime exposes installation. If it
requires direct binding types, the exact additional edge is driver `amazing` →
`amazing-bindings`; manifest/lock updates are routed, not made speculatively.

Validation after dependencies land must show an API is visible in actual driver
WPT execution, mutations reach the parsed arena, factory/query/new-interface
node identity agrees, and a retained `childNodes` object updates after mutation.
Use a newly installed API (for example Text construction or append) and report
measured before/after separately from native DOM/binding counts. This map earns
no WPT credit and does not suspend the active no-fallback/no-scripting XML work.
