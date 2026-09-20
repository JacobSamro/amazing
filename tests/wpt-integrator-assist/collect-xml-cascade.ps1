param([string]$Stage, [string]$DriverRevision, [string]$Executable)
$ErrorActionPreference = 'Stop'
$root = Join-Path (Get-Location) 'target/diagnostics/ia-xml'
$out = Join-Path $root 'evidence'
New-Item -ItemType Directory -Force $out | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
function Write-Json($path, $value) {
    $json = ConvertTo-Json -InputObject $value -Depth 12
    $json = $json.Replace(([string][char]13 + [char]10), [string][char]10)
    [IO.File]::WriteAllText($path, $json + [char]10, $utf8)
}
$summary = @()
foreach ($round in 1..2) {
    $file = Get-ChildItem (Join-Path $root "$Stage-$round") -Filter report.json -Recurse |
        Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    if (-not $file) { throw "Missing $Stage-$round report" }
    $report = Get-Content $file.FullName -Raw | ConvertFrom-Json
    $records = @($report.tests | ForEach-Object {
        [ordered]@{ id=$_.id; classification=$_.classification; actual_sha256=$_.actual_sha256;
            reference_sha256=$_.reference_sha256; diff_sha256=$_.diff_sha256;
            differing_pixels=$_.differing_pixels; max_difference=$_.max_difference; detail=$_.detail }
    })
    $projection = Join-Path $out "$Stage-$round.json"
    Write-Json $projection $records
    Write-Json (Join-Path $out "$Stage-$round-report.json") $report
    $summary += [ordered]@{
        stage=$Stage; round=$round; driver_revision=$DriverRevision; wpt_revision=$report.wpt_revision;
        executable_sha256=(Get-FileHash $Executable -Algorithm SHA256).Hash.ToLowerInvariant();
        manifest_sha256=(Get-FileHash tests/wpt-render/cascade-manifest.json -Algorithm SHA256).Hash.ToLowerInvariant();
        cases=$records.Count; counts=[ordered]@{ pass=$report.counts.pass; mismatch=$report.counts.mismatch;
            error=[int]$report.counts.error; timeout=$report.counts.timeout;
            unsupported=$report.counts.unsupported; crash=$report.counts.crash };
        projection_sha256=(Get-FileHash $projection -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
Write-Json (Join-Path $out "$Stage-summary.json") $summary
if ($summary[0].projection_sha256 -ne $summary[1].projection_sha256) { throw 'Cascade repeats differ' }
$summary | ConvertTo-Json -Depth 6
