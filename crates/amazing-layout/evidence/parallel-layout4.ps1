# Windows only. Run independent rendering comparisons against immutable binaries.
# Does not build; four single-suite jobs leave capacity for the other lanes.
param(
    [ValidateSet('baseline','final')][string[]]$Phases=@('baseline','final'),
    [ValidateRange(1,4)][int]$Workers=4
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$target='C:\Users\server\target\layout'
$output=Join-Path $target 'layout4-recovery-evidence'
$runner=Join-Path $PSScriptRoot 'run-layout4.ps1'
$queue=New-Object Collections.Queue
foreach($phase in $Phases){
    foreach($suite in @('geometry','tables','contents','floats','block-collapse','position','text-flow','flex-grid','paint-compositing','images')){foreach($repeat in @(1,2)){
        $directory=Join-Path $output "$phase-$suite-$repeat"
        $reports=@(Get-ChildItem $directory -Recurse -Filter report.json -ErrorAction SilentlyContinue)
        if($reports.Count -gt 1){throw "Ambiguous reports: $directory"}
        if($reports.Count -eq 0){$queue.Enqueue(@($phase,$repeat,$suite))}
    }
}}
$jobs=@()
try {
    while($queue.Count -or $jobs.Count){
        while($queue.Count -and $jobs.Count -lt $Workers){
            $item=$queue.Dequeue()
            $jobs+=Start-Job -ArgumentList $runner,$target,$output,$item[0],$item[1],$item[2] -ScriptBlock {
                param($runner,$target,$output,$phase,$repeat,$suite)
                $ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
                $artifactPhase=if($phase -eq 'baseline'){'dev-baseline'}else{'final'}
                & $runner -ExecutablePath "$target\layout4-$artifactPhase.exe" `
                    -ImageExecutablePath "$target\layout4-$artifactPhase-images.exe" `
                    -WptRoot "$target\wpt" -OutputDirectory $output -Phase $phase `
                    -Suites @($suite) -Repeat @($repeat)
            }
        }
        $finished=@($jobs|Where-Object {$_.State -in @('Completed','Failed','Stopped')})
        foreach($job in $finished){
            Receive-Job $job -ErrorAction Stop
            if($job.State -ne 'Completed'){throw "Rendering job failed: $($job.Id)"}
            Remove-Job $job
        }
        $jobs=@($jobs|Where-Object {$_.State -notin @('Completed','Failed','Stopped')})
        if($jobs.Count){Wait-Job -Job $jobs -Any -Timeout 1|Out-Null}
    }
} finally {
    $jobs|Stop-Job
    $jobs|Remove-Job -Force
}
