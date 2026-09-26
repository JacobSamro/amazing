#!/usr/bin/env pwsh
# RC-21 pinned render sweep. Wraps the repository render harness with the
# frozen 226-case manifest committed beside the paint fixtures.
#
# Usage (from the Windows lane tree root, with CARGO_TARGET_DIR set):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\wpt-replaced-content\rc21\run-render-sweep.ps1 `
#     -WptRoot C:\Users\server\target\layout\wpt `
#     -ExecutablePath C:\Users\server\target\replaced-content\debug\amazing.exe
param(
    [string]$WptRoot = "C:\Users\server\target\layout\wpt",
    [string]$ExecutablePath = "C:\Users\server\target\replaced-content\debug\amazing.exe",
    [string]$ManifestPath = "",
    [string]$OutputDirectory = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\..\.."))
if (-not $ManifestPath) {
    $ManifestPath = Join-Path $repositoryRoot "crates\amazing-paint\tests\fixtures\rc21-wpt-render.json"
}
if (-not $OutputDirectory) {
    $runName = "run-{0}" -f [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
    $OutputDirectory = Join-Path $PSScriptRoot $runName
}

& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $repositoryRoot "scripts\run-render-wpt.ps1") `
    -ManifestPath $ManifestPath `
    -WptRoot $WptRoot `
    -OutputDirectory $OutputDirectory `
    -ExecutablePath $ExecutablePath `
    -SkipFetch
exit $LASTEXITCODE
