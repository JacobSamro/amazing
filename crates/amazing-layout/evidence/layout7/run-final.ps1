# Windows only: pinned, immutable final LAYOUT-7 render gates.
param(
    [Parameter(Mandatory=$true)][string]$ExecutablePath,
    [Parameter(Mandatory=$true)][string]$ImageExecutablePath,
    [Parameter(Mandatory=$true)][string]$WptRoot,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [switch]$PrimaryOnly,
    [switch]$RetainedOnly
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..\..'))
$target='C:\Users\server\target\layout'
$runner=Join-Path $root 'scripts\run-render-wpt.ps1'
$hash=(Get-FileHash $ExecutablePath).Hash
$imageHash=(Get-FileHash $ImageExecutablePath).Hash
New-Item -ItemType Directory -Force $OutputDirectory|Out-Null

if(-not $RetainedOnly){
    foreach($name in @('broad','focused')){
        $manifest=if($name -eq 'broad'){
            Join-Path $root 'tests\wpt-render\flex-grid-broad-manifest.json'
        }else{
            Join-Path $PSScriptRoot 'subgrid-grid-lanes-manifest.json'
        }
        foreach($n in 1..2){
            $dest=Join-Path $OutputDirectory ($name+'-'+$n)
            & $runner -ManifestPath $manifest -WptRoot $WptRoot `
                -OutputDirectory $dest -ExecutablePath $ExecutablePath `
                -SkipBuild -SkipFetch *> (Join-Path $OutputDirectory ($name+'-'+$n+'.log'))
            $report=Get-ChildItem $dest -Filter report.json -Recurse|Select-Object -First 1
            if(-not $report){throw "Missing $name $n report"}
            Copy-Item $report.FullName (Join-Path $target ('layout7-candidate-'+$name+'-'+$n+'.json')) -Force
            $data=Get-Content $report.FullName -Raw|ConvertFrom-Json
            Write-Output ($name+'-'+$n+' '+($data.counts|ConvertTo-Json -Compress))
        }
    }
}

if(-not $PrimaryOnly){
    $suites=@('flex-grid','geometry','tables','floats','block-collapse',
        'position','text-flow','contents','paint-compositing','images')
    $retained=Join-Path $OutputDirectory 'retained'
    & (Join-Path $PSScriptRoot '..\layout5\run-final.ps1') `
        -ExecutablePath $ExecutablePath -ImageExecutablePath $ImageExecutablePath `
        -WptRoot $WptRoot -OutputDirectory $retained -Workers 2 -Suites $suites
    foreach($suite in $suites){foreach($n in 1..2){
        $folder=Join-Path $retained ('final-'+$suite+'-'+$n)
        $report=Get-ChildItem $folder -Filter report.json -Recurse|Select-Object -First 1
        if(-not $report){throw "Missing $suite $n report"}
        Copy-Item $report.FullName (Join-Path $target ('layout7-candidate-'+$suite+'-'+$n+'.json')) -Force
        $data=Get-Content $report.FullName -Raw|ConvertFrom-Json
        Write-Output ($suite+'-'+$n+' '+($data.counts|ConvertTo-Json -Compress))
    }}
}
if((Get-FileHash $ExecutablePath).Hash -ne $hash -or
   (Get-FileHash $ImageExecutablePath).Hash -ne $imageHash){throw 'Artifact changed'}
