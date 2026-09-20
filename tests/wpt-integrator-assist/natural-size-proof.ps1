param([string]$Before, [string]$After, [string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$results=@()
foreach ($item in @(@{name='before';exe=$Before},@{name='after';exe=$After})) {
 $actual=Join-Path $OutputDirectory ($item.name+'-actual.ppm')
 $reference=Join-Path $OutputDirectory ($item.name+'-reference.ppm')
 & $item.exe render-wpt tests/wpt-integrator-assist/natural-size.html $actual 8 8
 if ($LASTEXITCODE -ne 0) { throw 'Actual render failed' }
 & $item.exe render-wpt tests/wpt-integrator-assist/natural-size-ref.html $reference 8 8
 if ($LASTEXITCODE -ne 0) { throw 'Reference render failed' }
 $a=(Get-FileHash $actual -Algorithm SHA256).Hash
 $b=(Get-FileHash $reference -Algorithm SHA256).Hash
 $results += [ordered]@{name=$item.name; actual=$a; reference=$b; matches=($a -eq $b)}
}
if ($results[0].matches -or -not $results[1].matches) { throw 'Expected baseline mismatch and fixed natural aspect ratio' }
$results | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'proof.json')
