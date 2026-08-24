# Upstream WPT render gate

`smoke.json` pins an exact commit from
`https://github.com/web-platform-tests/wpt.git`. The source is sparse-checked
out under ignored `target/wpt`; upstream test and reference files are never
copied into or modified by this repository.

Run on `win-001` from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1
```

The runner accepts alternate manifest, source, output, executable, viewport,
and timeout parameters. It reads each test's upstream `rel=match` or
`rel=mismatch` link, renders both sides through `amazing render-wpt`, applies
exact or upstream `meta name=fuzzy` pixel limits, and writes PPM
actual/reference/diff images plus a JSON report. A manifest's `expected` field
validates the harness smoke outcome; it cannot change the upstream assertion.

`-SkipFetch` requires `WptRoot` already to be at the manifest revision.
`-SkipBuild` uses the selected existing executable.

`baseline.json` is the normalized, committed evidence from the ticket #1
repeatability gate. It records results and hashes only; `smoke.json` remains
the executable selection and expected harness outcome.

Ticket #2 adds a deterministic cascade/value inventory:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\inventory-render-wpt.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath target\wpt-cascade-baseline.json -SkipFetch
```

The committed `cascade-inventory.json`, `cascade-manifest.json`,
`cascade-results.json`, and `cascade-oracles.json` preserve the exact selection,
measured baseline, and exact-revision Blink/WebKit comparison.
Inventory provisions the pinned sparse checkout unless `-SkipFetch` is passed.

Ticket #2a uses `computed-values-manifest.json`, the fixed 24-case
Cascade+Variables projection of the baseline. `computed-values-results.json`
records its repeat-stable 4/24 → 16/24 result; the Variables half moved from
0/12 to 12/12. `cascade-results-ticket2a.json` records the full-corpus
30/72 → 43/72 measurement without overwriting the original baseline.
