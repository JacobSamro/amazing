param([string]$Executable, [string]$OutputDirectory)
$ErrorActionPreference='Stop'
$root=(Resolve-Path tests/wpt-integrator-assist/root-fixtures).Path
$runner=(Resolve-Path scripts/run-render-wpt.ps1).Path
$manifest=(Resolve-Path tests/wpt-integrator-assist/root-mapping-manifest.json).Path
$script:gitCalls=0
function global:git { $script:gitCalls++; throw 'Server-side Git must never be called' }
$hashes=@()
foreach ($round in 1..2) {
 $global:LASTEXITCODE=0
 & $runner -ManifestPath $manifest -WptRoot $root -RevisionMarker revision.txt -ExecutablePath $Executable -OutputDirectory (Join-Path $OutputDirectory "round-$round") -ViewportWidth 16 -ViewportHeight 16 -SkipBuild -SkipFetch
 if ($LASTEXITCODE -ne 0) { throw 'Root mapping runner failed' }
 $reportPath=Get-ChildItem (Join-Path $OutputDirectory "round-$round") -Filter report.json -Recurse | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
 $report=Get-Content $reportPath.FullName -Raw | ConvertFrom-Json
 if ($report.counts.pass -ne 3 -or $report.counts.unsupported -ne 2 -or $report.unexpected -ne 0) { throw 'Root mapping classifications differ' }
 $records=@($report.tests | ForEach-Object { [ordered]@{id=$_.id; classification=$_.classification; actual=$_.actual_sha256; reference=$_.reference_sha256; diff=$_.diff_sha256} })
 $path=Join-Path $OutputDirectory "root-$round.json"
 $records | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 $path
 $hashes+=(Get-FileHash $path -Algorithm SHA256).Hash
}
$rejected=0
foreach ($marker in @('wrong-revision.txt','missing-marker.txt')) {
 try {
  & $runner -ManifestPath $manifest -WptRoot $root -RevisionMarker $marker -ExecutablePath $Executable -OutputDirectory (Join-Path $OutputDirectory 'rejected') -SkipBuild -SkipFetch
  throw 'Invalid marker was accepted'
 } catch {
  if ($_.Exception.Message -notmatch 'revision marker') { throw }
  $rejected++
 }
}
if ($hashes[0] -ne $hashes[1] -or $script:gitCalls -ne 0) { throw 'Unstable results or Git invoked' }
[ordered]@{suite='scaffolding-only';wpt_credit=0;cases=5;pass=3;unsupported=2;repeats=2;sha256=$hashes[0];rejected_markers=$rejected;git_calls=$script:gitCalls} | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'summary.json')
Remove-Item Function:\git
