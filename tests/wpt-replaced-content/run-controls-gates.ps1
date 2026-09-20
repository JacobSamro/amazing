param([ValidateSet('before','after')][string]$Phase, [string]$RepositoryRoot)
$ErrorActionPreference='Stop'
Set-Location $RepositoryRoot
foreach($run in 1,2) {
 foreach($kind in 'controls','images','focused') {
  $manifest=if($kind -eq 'controls'){'tests/wpt-replaced-content/controls-manifest.json'}elseif($kind -eq 'images'){'tests/wpt-replaced-content/image-manifest.json'}else{'tests/wpt-render/paint-compositing-manifest.json'}
  $exe=if($kind -eq 'images'){"target/rc3-images-$Phase.exe"}else{"target/rc3-driver-$Phase.exe"}
  $fonts=if($kind -eq 'images'){'Fallback font; static image adapter artifact'}else{'Amazing text provider; pinned WPT font tree'}
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-render-wpt.ps1 -ManifestPath $manifest -WptRoot target/wpt -OutputDirectory "target/rc3-$kind-$Phase-$run" -ExecutablePath $exe -FontSet $fonts -SkipFetch -SkipBuild *> "target/rc3-$kind-$Phase-$run.log"
  if($LASTEXITCODE -ne 0){Get-Content "target/rc3-$kind-$Phase-$run.log" -Tail 20;exit $LASTEXITCODE}
  "$kind $Phase $run"
  Get-Content "target/rc3-$kind-$Phase-$run.log" -Tail 2
 }
}
