# RC-2 evidence

WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
The original RC-1 manifests and result files remain unchanged. RC-2 uses the
same 12 PNG cases, 48-entry six-cluster scope, and 24-case paint gate, plus
`../svg-manifest.json`: the 12 paired SVG counterparts. All upstream test and
reference files are unmodified; background references use explicit longhands.

The before binaries are built from clean dev 80c6636 after the integrator's
server refresh. The after binaries include this lane's changes, re-synced from
the Mac source of truth. All builds and renders run on win-001. Earlier runs
at 26c64de were superseded by the refresh and are excluded here.

The PNG/SVG diagnostics use the static `render_reftest` example at 800×600,
scale 1, fallback fonts, local resources and no script execution. Scope and the
focused paint gate use the production root `amazing.exe` renderer. These paths
have different resource loaders: the root driver still supplies only BMP/PPM
images. The GPUI shell uses amazing-image for img and background resources,
checked separately with its lockfile. No interactive-shell behavior or visual
UI proof is inferred from these runs.

## Reproduction

From the Windows lane tree, build and copy binaries before changing sources:

```powershell
$env:CARGO_TARGET_DIR='C:\Users\server\target\replaced-content'
cargo build -p amazing --locked
Copy-Item "$env:CARGO_TARGET_DIR\debug\amazing.exe" target/rc2h-driver-before.exe
$env:CARGO_TARGET_DIR='C:\Users\server\target\replaced-content-image'
cargo build --manifest-path crates/amazing-image/Cargo.toml --example render_reftest --locked
Copy-Item "$env:CARGO_TARGET_DIR\debug\examples\render_reftest.exe" target/rc2h-images-before.exe
```

Re-sync the lane patch and repeat with `after` binary names. Run each manifest
twice for each phase using `scripts/run-render-wpt.ps1 -SkipFetch -SkipBuild`,
`-WptRoot target/wpt`, the appropriate `-ExecutablePath`, and
`-OutputDirectory target/rc2h-<images|svg|scope|focused>-<before|after>-<1|2>`.
Use `../collect-rc2.ps1` to preserve reports with normalized paths.
`analyze-rc2.py` summarizes the saved reports into `repeatability.json`, which
compares ordered classifications, differing pixel counts,
and actual/reference/diff hashes, excluding run timestamps and paths.

## Credit boundaries

A same-pixel result is not sufficient for conformance credit. In particular,
the paired SVG resources use inline CSS fill declarations, which this bounded
SVG slice explicitly rejects. Both sides omit those images. Its raw equal
results must receive **zero SVG conformance credit**. Focused adapter tests
separately cover presentation attributes, rectangles, circles, ellipses,
inherited fill/currentColor, alpha, viewBox mapping and input/work limits.
SVG CSS, paths, text, transforms, strokes, filters, external references,
percent/fractional intrinsic viewport sizing, animation and antialiasing remain
outside this slice. Unsupported features reject the whole image.

The scope diagnostic also contains inert matches and script-driven cases.
Its `crash` classification includes renderer nonzero exits such as script
rejection; it is not a native-crash count. CSS background shorthand expansion
and stylesheet-relative URL provenance are accepted CSS-3 work. This ticket
does not rewrite upstream shorthand references or claim complete CSS
Backgrounds, CSS Images, or SVG conformance.

## Measured result

| Selection | Before | After | Interpretation |
| --- | ---: | ---: | --- |
| PNG fitting/position | 0/12 | 12/12 | Selected static image cases now match |
| SVG counterparts | 12/12 raw | 12/12 raw | Zero SVG credit; unsupported images omitted |
| Six-cluster scope | 25/48 raw | 25/48 raw | 5 mismatches, 1 unsupported, 17 renderer errors |
| Focused paint | 24/24 | 24/24 | No regression |

All four selections ran twice before and twice after. All eight paired
classification/pixel-hash projections are stable. Every PNG reference hash
changed; only the two half-pixel object-position actual hashes changed. SVG
actual and reference hashes are unchanged across phases, confirming no gain.
Workspace: 927 passed, 0 failed, 3 ignored. Paint: 74 passed, including 12 new
checks. Adapter: 12 passed. Standalone shell: locked check passed.
