# Windows only; collect the named final gate epoch and verify retained provenance.
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$root='C:\Users\server\target\layout'
$expected=Get-Content "$root\layout3-final-recovery-sources.json" -Raw|ConvertFrom-Json
foreach($e in $expected.PSObject.Properties){if((Get-FileHash (Join-Path 'C:\Users\server\src\wt\layout' $e.Name)).Hash.ToLowerInvariant() -ne $e.Value){throw "Source drift: $($e.Name)"}}
$repo='C:\Users\server\src\wt\layout'
$known=@{};foreach($p in $expected.PSObject.Properties){$known[$p.Name]=$true}
Get-ChildItem "$repo\crates" -Recurse -File | Where-Object {($_.Extension -eq '.rs' -or $_.Name -in @('Cargo.toml','Cargo.lock')) -and $_.FullName -notmatch '\\target\\'} | ForEach-Object {
 $rel=$_.FullName.Substring($repo.Length+1).Replace('\','/')
 if(-not $known.ContainsKey($rel)){throw "Unexpected source: $rel"}
}
$reports=@()
Get-ChildItem "$root\layout3-recovery-evidence" -Filter report.json -Recurse | ForEach-Object {
    $item=Get-Content $_.FullName -Raw | ConvertFrom-Json
    $reports+=@{name=$_.Directory.Parent.Name;report=$item}
}
$builds=@{}
foreach($phase in @('baseline-recovery','final-recovery')) {
    $builds[$phase]=Get-Content "$root\layout3-$phase-build.json" -Raw|ConvertFrom-Json
}
foreach($phase in @('baseline-recovery','final-recovery')) {
 $b=$builds[$phase]
 foreach($pair in @(@($b.executable,$b.executable_sha256),@($b.image_executable,$b.image_executable_sha256))) {
  if((Get-FileHash $pair[0]).Hash.ToLowerInvariant() -ne $pair[1]){throw "Artifact changed: $($pair[0])"}
 }
 foreach($log in $b.logs.PSObject.Properties){
  if((Get-FileHash (Join-Path $root $log.Name)).Hash.ToLowerInvariant() -ne $log.Value){throw 'Build log changed'}
  if([int](Get-Content (Join-Path $root "$($log.Name).exit")) -ne 0){throw 'Nonzero build exit'}
 }
}
if([int](Get-Content "$root\layout3-final-recovery-workspace-repeat.log.exit") -ne 0){throw 'Workspace repeat failed'}
$units=@{}
foreach($phase in @('baseline-recovery','final-recovery')) {
    $units[$phase]=@(Select-String -Path "$root\layout3-$phase-tests.log" -Pattern '^test result:'|ForEach-Object {$_.Line})
}
$workspaceRepeat=@(Select-String -Path "$root\layout3-final-recovery-workspace-repeat.log" -Pattern '^test result:'|ForEach-Object {$_.Line})
$workspace=@(Select-String -Path "$root\layout3-final-recovery-workspace.log" -Pattern '^test result:'|ForEach-Object {$_.Line})
$audit=@{
 verified_files=@($expected.PSObject.Properties).Count
 unexpected_rust_or_cargo_files=0
 collection_verified_source_and_artifact_and_log_hashes=$true
 completed_utc=[DateTime]::UtcNow.ToString('o')
 workspace_repeat_log_sha256=(Get-FileHash "$root\layout3-final-recovery-workspace-repeat.log").Hash.ToLowerInvariant()
}
@{audit=$audit;reports=$reports;builds=$builds;units=$units;workspace=$workspace;workspace_repeat=$workspaceRepeat;baseline_image_sha256=(Get-FileHash "$root\layout3-baseline-recovery-images.exe").Hash.ToLowerInvariant()} | ConvertTo-Json -Depth 20 | Set-Content -Encoding UTF8 "$root\layout3-recovery-collected.json"
