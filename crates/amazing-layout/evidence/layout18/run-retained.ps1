param(
    [string]$Repository = (Join-Path $PSScriptRoot '..\..\..\..'),
    [string]$Executable = 'C:\Users\server\target\layout\layout18-candidate.exe',
    [string]$WptRoot = 'C:\Users\server\target\layout\wpt',
    [string]$Output = 'C:\Users\server\target\layout\layout18-retained'
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-Location (Resolve-Path $Repository)
Get-FileHash -Algorithm SHA256 $Executable
foreach ($selection in @(
    @{name='values'; manifest='crates\amazing-layout\evidence\layout18\css-values-manifest.json'},
    @{name='retained'; manifest='crates\amazing-layout\evidence\layout16\layout16-full-manifest.json'}
)) {
    & scripts\run-render-wpt.ps1 -ManifestPath $selection.manifest -WptRoot $WptRoot `
        -OutputDirectory (Join-Path $Output $selection.name) -ExecutablePath $Executable `
        -ViewportWidth 800 -ViewportHeight 600 -TimeoutSeconds 20 -SkipBuild -SkipFetch
    if ($LASTEXITCODE -ne 0) { throw "Render runner failed: $($selection.name), $LASTEXITCODE" }
}
