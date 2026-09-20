# RC-1 replaced-content evidence

All rendering and builds run on win-001. Revision:
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` (WPT), viewport 800×600,
device scale 1. Baseline engine commit: `e9c1e21`.

## Selections

`manifest.json` is a deterministic dependency-cluster diagnostic: eight
SHA256-path-ranked HTML `rel=match` tests per direct-source feature marker.
There are 48 cluster entries and 41 unique upstream tests; clusters overlap.
The candidates are recorded separately in `inventory-*.json`, with counts in
`inventory.json`. `inventory.ps1` recreates them from an unmodified pinned
WPT checkout. Tools/templates are excluded. The screen does not recursively
expand script/CSS/resource dependencies and is not a corpus-wide causal claim.
It deliberately retains script-dependent and cross-feature cases so their
prerequisites remain visible. Nothing is selected based on a favorable result.

`image-manifest.json` is a 12-case static PNG diagnostic: five object-fit modes
across the two upstream image aspect ratios, plus two object-position cases.
Its references use background-image painting and its tests use floated images;
those cross-feature dependencies are retained rather than rewritten.

`baseline-renderer.rs` preserves the small static image harness used before the
change. It uses the baseline engine crates with `AMAZING_IMAGE_BASELINE=1`,
which selects the existing BMP/PPM decoder. The after artifact is
`amazing-image/examples/render_reftest.rs`, built against the shell workspace
lockfile. Both harnesses use the same fallback font and layout/paint interfaces;
neither executes scripts or fetches network resources. They are diagnostic
artifacts, not substitutes for the production driver. The root driver's image
loader remains BMP/PPM-only; the production GPUI shell now uses the adapter.

## Reproduce on Windows

Set `CARGO_TARGET_DIR` to the lane's dedicated target directory. Use PowerShell
`-EncodedCommand` over SSH; cmd.exe is not suitable for the compound commands.
The examples below run from the lane worktree.

```powershell
cargo test -p amazing-paint -p amazing-media --locked
cargo test --manifest-path crates\amazing-shell-gpui\Cargo.toml -p amazing-image --locked
cargo check --manifest-path crates\amazing-shell-gpui\Cargo.toml --locked
cargo build --manifest-path crates\amazing-shell-gpui\Cargo.toml -p amazing-image --example render_reftest --locked

powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath tests\wpt-replaced-content\image-manifest.json `
  -WptRoot target\wpt -OutputDirectory target\rc1-images-final-1 `
  -ExecutablePath C:\Users\server\target\replaced-content-image\debug\examples\render_reftest.exe `
  -FontSet 'Fallback font; static image adapter artifact' -SkipBuild -SkipFetch
```

Run again into `rc1-images-final-2`. For the scope diagnostic and focused gate,
use the production `amazing.exe`, `manifest.json` and
`tests/wpt-render/paint-compositing-manifest.json` respectively. Snapshot the
executable before rendering so other builds cannot replace it mid-measurement.
`collect-results.ps1` normalizes report locations; it preserves classifications,
pixel differences, hashes and error details. `validation.json` records artifact
hashes, Windows test counts and the successful locked shell check.

## Interpretation

- Scope diagnostic: **9 pass / 21 mismatch / 0 timeout / 1 unsupported /
  17 crash**, unchanged before/after. The runner calls any renderer nonzero
  exit a crash, including script parse/runtime rejection; this is not a claim
  that all 17 are native crashes. The XHTML reference is unsupported.
- Image diagnostic: **2 pass / 10 mismatch** before, **0 pass / 12 mismatch**
  after; timeout/unsupported/crash are all zero. The two former object-position
  matches were inert equal-pixel results while neither image content nor the
  reference background image painted. Their reference hashes stay unchanged
  while decoded actual images now differ. **No new WPT conformance is claimed.**
- Focused paint gate: **24 pass / 0 mismatch / 0 timeout / 0 unsupported /
  0 crash** before and after.

Pixel behavior is separately covered by real HTML→cascade→layout→paint tests
for block/inline fitting and opacity/overflow, plus direct geometry/raster
checks for crop, positioning, alpha and hostile buffers. Natural dimensions,
definite overrides and the one-auto-axis aspect ratio are covered in media.
These are regression tests, not upstream-conformance credits.

Remaining WPT dependencies include inline CSS used-size/border/padding layout,
floats and background images. The natural sizing helper does not claim to solve
CSS containing-block percentages, min/max constraints or general inline sizing.
Decoder output is one frame, straight RGBA8; animation, ICC management and EXIF
orientation are deferred. Codec-internal allocation limits remain best-effort;
input, dimensions and returned RGBA bytes have independent explicit caps.
