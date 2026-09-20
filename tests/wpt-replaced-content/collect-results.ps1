param([string]$RepositoryRoot, [string]$OutputDirectory = $PSScriptRoot)
$ErrorActionPreference = 'Stop'
$names = @('focused-before','focused-final-1','focused-final-2','images-before','images-final-1','images-final-2','scope-final-before','scope-final-after-1','scope-final-after-2')
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
foreach ($name in $names) {
 $directory = Join-Path $RepositoryRoot "target\rc1-$name"
 $run = Get-ChildItem $directory -Directory | Sort-Object Name | Select-Object -Last 1
 if (!$run) { throw "Missing run: $name" }
 $report = Get-Content (Join-Path $run.FullName 'report.json') -Raw | ConvertFrom-Json
 $report.manifest = $report.manifest.Substring($RepositoryRoot.Length+1).Replace('\','/')
 $report.run_directory = $run.Name
 $report | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 (Join-Path $OutputDirectory "$name.json")
}
