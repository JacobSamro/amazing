# win-001 only: source-guarded LAYOUT-8 workspace gate.
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..\..'))
Set-Location $root
$env:CARGO_TARGET_DIR='C:\Users\server\target\layout'
$env:CARGO_INCREMENTAL='0'
$expected=Get-Content (Join-Path $PSScriptRoot 'final-sources.json') -Raw|ConvertFrom-Json
function Verify-Source {
    foreach($entry in $expected.PSObject.Properties){
        if((Get-FileHash (Join-Path $root $entry.Name)).Hash.ToLowerInvariant() -ne $entry.Value){
            throw "Source drift: $($entry.Name)"
        }
    }
}
Verify-Source
$target=$env:CARGO_TARGET_DIR
$nextest=Join-Path $target 'layout8-final-workspace-nextest.log'
$docs=Join-Path $target 'layout8-final-workspace-doc.log'
& cmd /c "cargo nextest run --workspace --locked --status-level fail > `"$nextest`" 2>&1"
$nextestExit=$LASTEXITCODE
Get-Content $nextest -Tail 20
if($nextestExit -ne 0){throw "Workspace nextest failed ($nextestExit)"}
Verify-Source
& cmd /c "cargo test --workspace --locked --doc > `"$docs`" 2>&1"
$docsExit=$LASTEXITCODE
Get-Content $docs -Tail 30
if($docsExit -ne 0){throw "Workspace docs failed ($docsExit)"}
Verify-Source
@{
    nextest_log=$nextest
    nextest_log_sha256=(Get-FileHash $nextest).Hash.ToLowerInvariant()
    nextest_exit=$nextestExit
    docs_log=$docs
    docs_log_sha256=(Get-FileHash $docs).Hash.ToLowerInvariant()
    docs_exit=$docsExit
    source_manifest_sha256=(Get-FileHash (Join-Path $PSScriptRoot 'final-sources.json')).Hash.ToLowerInvariant()
    source_count=@($expected.PSObject.Properties).Count
    completed_utc=[DateTime]::UtcNow.ToString('o')
}|ConvertTo-Json|Set-Content (Join-Path $target 'layout8-final-workspace.json')
