$ErrorActionPreference = 'Stop'
$evidence = 'crates/amazing-html/evidence/html23'
$audit = Get-Content "$evidence/source-hashes.json" -Raw | ConvertFrom-Json
function Assert-Hashes($phase) {
    $count = 0
    foreach ($item in $audit.PSObject.Properties) {
        $actual = (Get-FileHash -Algorithm SHA256 $item.Name).Hash.ToLowerInvariant()
        if ($actual -ne $item.Value) { throw "$phase hash mismatch: $($item.Name): $actual != $($item.Value)" }
        $count++
    }
    Write-Output "$phase source/fixture/lock audit: $count matching files"
}
Assert-Hashes 'before'
& cargo nextest run -p amazing-html --locked
if ($LASTEXITCODE -ne 0) { throw 'package nextest failed' }
& cargo test -p amazing-html --locked --doc
if ($LASTEXITCODE -ne 0) { throw 'package doctest failed' }
& cargo nextest run --workspace --locked
if ($LASTEXITCODE -ne 0) { throw 'workspace nextest failed' }
& cargo test --workspace --locked --doc
if ($LASTEXITCODE -ne 0) { throw 'workspace doctest failed' }
$out = Join-Path $env:CARGO_TARGET_DIR 'diagnostics/html23'
New-Item -ItemType Directory -Force $out | Out-Null
for ($n = 1; $n -le 2; $n++) {
    $cases = & cargo run -q -p amazing-html --example html23_probe --locked -- $evidence
    if ($LASTEXITCODE -ne 0) { throw 'new corpus probe failed' }
    $cases | Set-Content -Encoding UTF8 (Join-Path $out "after-$n.tsv")
    $ret22 = & cargo run -q -p amazing-html --example html22_probe --locked -- 'crates/amazing-html/evidence/html22'
    if ($LASTEXITCODE -ne 0) { throw 'HTML-22 retention probe failed' }
    $ret22 | Set-Content -Encoding UTF8 (Join-Path $out "ret22-after-$n.tsv")
    $ret21 = & cargo run -q -p amazing-html --example html22_probe --locked -- 'crates/amazing-html/evidence/html21' '--retained'
    if ($LASTEXITCODE -ne 0) { throw 'HTML-21 retention probe failed' }
    $ret21 | Set-Content -Encoding UTF8 (Join-Path $out "ret21-after-$n.tsv")
}
Get-ChildItem $out -Filter '*.tsv' | Sort-Object Name | ForEach-Object { Write-Output "$($_.Name) $((Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLowerInvariant())" }
Assert-Hashes 'after'
