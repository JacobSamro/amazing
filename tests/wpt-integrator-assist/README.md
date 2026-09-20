# IA-1 driver evidence

Run these scripts **on win-001**, from the lane's repository root. The custom
fixtures are diagnostics, not upstream WPT and not conformance credit.

- `cli-parity.ps1`: capture stdout, stderr, exit status, timeout, and render
  artifact SHA-256 for 36 cases covering every named command in the dispatch.
  The native GUI is bounded at 1.5 seconds and is not a screenshot/visual proof.
  Audio playback, browser-peer, SDP answer, and internal peer commands exercise
  error/usage paths; the existing driver tests additionally exercise secure
  streaming success. Default no-argument GPUI launching is not invoked.
- `compare-cli.ps1`: compare captures exactly except OS-assigned localhost
  ports in three transport probes. No other output fields are normalized.
- `natural-size-proof.ps1`: baseline-versus-fixed render of an attributed image
  with CSS auto height, compared against explicit natural-aspect-ratio sizing.
- `attribute-size.html`: diagnostic for the separately routed absence of HTML
  width/height presentational hints. Render at 12×8; evidence records the
  baseline 8×4 versus final 2×1 colored region. It is not a passing WPT claim.
- `render-gates.ps1`: use the existing render runner for every committed render
  and replaced-content manifest. Verify detached WPT HEAD directly, without
  executing Git on the build server; require all selected input files to exist.
  Existing checkout selection is read-only. Reports preserve the upstream
  runner's classifications, pixel hashes, viewport, and revision. Temporary
  case-directory names are hashed to avoid Windows MAX_PATH; report IDs are
  unchanged. `-ExcludeSelections` can resume unfinished selections with the
  same immutable binary without rerunning completed reports.

Build original and candidate binaries separately and retain immutable copies
below `target/diagnostics/`. Example invocation:

```powershell
$base = 'C:\Users\server\src\wt\integrator-assist\target\diagnostics'
& tests/wpt-integrator-assist/cli-parity.ps1 `
  -Executable "$base/final.exe" -OutputDirectory "$base/final-1"
& tests/wpt-integrator-assist/compare-cli.ps1 `
  -Baseline "$base/before/cli.json" `
  -Candidates @("$base/final-1/cli.json", "$base/final-2/cli.json") `
  -Output "$base/cli-comparison.json"
& tests/wpt-integrator-assist/natural-size-proof.ps1 `
  -Before "$base/before.exe" -After "$base/final.exe" `
  -OutputDirectory "$base/natural-size"
& tests/wpt-integrator-assist/render-gates.ps1 `
  -Executable "$base/final.exe" -OutputDirectory "$base/gates-final-1"
```

Run a second capture/gate with a separate output directory. The CLI renderer
uses fixed `target/diagnostics/ia-parity.*` paths, so CLI captures run sequentially.
Pinned render gates have separate run directories and may run concurrently.

Expanded routing adds `verify-root-mapping.ps1`: run it with `-Executable` and
`-OutputDirectory` on Windows. Three rendering diagnostics must pass and two
reference diagnostics must remain unsupported on both runs. Missing/wrong
revision markers must be rejected, and the Git sentinel must record zero calls.
`root-fixtures/revision.txt` is a synthetic marker, not an upstream revision.
The production runner accepts `-RevisionMarker` for Git-free transferred trees;
otherwise it reads a detached `.git/HEAD` and checks it against the manifest.
The runner no longer provisions or fetches WPT, even if `-SkipFetch` is omitted.
