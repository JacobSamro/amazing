# win-001 only: fixed-base LAYOUT-8 WPT evidence from immutable artifacts.
param(
    [Parameter(Mandatory=$true)][string]$ExecutablePath,
    [Parameter(Mandatory=$true)][string]$ImageExecutablePath,
    [Parameter(Mandatory=$true)][string]$WptRoot,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..\..'))
$runner=Join-Path $root 'scripts\run-render-wpt.ps1'
$sourceManifest=Get-Content (Join-Path $PSScriptRoot 'final-sources.json') -Raw|ConvertFrom-Json
function Verify-Source {
    foreach($entry in $sourceManifest.PSObject.Properties){
        if((Get-FileHash (Join-Path $root $entry.Name)).Hash.ToLowerInvariant() -ne $entry.Value){
            throw "Source drift: $($entry.Name)"
        }
    }
}
Verify-Source
$exeHash=(Get-FileHash $ExecutablePath).Hash
$imageHash=(Get-FileHash $ImageExecutablePath).Hash
New-Item -ItemType Directory -Force $OutputDirectory|Out-Null
foreach($suite in @('calc','broad')){
    $manifest=if($suite -eq 'calc'){
        Join-Path $PSScriptRoot 'calc-used-values-manifest.json'
    }else{
        Join-Path $root 'tests\wpt-render\flex-grid-broad-manifest.json'
    }
    foreach($repeat in 1..2){
        $dest=Join-Path $OutputDirectory ($suite+'-'+$repeat)
        & $runner -ManifestPath $manifest -WptRoot $WptRoot -OutputDirectory $dest `
            -ExecutablePath $ExecutablePath -SkipBuild -SkipFetch *> (Join-Path $OutputDirectory ($suite+'-'+$repeat+'.log'))
        $report=Get-ChildItem $dest -Filter report.json -Recurse|Select-Object -First 1
        if(-not $report){throw "Missing $suite $repeat report"}
        Copy-Item $report.FullName (Join-Path $OutputDirectory ($suite+'-'+$repeat+'.json')) -Force
        $data=Get-Content $report.FullName -Raw|ConvertFrom-Json
        Write-Output ($suite+'-'+$repeat+' '+($data.counts|ConvertTo-Json -Compress))
    }
}
$suites=@('flex-grid','geometry','tables','floats','block-collapse',
    'position','text-flow','contents','paint-compositing','images')
$retained=Join-Path $OutputDirectory 'retained'
& (Join-Path $PSScriptRoot '..\layout5\run-final.ps1') `
    -ExecutablePath $ExecutablePath -ImageExecutablePath $ImageExecutablePath `
    -WptRoot $WptRoot -OutputDirectory $retained -Workers 2 -Suites $suites
foreach($suite in $suites){foreach($repeat in 1..2){
    $dest=Join-Path $retained ('final-'+$suite+'-'+$repeat)
    $report=Get-ChildItem $dest -Filter report.json -Recurse|Select-Object -First 1
    if(-not $report){throw "Missing $suite $repeat report"}
    Copy-Item $report.FullName (Join-Path $OutputDirectory ($suite+'-'+$repeat+'.json')) -Force
    $data=Get-Content $report.FullName -Raw|ConvertFrom-Json
    Write-Output ($suite+'-'+$repeat+' '+($data.counts|ConvertTo-Json -Compress))
}}
Verify-Source
if((Get-FileHash $ExecutablePath).Hash -ne $exeHash -or
   (Get-FileHash $ImageExecutablePath).Hash -ne $imageHash){throw 'Artifact changed'}
