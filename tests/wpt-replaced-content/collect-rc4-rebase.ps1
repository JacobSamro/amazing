param([string]$RepositoryRoot,[string]$Tag='acb57e5',[string]$OutputDirectory)
$ErrorActionPreference='Stop'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
foreach($kind in 'scope','controls','images','svg','focused'){foreach($n in 1,2){
 $name="$kind-after-$n"
 $run=Get-ChildItem (Join-Path $RepositoryRoot "target/rc4-$Tag-$name") -Directory | Sort-Object Name | Select-Object -Last 1
 if(!$run){throw "Missing $name"}
 $report=Get-Content (Join-Path $run.FullName report.json) -Raw | ConvertFrom-Json
 $report.manifest=$report.manifest.Substring($RepositoryRoot.Length+1).Replace('\','/')
 $report.run_directory=$run.Name
 $report | ConvertTo-Json -Depth 20 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory "$name.json")
}}
foreach($n in 1,2){Copy-Item (Join-Path $RepositoryRoot "target/rc4-$Tag-fixtures-after-$n/report.json") (Join-Path $OutputDirectory "fixtures-after-$n.json")}
$artifacts=@();foreach($kind in 'driver','images'){
 $name="rc4-$Tag-$kind-after.exe";$file=Get-Item (Join-Path $RepositoryRoot "target/$name")
 $artifacts+=@{file=$name;sha256=(Get-FileHash $file.FullName).Hash.ToLower();bytes=$file.Length;built_utc=$file.LastWriteTimeUtc.ToString('o')}
}
$artifacts | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory 'artifacts.json')
