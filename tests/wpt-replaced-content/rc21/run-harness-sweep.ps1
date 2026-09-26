#!/usr/bin/env pwsh
# RC-21 pinned WPT testharness sweep. Runs each frozen 117-file img/object-fit
# selection through `amazing wpt-harness` and aggregates the per-test status.
#
# Usage (from the Windows lane tree root, with CARGO_TARGET_DIR set):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\wpt-replaced-content\rc21\run-harness-sweep.ps1 `
#     -WptRoot C:\Users\server\src\wpt `
#     -ExecutablePath C:\Users\server\target\replaced-content\debug\amazing.exe
param(
    [string]$WptRoot = "C:\Users\server\src\wpt",
    [string]$ExecutablePath = "C:\Users\server\target\replaced-content\debug\amazing.exe",
    [string]$ManifestPath = "",
    [string]$OutputDirectory = "",
    [int]$Width = 800,
    [int]$Height = 600,
    [int]$TimeoutMs = 20000
)

$ErrorActionPreference = "Continue"
Set-StrictMode -Version Latest

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\..\.."))
if (-not $ManifestPath) {
    $ManifestPath = Join-Path $repositoryRoot "crates\amazing-paint\tests\fixtures\rc21-wpt-harness.json"
}
$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
$revision = [string]$manifest.revision
if ($revision -notmatch '^[0-9a-f]{40}$') {
    throw "Manifest revision must be a full 40-character Git object id."
}
if (-not $OutputDirectory) {
    $runName = "run-{0}" -f [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
    $OutputDirectory = Join-Path $PSScriptRoot $runName
}
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null

$counts = [ordered]@{ pass = 0; fail = 0; timeout = 0; error = 0; unsupported = 0 }
$records = New-Object Collections.Generic.List[object]

foreach ($test in @($manifest.tests)) {
    $argv = @("wpt-harness", $WptRoot, [string]$test, [string]$Width, [string]$Height, [string]$TimeoutMs)
    $raw = & $ExecutablePath @argv 2>$null
    $line = ($raw | Where-Object { $_ -match '^\{"schema"' } | Select-Object -Last 1)
    if (-not $line) {
        $counts["error"]++
        $records.Add([ordered]@{ test = [string]$test; status = "error"; detail = "no report line" })
        Write-Host ("{0,-12} {1}" -f "error", $test)
        continue
    }
    $report = $line | ConvertFrom-Json
    $status = [string]$report.status
    if (-not $counts.Contains($status)) { $status = "error" }
    $counts[$status]++
    $records.Add([ordered]@{
        test            = [string]$test
        status          = $status
        harness_status  = $report.harness_status
        pass            = $report.counts.pass
        fail            = $report.counts.fail
        timeout         = $report.counts.timeout
        error           = $report.counts.error
        detail          = $report.detail
    })
    Write-Host ("{0,-12} {1}" -f $status, $test)
}

$reportPath = Join-Path $OutputDirectory "report.json"
$summary = [ordered]@{
    schema       = 1
    wpt_revision = $revision
    manifest     = $ManifestPath
    viewport     = [ordered]@{ width = $Width; height = $Height }
    timeout_ms   = $TimeoutMs
    counts       = $counts
    tests        = $records
}
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8
Write-Host "Report: $reportPath"
Write-Host ("Counts: pass={0} fail={1} timeout={2} error={3} unsupported={4}" -f
    $counts.pass, $counts.fail, $counts.timeout, $counts.error, $counts.unsupported)
