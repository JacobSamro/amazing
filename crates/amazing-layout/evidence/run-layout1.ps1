# Run on win-001 only. The executable must be built from the reviewed source.
param(
    [Parameter(Mandatory=$true)][string]$ExecutablePath,
    [Parameter(Mandatory=$true)][string]$WptRoot,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$runner = Join-Path $repositoryRoot 'scripts\run-render-wpt.ps1'
foreach ($repeat in @(1, 2)) {
    foreach ($suite in @('geometry', 'floats')) {
        $manifest = if ($suite -eq 'floats') {
            Join-Path $PSScriptRoot 'floats-manifest.json'
        } else {
            Join-Path $repositoryRoot 'tests\wpt-render\geometry-manifest.json'
        }
        & $runner -ManifestPath $manifest -WptRoot $WptRoot `
            -ExecutablePath $ExecutablePath -OutputDirectory (Join-Path $OutputDirectory "$suite-$repeat") `
            -SkipFetch -SkipBuild
    }
}
foreach ($suite in @('block-collapse', 'position', 'text-flow', 'flex-grid', 'paint-compositing')) {
    & $runner -ManifestPath (Join-Path $repositoryRoot "tests\wpt-render\$suite-manifest.json") `
        -WptRoot $WptRoot -ExecutablePath $ExecutablePath `
        -OutputDirectory (Join-Path $OutputDirectory $suite) -SkipFetch -SkipBuild
}
