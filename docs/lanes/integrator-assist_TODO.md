# Integrator assist decision map

## IA-1 — Split the driver and migrate natural image dimensions

Base: `ea57978ee5e6a776ff9e278e4973df93471988ae` (`origin/dev`).

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
intrinsic-size adapters. `engine_viewport.rs` remains outside this ticket's grant. Its separate adapter
still returns attribute-adjusted `used_size` and is routed through `questions.md`
to the existing owner; this ticket does not claim that shell bug is fixed.

### Validation

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

### Additional routing — in progress

The integrator granted the specific shell natural-size migration and assigned
root-relative WPT resource/reference resolution plus removal of server-side Git
from the render runner. The earlier evidence above measures the original ticket;
fresh validation will follow these newly authorized changes. CSS owns the HTML
presentational-hint follow-up; adjusted intrinsic sizes must not be restored.

## IA-2 proposal — shell/driver URL migration

Follow the caller inventory and persisted-key concerns in
[net_TODO.md](net_TODO.md). Replace legacy `Url` use in driver resource resolution,
DOM URL completion, and shell navigation with the canonical network URL API.
Then migrate cookie/cache keys, storage origins, and persisted session state with
an explicit old-key policy. Require grants for each shell file first, especially
`engine_viewport.rs`, which another lane currently owns.

Gate the migration with relative-resource/navigation fixtures, canonical-origin
and default-port cases, persistence round trips, the pinned URL selection, and
actual Windows shell screenshot proof. Keep it a separate ticket: URL behavior
must not be folded into this command extraction.
