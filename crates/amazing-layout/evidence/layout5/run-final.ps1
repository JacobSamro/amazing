# win-001: immutable LAYOUT-5 renderer, pinned WPT, bounded concurrent suites.
param(
    [Parameter(Mandatory=$true)][string]$ExecutablePath,
    [Parameter(Mandatory=$true)][string]$ImageExecutablePath,
    [string]$WptRoot = 'C:\Users\server\target\layout\wpt',
    [string]$OutputDirectory = 'C:\Users\server\target\layout\layout5-final-evidence',
    [ValidateRange(1,4)][int]$Workers = 3,
    [string[]]$Suites = @('flex-grid-broad','flex-grid','geometry','tables','floats','block-collapse','position','text-flow','contents','paint-compositing','images')
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$runner=Join-Path $PSScriptRoot '..\run-layout4.ps1'
$exeHash=(Get-FileHash $ExecutablePath).Hash
$imageHash=(Get-FileHash $ImageExecutablePath).Hash
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$queue=New-Object Collections.Queue
foreach($suite in $Suites){foreach($repeat in @(1,2)){$queue.Enqueue(@($suite,$repeat))}}
$jobs=@()
try {
    while($queue.Count -or $jobs.Count){
        while($queue.Count -and $jobs.Count -lt $Workers){
            $item=$queue.Dequeue()
            $jobs+=Start-Job -ArgumentList $runner,$ExecutablePath,$ImageExecutablePath,$WptRoot,$OutputDirectory,$item[0],$item[1] -ScriptBlock {
                param($runner,$exe,$image,$wpt,$output,$suite,$repeat)
                $ErrorActionPreference='Stop'
                & $runner -ExecutablePath $exe -ImageExecutablePath $image -WptRoot $wpt `
                    -OutputDirectory $output -Phase 'final' -Suites @($suite) -Repeat @($repeat)
            }
        }
        $done=@($jobs|Where-Object {$_.State -in @('Completed','Failed','Stopped')})
        foreach($job in $done){
            Receive-Job $job -ErrorAction Stop
            if($job.State -ne 'Completed'){throw "Render failed: $($job.Id)"}
            Remove-Job $job
        }
        $jobs=@($jobs|Where-Object {$_.State -notin @('Completed','Failed','Stopped')})
        if($jobs.Count){Wait-Job -Job $jobs -Any -Timeout 1|Out-Null}
    }
} finally {
    $jobs|Stop-Job
    $jobs|Remove-Job -Force
}
if((Get-FileHash $ExecutablePath).Hash -ne $exeHash -or (Get-FileHash $ImageExecutablePath).Hash -ne $imageHash){throw 'Artifact changed during rendering'}
