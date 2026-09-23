param([ValidateSet('before','after')][string]$Phase,[string]$RepositoryRoot)
$ErrorActionPreference='Stop'
Set-Location $RepositoryRoot
foreach($run in 1,2){foreach($kind in 'scope','controls','images','svg','focused'){
 $manifest=switch($kind){
  'scope' {'tests/wpt-replaced-content/manifest.json'}
  'controls' {'tests/wpt-replaced-content/controls-manifest.json'}
  'images' {'tests/wpt-replaced-content/image-manifest.json'}
  'svg' {'tests/wpt-replaced-content/svg-manifest.json'}
  'focused' {'tests/wpt-render/paint-compositing-manifest.json'}
 }
 $exe=if($kind -in @('images','svg')){"target/rc4-images-$Phase.exe"}else{"target/rc4-driver-$Phase.exe"}
 $fonts=if($kind -in @('images','svg')){'Fallback font; static image adapter artifact'}else{'Amazing text provider; pinned WPT font tree'}
 powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-render-wpt.ps1 -ManifestPath $manifest -WptRoot target/wpt -OutputDirectory "target/rc4-$kind-$Phase-$run" -ExecutablePath $exe -FontSet $fonts -SkipFetch -SkipBuild *> "target/rc4-$kind-$Phase-$run.log"
 if($LASTEXITCODE -ne 0){Get-Content "target/rc4-$kind-$Phase-$run.log" -Tail 20;exit $LASTEXITCODE}
 "$kind $Phase $run"
 Get-Content "target/rc4-$kind-$Phase-$run.log" -Tail 2
}}
