param([ValidateSet('before','after')][string]$Phase,[string]$RepositoryRoot)
$ErrorActionPreference='Stop'
Set-Location $RepositoryRoot
$fixtures=Join-Path $RepositoryRoot 'tests/wpt-replaced-content/rc4-fixtures'
$manifest=Get-Content (Join-Path $fixtures 'manifest.json') -Raw | ConvertFrom-Json
foreach($n in 1,2){
 $out=Join-Path $RepositoryRoot "target/rc4-fixtures-$Phase-$n"
 New-Item -ItemType Directory -Force $out | Out-Null
 $rows=@()
 foreach($t in $manifest.tests){
  $exe=Join-Path $RepositoryRoot "target/rc4-$($t.renderer)-$Phase.exe"
  $actual=Join-Path $out "$($t.id).ppm"
  $reference=Join-Path $out "$($t.id)-ref.ppm"
  & $exe render-wpt (Join-Path $fixtures $t.test) $actual 64 64 *> (Join-Path $out "$($t.id).log")
  if($LASTEXITCODE -ne 0){throw "Actual failed: $($t.id)"}
  & $exe render-wpt (Join-Path $fixtures $t.reference) $reference 64 64 *> (Join-Path $out "$($t.id)-ref.log")
  if($LASTEXITCODE -ne 0){throw "Reference failed: $($t.id)"}
  $a=[IO.File]::ReadAllBytes($actual);$b=[IO.File]::ReadAllBytes($reference)
  # Both artifacts emit P6 RGB with the same header. Count differing bytes;
  # PPM pixel differences are summarized separately from the saved files.
  $diff=0;for($i=0;$i -lt [Math]::Min($a.Length,$b.Length);$i++){if($a[$i] -ne $b[$i]){$diff++}}
  $diff += [Math]::Abs($a.Length-$b.Length)
  $rows+=@{id=$t.id;actual_sha256=(Get-FileHash $actual).Hash.ToLower();reference_sha256=(Get-FileHash $reference).Hash.ToLower();differing_bytes=$diff;match=($diff -eq 0)}
 }
 $rows | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $out 'report.json')
 "$Phase fixtures run $n : $(($rows | Where-Object match).Count)/$($rows.Count)"
}
