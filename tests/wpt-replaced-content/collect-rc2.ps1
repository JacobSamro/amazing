param([string]$RepositoryRoot, [string]$OutputDirectory = "$PSScriptRoot\rc2")
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
foreach ($kind in 'images','svg','scope','focused') {
 foreach ($phase in 'before','after') {
  foreach ($runNumber in 1,2) {
   $name="$kind-$phase-$runNumber"
   $directory=Join-Path $RepositoryRoot "target\rc2h-$name"
   $run=Get-ChildItem $directory -Directory | Sort-Object Name | Select-Object -Last 1
   if (!$run) {throw "Missing $name"}
   $report=Get-Content (Join-Path $run.FullName 'report.json') -Raw | ConvertFrom-Json
   $report.manifest=$report.manifest.Substring($RepositoryRoot.Length+1).Replace('\','/')
   $report.run_directory=$run.Name
   $report | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory "$name.json")
  }
 }
}
