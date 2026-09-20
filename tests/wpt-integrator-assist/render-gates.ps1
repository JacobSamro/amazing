param([string]$Executable, [string]$OutputDirectory, [string]$WptRoot='C:\Users\server\target\layout\wpt', [string[]]$ExcludeSelections=@())
$ErrorActionPreference = 'Stop'
# The shared runner normally invokes Git to verify HEAD. Read its detached
# pinned HEAD directly here: this lane must not execute Git on the build box.
$runner = Get-Content scripts/run-render-wpt.ps1 -Raw
$runner = $runner.Replace('$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))', '$repositoryRoot = (Get-Location).Path')
$runner = $runner.Replace('$actualRevision = (& git -C $WptRoot rev-parse HEAD).Trim()', '$actualRevision = (Get-Content (Join-Path $WptRoot ''.git/HEAD'') -Raw).Trim(); $LASTEXITCODE = 0')
# Return to this loop even when the existing runner reports an unexpected case.
$runner = $runner.Replace('if ($unexpected -ne 0) { exit 1 }', '')
# Long upstream IDs can exceed Win32 MAX_PATH under the lane tree. Only
# temporary directory names change; report IDs and assertions remain intact.
$runner = $runner.Replace('$testDirectory = Join-Path $runDirectory $id', '$testDirectory = Join-Path $runDirectory (Get-CaseDirectoryName $id)')
function Get-CaseDirectoryName([string]$id) {
 $sha=[Security.Cryptography.SHA256]::Create()
 try { [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($id))).Replace('-','').Substring(0,16) } finally { $sha.Dispose() }
}
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
