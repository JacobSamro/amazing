param([string]$Executable, [string]$OutputDirectory, [string]$WptRoot='C:\Users\server\target\layout\wpt', [string[]]$ExcludeSelections=@())
$ErrorActionPreference = 'Stop'
$runner = Get-Content scripts/run-render-wpt.ps1 -Raw
$runner = $runner.Replace('$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))', '$repositoryRoot = (Get-Location).Path')
# Return to this loop even when the existing runner reports an unexpected case.
$runner = $runner.Replace('if ($unexpected -ne 0) { exit 1 }', '')
$gate = [scriptblock]::Create($runner)
$manifests = @(Get-ChildItem tests/wpt-render/*-manifest.json) + @(Get-ChildItem tests/wpt-replaced-content/*manifest*.json)
foreach ($manifest in ($manifests | Where-Object { -not $_.Name.StartsWith('._') -and $_.BaseName -notin $ExcludeSelections })) {
 $pin = Get-Content $manifest.FullName -Raw | ConvertFrom-Json
 $roots = @($WptRoot, 'C:\Users\server\target\css\wpt', 'C:\Users\server\src\wt\replaced-content\target\wpt')
 $source = $null
 foreach ($candidate in $roots) {
  $head = (Get-Content (Join-Path $candidate '.git/HEAD') -Raw).Trim()
  if ($head -ne $pin.revision) { continue }
  $missing = @($pin.tests | Where-Object { -not (Test-Path (Join-Path $candidate $_.test)) }).Count
  if ($missing -eq 0) { $source=$candidate; break }
 }
 if (-not $source) { throw "No complete pinned checkout for $manifest" }
 & $gate -ManifestPath $manifest.FullName -WptRoot $source -ExecutablePath $Executable -OutputDirectory (Join-Path $OutputDirectory $manifest.BaseName) -SkipFetch -SkipBuild
}
