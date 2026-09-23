# Windows only: verify the final source set and retained artifact/log provenance.
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$target='C:\Users\server\target\layout'
$repo='C:\Users\server\src\wt\layout'
$wptRevision=(& git -C "$target\wpt" rev-parse HEAD).Trim()
if($LASTEXITCODE -or $wptRevision -ne 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293'){throw 'Wrong WPT source revision'}
$dirty=@(& git -C "$target\wpt" status --porcelain --untracked-files=no)
if($LASTEXITCODE -or $dirty.Count){throw 'WPT tracked files modified'}
$expected=Get-Content "$target\final-sources.json" -Raw|ConvertFrom-Json
foreach($entry in $expected.PSObject.Properties){
    if((Get-FileHash (Join-Path $repo $entry.Name)).Hash.ToLowerInvariant() -ne $entry.Value){throw "Source drift: $($entry.Name)"}
}
Get-ChildItem "$repo\crates" -Recurse -File | Where-Object {
    ($_.Extension -eq '.rs' -or $_.Name -in @('Cargo.toml','Cargo.lock')) -and $_.FullName -notmatch '\\target\\'
} | ForEach-Object {
    $relative=$_.FullName.Substring($repo.Length+1).Replace('\','/')
    if(-not $expected.PSObject.Properties[$relative]){throw "Unexpected source: $relative"}
}
$gateFiles=@{}
foreach($suite in @('tables','geometry','floats','block-collapse','position','text-flow','flex-grid','paint-compositing','images','contents')){
    $path=if($suite -in @('tables','floats','contents')){"crates/amazing-layout/evidence/$suite-manifest.json"}
        elseif($suite -eq 'images'){'tests/wpt-replaced-content/image-manifest.json'}
        else{"tests/wpt-render/$suite-manifest.json"}
    $gateFiles[$path]=(Get-FileHash (Join-Path $repo $path)).Hash.ToLowerInvariant()
}
foreach($path in @('scripts/run-render-wpt.ps1','crates/amazing-layout/evidence/run-layout4.ps1')){
    $gateFiles[$path]=(Get-FileHash (Join-Path $repo $path)).Hash.ToLowerInvariant()
}
$reports=@(Get-ChildItem "$target\layout4-recovery-evidence" -Recurse -Filter report.json | ForEach-Object {
    @{name=$_.Directory.Parent.Name;report=(Get-Content $_.FullName -Raw|ConvertFrom-Json)}
})
$builds=@{};$logs=@{}
foreach($phase in @('dev-baseline','final')){
    $b=Get-Content "$target\layout4-$phase-build.json" -Raw|ConvertFrom-Json
    $builds[$phase]=$b
    foreach($pair in @(@($b.executable,$b.executable_sha256),@($b.image_executable,$b.image_executable_sha256))){
        if((Get-FileHash $pair[0]).Hash.ToLowerInvariant() -ne $pair[1]){throw 'Artifact drift'}
    }
    foreach($log in $b.logs.PSObject.Properties){
        $path=Join-Path $target $log.Name
        if((Get-FileHash $path).Hash.ToLowerInvariant() -ne $log.Value){throw 'Log drift'}
        if([int](Get-Content "$path.exit") -ne 0){throw 'Build or test failed'}
        $logs[$log.Name]=@(Select-String $path -Pattern 'Summary .*tests run:|^test result:|^\s*Doc-tests '|ForEach-Object {$_.Line})
    }
}
$repeatHashes=@{}
foreach($name in @('workspace','workspace-doc')){
    $path="$target\layout4-final-$name-repeat.log"
    if([int](Get-Content "$path.exit") -ne 0){throw 'Workspace repeat failed'}
    $logs[(Split-Path $path -Leaf)]=@(Select-String $path -Pattern 'Summary .*tests run:|^test result:|^\s*Doc-tests '|ForEach-Object {$_.Line})
    $repeatHashes[(Split-Path $path -Leaf)]=(Get-FileHash $path).Hash.ToLowerInvariant()
}
@{reports=$reports;builds=$builds;logs=$logs;gate_files=$gateFiles;audit=@{
    verified_files=@($expected.PSObject.Properties).Count
    unexpected_rust_or_cargo_files=0
    wpt_tracked_files_clean=$true
    wpt_revision=$wptRevision
    collection_verified_source_and_artifact_and_log_hashes=$true
    completed_utc=[DateTime]::UtcNow.ToString('o')
    workspace_repeat_log_hashes=$repeatHashes
}} | ConvertTo-Json -Depth 20 | Set-Content -Encoding UTF8 "$target\layout4-collected.json"
