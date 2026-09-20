# Windows only; named immutable artifacts, pinned WPT source, two repeats.
param(
    [Parameter(Mandatory=$true)][string]$ExecutablePath,
    [Parameter(Mandatory=$true)][string]$ImageExecutablePath,
    [Parameter(Mandatory=$true)][string]$WptRoot,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [string]$Phase='final'
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$repositoryRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$runner=Join-Path $repositoryRoot 'scripts\run-render-wpt.ps1'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$before=(Get-FileHash $ExecutablePath).Hash
$imageBefore=(Get-FileHash $ImageExecutablePath).Hash
foreach($repeat in 1,2) {
    foreach($suite in @('tables','geometry','floats','block-collapse','position','text-flow','flex-grid','paint-compositing','images')) {
        $manifest=if($suite -in @('tables','floats')) {
            Join-Path $PSScriptRoot "$suite-manifest.json"
        } elseif($suite -eq 'images') {
            Join-Path $repositoryRoot 'tests\wpt-replaced-content\image-manifest.json'
        } else {Join-Path $repositoryRoot "tests\wpt-render\$suite-manifest.json"}
        $exe=if($suite -eq 'images'){$ImageExecutablePath}else{$ExecutablePath}
        $font=if($suite -eq 'images'){'Fallback font; static image adapter artifact'}else{'Amazing text provider: WPT @font-face via HarfRust 0.13.2 + Swash 0.2.10; deterministic fallback otherwise'}
        $output=Join-Path $OutputDirectory "$Phase-$suite-$repeat"
        & $runner -ManifestPath $manifest -WptRoot $WptRoot -ExecutablePath $exe `
            -OutputDirectory $output -FontSet $font -ViewportWidth 800 -ViewportHeight 600 -TimeoutSeconds 20 `
            -SkipBuild -SkipFetch *> (Join-Path $OutputDirectory "$Phase-$suite-$repeat.log")
        Get-Content (Join-Path $OutputDirectory "$Phase-$suite-$repeat.log") -Tail 2
    }
}
if((Get-FileHash $ExecutablePath).Hash -ne $before -or (Get-FileHash $ImageExecutablePath).Hash -ne $imageBefore) {
    throw 'An artifact changed during measurement'
}
