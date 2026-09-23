$ErrorActionPreference='Stop'
$owned=@('crates/amazing-fetch','crates/amazing-net','crates/amazing-net-tls','crates/amazing-net-brotli','docs/lanes/net_TODO.md')
function SourceHash {
  $lines=Get-ChildItem $owned -Recurse -File | Sort-Object FullName | ForEach-Object { "$($_.FullName.Substring($PWD.Path.Length)) $((Get-FileHash $_.FullName -Algorithm SHA256).Hash)" }
  $bytes=[Text.Encoding]::UTF8.GetBytes(($lines -join "`n"))
  $digest=[Security.Cryptography.SHA256]::Create().ComputeHash($bytes)
  return [BitConverter]::ToString($digest).Replace('-','')
}
function RunStep([string]$label,[scriptblock]$command) {
  Write-Output "START $label"
  & $command
  if ($LASTEXITCODE -ne 0) { throw "$label failed: $LASTEXITCODE" }
  Write-Output "PASS $label"
}
$before=SourceHash
Write-Output "SOURCE_BEFORE $before"
RunStep 'url-before' { cargo run --locked -p amazing-net --example url_wpt }
RunStep 'idna-before' { cargo run --locked -p amazing-net --example idna_wpt }
RunStep 'mime-before' { cargo run --locked -p amazing-fetch --example corpora_wpt }
RunStep 'cookies-before' { cargo run --locked -p amazing-fetch --example cookies_wpt }
RunStep 'setters-before' { cargo run --locked -p amazing-net --example setters_wpt }
RunStep 'lane-nextest' { cargo nextest run --locked --status-level fail --final-status-level fail -p amazing-net -p amazing-net-tls -p amazing-net-brotli -p amazing-fetch }
RunStep 'header-wpt' { cargo test --locked -p amazing-fetch --test wpt_headers_xhr -- --nocapture }
RunStep 'fetch-xhr-wpt' { cargo test --locked -p amazing-fetch --test wpt_fetch_xhr -- --nocapture }
RunStep 'lane-doc' { cargo test --locked --doc -p amazing-net -p amazing-net-tls -p amazing-net-brotli -p amazing-fetch }
RunStep 'workspace-nextest' { cargo nextest run --workspace --locked --status-level fail --final-status-level fail }
RunStep 'workspace-doc' { cargo test --workspace --locked --doc }
RunStep 'url-after' { cargo run --locked -p amazing-net --example url_wpt }
RunStep 'idna-after' { cargo run --locked -p amazing-net --example idna_wpt }
RunStep 'mime-after' { cargo run --locked -p amazing-fetch --example corpora_wpt }
RunStep 'cookies-after' { cargo run --locked -p amazing-fetch --example cookies_wpt }
RunStep 'setters-after' { cargo run --locked -p amazing-net --example setters_wpt }
$after=SourceHash
Write-Output "SOURCE_AFTER $after"
if ($before -ne $after) { throw 'source changed during gate' }
Write-Output 'NET24_GATE_OK'
