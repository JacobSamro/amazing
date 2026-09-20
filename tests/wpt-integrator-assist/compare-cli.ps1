param([string]$Baseline, [string[]]$Candidates, [string]$Output)
$ErrorActionPreference = 'Stop'
function Normalize($records) {
 foreach ($r in $records) {
  if ($r.args[0] -in @('webrtc-loopback','turn-probe','secure-media-loopback')) {
   $r.stdout = $r.stdout -replace '127\.0\.0\.1:\d+', '127.0.0.1:<ephemeral-port>'
  }
 }
 ($records | ForEach-Object {
  [ordered]@{args=@($_.args); exit=[int]$_.exit; timeout=[bool]$_.timeout; stdout=[string]$_.stdout; stderr=[string]$_.stderr; artifact=$_.artifact} | ConvertTo-Json -Depth 6 -Compress
 }) -join "`n"
}
$before = (Get-Content $Baseline -Raw | ConvertFrom-Json)
$normalizedBefore = Normalize (Get-Content $Baseline -Raw | ConvertFrom-Json)
$results = @()
foreach ($path in $Candidates) {
 $after = (Get-Content $path -Raw | ConvertFrom-Json)
 if ($before.Count -ne $after.Count) { throw 'Case count changed' }
 $exact = 0
 for ($i=0; $i -lt $before.Count; $i++) {
  if ((ConvertTo-Json $before[$i] -Depth 6 -Compress) -ceq (ConvertTo-Json $after[$i] -Depth 6 -Compress)) { $exact++ }
 }
 $normalized = Normalize $after
 if ($normalized -cne $normalizedBefore) { throw "CLI behavior differs: $path" }
 $sha = [Security.Cryptography.SHA256]::Create()
 try { $hash = [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($normalized))).Replace('-','').ToLowerInvariant() } finally { $sha.Dispose() }
 $results += [ordered]@{candidate=$path; cases=$before.Count; exact=$exact; normalized_matches=$after.Count; sha256=$hash}
}
$results | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 $Output
$results | Format-Table
