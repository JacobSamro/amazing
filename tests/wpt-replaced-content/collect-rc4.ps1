param([string]$RepositoryRoot,[string]$OutputDirectory="$PSScriptRoot\rc4")
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
foreach($kind in 'scope','controls','images','svg','focused'){foreach($phase in 'before','after'){foreach($n in 1,2){
 $name="$kind-$phase-$n"
 $run=Get-ChildItem (Join-Path $RepositoryRoot "target/rc4-$name") -Directory | Sort-Object Name | Select-Object -Last 1
 if(!$run){throw "Missing $name"}
 $report=Get-Content (Join-Path $run.FullName report.json) -Raw | ConvertFrom-Json
 $report.manifest=$report.manifest.Substring($RepositoryRoot.Length+1).Replace('\','/')
 $report.run_directory=$run.Name
 $report | ConvertTo-Json -Depth 20 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory "$name.json")
}}}
foreach($phase in 'before','after'){foreach($n in 1,2){
 Copy-Item (Join-Path $RepositoryRoot "target/rc4-fixtures-$phase-$n/report.json") (Join-Path $OutputDirectory "fixtures-$phase-$n.json")
}}
$artifacts=@();foreach($phase in 'before','after'){foreach($kind in 'driver','images'){
 $name="rc4-$kind-$phase.exe";$file=Get-Item (Join-Path $RepositoryRoot "target/$name")
 $artifacts+=@{file=$name;sha256=(Get-FileHash $file.FullName).Hash.ToLower();bytes=$file.Length;built_utc=$file.LastWriteTimeUtc.ToString('o')}
}}
$artifacts | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'artifacts.json')
$hashes=@{};foreach($p in @('crates/amazing-image/src/svg.rs','crates/amazing-image/src/svg_transform.rs','crates/amazing-image/tests/svg.rs','crates/amazing-paint/src/control_raster.rs','crates/amazing-paint/src/control_tests.rs','Cargo.lock','crates/amazing-shell-gpui/Cargo.lock')){
 $hashes[$p]=(Get-FileHash (Join-Path $RepositoryRoot $p)).Hash.ToLower()
}
$hashes | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'source-hashes.json')
