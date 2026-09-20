# Windows only; named immutable artifacts, pinned WPT source, two repeats.
param(
    [Parameter(Mandatory=$true)][string]$ExecutablePath,
    [string]$ImageExecutablePath,
    [Parameter(Mandatory=$true)][string]$WptRoot,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [string]$Phase='final',
    [string[]]$Suites=@('tables','geometry','floats','block-collapse','position','text-flow','flex-grid','paint-compositing','images'),
    [ValidateSet(1,2)][int[]]$Repeat=@(1,2)
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$repositoryRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$runner=Join-Path $repositoryRoot 'scripts\run-render-wpt.ps1'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$before=(Get-FileHash $ExecutablePath).Hash
$imageBefore=if('images' -in $Suites){(Get-FileHash $ImageExecutablePath).Hash}else{$null}
foreach($repeatNumber in $Repeat) {
    foreach($suite in $Suites) {
        $manifest=if($suite -in @('tables','floats','contents')) {
            Join-Path $PSScriptRoot "$suite-manifest.json"
        } elseif($suite -eq 'images') {
            Join-Path $repositoryRoot 'tests\wpt-replaced-content\image-manifest.json'
        } else {Join-Path $repositoryRoot "tests\wpt-render\$suite-manifest.json"}
        $exe=if($suite -eq 'images'){$ImageExecutablePath}else{$ExecutablePath}
        $font=if($suite -eq 'images'){'Fallback font; static image adapter artifact'}else{'Amazing text provider: WPT @font-face via HarfRust 0.13.2 + Swash 0.2.10; deterministic fallback otherwise'}
        $output=Join-Path $OutputDirectory "$Phase-$suite-$repeatNumber"
        & $runner -ManifestPath $manifest -WptRoot $WptRoot -ExecutablePath $exe `
            -OutputDirectory $output -FontSet $font -ViewportWidth 800 -ViewportHeight 600 -TimeoutSeconds 20 `
            -SkipBuild -SkipFetch *> (Join-Path $OutputDirectory "$Phase-$suite-$repeatNumber.log")
        Get-Content (Join-Path $OutputDirectory "$Phase-$suite-$repeatNumber.log") -Tail 2
    }
}
if((Get-FileHash $ExecutablePath).Hash -ne $before -or ($imageBefore -and (Get-FileHash $ImageExecutablePath).Hash -ne $imageBefore)) {
    throw 'An artifact changed during measurement'
}
