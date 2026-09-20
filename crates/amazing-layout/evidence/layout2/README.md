# LAYOUT-2 evidence

Only runs made after the integrator refreshed the Windows tree to `133292f`
are credited. Earlier `26c64de` runs are excluded. The final implementation was
re-synced, and all 142 tracked Rust/build files were compared with the Mac;
`build-source-sha256.json` records that verification input.

The measurement artifact is the root `amazing.exe` renderer, with its existing
BMP/PPM resource loader and the font configuration recorded in each result.
It is not the separate PNG image adapter or the interactive GPUI shell.
WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Viewport: 800 × 600, device scale 1, runner default timeout 20 seconds.

On **win-001**, from `C:\Users\server\src\wt\layout`, reproduce the seven
unchanged selections twice with separately preserved baseline/final executables:

```powershell
& crates\amazing-layout\evidence\run-layout2.ps1 `
  -ExecutablePath C:\Users\server\target\layout\layout2-baseline-133.exe `
  -WptRoot C:\Users\server\target\layout\wpt `
  -OutputDirectory C:\Users\server\target\layout\layout2-evidence\baseline-verified
& crates\amazing-layout\evidence\run-layout2.ps1 `
  -ExecutablePath C:\Users\server\target\layout\layout2-final-verified.exe `
  -WptRoot C:\Users\server\target\layout\wpt `
  -OutputDirectory C:\Users\server\target\layout\layout2-evidence\final-verified
```

Use fresh output directories for a new measurement. `collect.ps1` rejects
ambiguous selections with multiple run directories and source drift against
`-SourceManifestPath` (the manifest verified before the build). Pass those directory names
via `-BaselinePhase` and `-FinalPhase`, and the corresponding executable paths
via `-BaselineExecutable` and `-FinalExecutable`. The required `-OutputPath`
is a scratch JSON file; full raw reports and PPMs stay on the server.

`normalize.py` only analyzes retained evidence; it does not execute tests. It
accepts the collected JSON, workspace log, workspace exit-code file, source-base
commit, and output directory. Beside the workspace log it expects unit logs
named `layout2-<phase>-tests.log`. It validates identical test selections and
two stable result/pixel-hash projections for both executables, then writes one
result file per suite and `results.json`. All source and executable hashes,
run directories, fonts, measured counts and classification changes are retained.

The final workspace command was `cargo test --workspace --locked`, run with `CARGO_TARGET_DIR=C:\Users\server\target\layout`. Its three ignored tests
require external HTTPS or a real interactive D3D11 device. An earlier passing workspace run was invalidated after a further server
overwrite was detected; the credited rerun checks all 142 source/build hashes
both before and after the command. No local engine builds or tests were run. Unit checks use the public layout entry points,
including both legacy and provider-shaped text paths.
