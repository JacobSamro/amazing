param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$OutputDirectory = [IO.Path]::GetFullPath((Join-Path (Get-Location) $OutputDirectory))
$Executable = [IO.Path]::GetFullPath((Join-Path (Get-Location) $Executable))
$root = Join-Path $OutputDirectory 'fixture'
New-Item -ItemType Directory -Force $root | Out-Null
$revision = 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293'
[IO.File]::WriteAllText((Join-Path $root 'revision.txt'), $revision)
$files = @{
    'reference.html' = '<html><body><div style="width:16px;height:16px;background:lime"></div></body></html>'
    'malformed.xhtml' = '<html xmlns="http://www.w3.org/1999/xhtml"><link rel="match" href="reference.html"/><broken></html>'
    'unsupported.xhtml' = '<!DOCTYPE html SYSTEM "missing.dtd"><html xmlns="http://www.w3.org/1999/xhtml"><link rel="match" href="reference.html"/><body>&external;</body></html>'
    'reference-error.xhtml' = '<html xmlns="http://www.w3.org/1999/xhtml"><link rel="match" href="malformed.xhtml"/><body><div style="width:16px;height:16px;background:lime"/></body></html>'
    'throw.html' = '<html><link rel="match" href="reference.html"><script>throw 7;</script></html>'
}
foreach ($name in $files.Keys) { [IO.File]::WriteAllText((Join-Path $root $name), $files[$name]) }
$manifest = [ordered]@{
    schema = 1
    revision = $revision
    tests = @(
        [ordered]@{ id='xml-error'; test='malformed.xhtml' },
        [ordered]@{ id='xml-unsupported'; test='unsupported.xhtml' },
        [ordered]@{ id='reference-error'; test='reference-error.xhtml' },
        [ordered]@{ id='html-crash'; test='throw.html' }
    )
}
$manifestPath = Join-Path $OutputDirectory 'manifest.json'
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
& scripts/run-render-wpt.ps1 -ManifestPath $manifestPath -WptRoot $root -RevisionMarker (Join-Path $root 'revision.txt') -ExecutablePath $Executable -OutputDirectory (Join-Path $OutputDirectory 'results') -ViewportWidth 16 -ViewportHeight 16 -SkipBuild -SkipFetch | Out-Null
$reportPath = Get-ChildItem (Join-Path $OutputDirectory 'results') -Filter report.json -Recurse |
    Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1 -ExpandProperty FullName
$report = Get-Content -LiteralPath $reportPath -Raw | ConvertFrom-Json
$byId = @{}
foreach ($test in $report.tests) { $byId[[string]$test.id] = $test }
foreach ($item in @(@('xml-error','error'), @('xml-unsupported','unsupported'), @('reference-error','error'), @('html-crash','crash'))) {
    if ($byId[$item[0]].classification -ne $item[1]) { throw "$($item[0]) classified $($byId[$item[0]].classification), expected $($item[1])" }
}
if (-not $byId['reference-error'].actual_sha256) { throw 'Actual pixel hash lost when XML reference failed' }
if ($report.counts.pass -ne 0 -or $report.counts.error -ne 2 -or $report.counts.unsupported -ne 1 -or $report.counts.crash -ne 1) { throw 'Unexpected protocol counts' }
[ordered]@{
    cases = 4; pass = 0; error = 2; unsupported = 1; crash = 1
    reference_error_actual_sha256 = $byId['reference-error'].actual_sha256
    executable_sha256 = (Get-FileHash $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
    report = $reportPath
    wpt_credit = 0
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'summary.json') -Encoding UTF8
Get-Content -LiteralPath (Join-Path $OutputDirectory 'summary.json') -Raw
