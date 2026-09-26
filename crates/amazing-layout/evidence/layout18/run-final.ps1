param(
    [Parameter(Mandatory=$true)][string]$SourceCommit,
    [string]$ExistingExecutableSha256 = ""
)
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
Set-Location (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..'))
Write-Output "LAYOUT-18 SOURCE LOCK: $SourceCommit"
Get-FileHash Cargo.lock, crates\amazing-layout\src\grid.rs, crates\amazing-layout\src\grid_repeat.rs, crates\amazing-layout\src\flex.rs
function Run-Cargo {
    param([string[]]$CargoArgs)
    & cargo @CargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo failed: $CargoArgs (exit $LASTEXITCODE)" }
}
function Lost-Passes {
    param([string]$Before, [string]$After)
    $old = Get-Content $Before -Raw | ConvertFrom-Json
    $new = Get-Content $After -Raw | ConvertFrom-Json
    $byId = @{}
    foreach ($case in $new.tests) {
        if ($byId.ContainsKey($case.id)) { throw "Duplicate result ID: $($case.id)" }
        $byId[$case.id] = $case
    }
    if ($old.tests.Count -ne $new.tests.Count -or $old.wpt_revision -ne $new.wpt_revision) {
        throw 'Incomparable render reports'
    }
    $losses = 0
    foreach ($case in $old.tests) {
        if (-not $byId.ContainsKey($case.id)) { throw "Missing result: $($case.id)" }
        if ($case.classification -eq 'pass' -and $byId[$case.id].classification -ne 'pass') {
            Write-Host "LOST PASS: $($case.id)"
            $losses++
        }
    }
    return $losses
}
try {
    $executable = Join-Path $env:CARGO_TARGET_DIR 'layout18-final.exe'
    if ($ExistingExecutableSha256) {
        if ((Get-FileHash $executable).Hash -ne $ExistingExecutableSha256) {
            throw 'Existing renderer hash differs from the completed source-locked build'
        }
        Write-Output 'Reusing verified renderer after fixture restoration'
    } else {
        Run-Cargo -CargoArgs @('build','-p','amazing','--release','--locked')
        Copy-Item (Join-Path $env:CARGO_TARGET_DIR 'release\amazing.exe') $executable
    }
    Get-FileHash $executable
    $totalLosses = 0
    $errors = 0
    foreach ($selection in @(
        @{name='full'; manifest='crates\amazing-layout\evidence\layout18\flex-grid-manifest.json'; before='crates\amazing-layout\evidence\layout18\flex-grid-before.json'},
        @{name='retained'; manifest='crates\amazing-layout\evidence\layout16\layout16-full-manifest.json'; before='crates\amazing-layout\evidence\layout16\after-paint-full.json'},
        @{name='values'; manifest='crates\amazing-layout\evidence\layout18\css-values-manifest.json'; before='crates\amazing-layout\evidence\layout18\css-values-before.json'}
    )) {
        $output = Join-Path $env:CARGO_TARGET_DIR ('layout18-final-' + $selection.name)
        & powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\run-fast.ps1" `
            -ManifestPath $selection.manifest -ExecutablePath $executable -OutputDirectory $output
        if ($LASTEXITCODE -ne 0) { throw "Render runner failed: $($selection.name)" }
        $run = Get-ChildItem $output -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1
        $report = Join-Path $run.FullName 'report.json'
        $totalLosses += Lost-Passes $selection.before $report
        $counts = (Get-Content $report -Raw | ConvertFrom-Json).counts
        $errors += $counts.error + $counts.timeout + $counts.unsupported + $counts.crash
        Write-Output "SELECTION $($selection.name) REPORT $report"
    }
    Write-Output "RENDER GATE: losses=$totalLosses non-rendered=$errors"
    if ($totalLosses -ne 0 -or $errors -ne 0) {
        throw 'Render regressions require review before the workspace gate'
    }
    Run-Cargo -CargoArgs @('nextest','run','-p','amazing-layout','--locked')
    Run-Cargo -CargoArgs @('test','-p','amazing-layout','--locked','--doc')
    Run-Cargo -CargoArgs @('nextest','run','--workspace','--locked')
    Run-Cargo -CargoArgs @('test','--workspace','--locked','--doc')
    Run-Cargo -CargoArgs @('test','-p','amazing','--release','--locked','--bin','amazing','incremental_','--','--ignored','--nocapture')
    Write-Output "LAYOUT-18 FINAL GATE PASS: $SourceCommit"
} catch {
    Write-Output ("LAYOUT-18 GATE ERROR: " + $_.Exception.Message)
    exit 1
}
