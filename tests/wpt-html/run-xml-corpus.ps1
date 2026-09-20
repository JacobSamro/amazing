# Parser-only diagnostic over XML test/reference files in the unchanged cascade
# selection. This is not a render gate or a WPT conformance pass rate.
param(
    [Parameter(Mandatory=$true)][string]$WptRoot,
    [Parameter(Mandatory=$true)][string]$ExecutablePath,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$manifest = Get-Content (Join-Path $repo 'tests\wpt-render\cascade-manifest.json') -Raw | ConvertFrom-Json
# Read the detached checkout marker without invoking Git on the build server.
$revision = (Get-Content (Join-Path $WptRoot '.git\HEAD') -Raw).Trim()
if ($revision -ne $manifest.revision) { throw "WPT detached HEAD $revision differs from manifest $($manifest.revision)" }
$paths = New-Object 'Collections.Generic.SortedSet[string]'
$rootPrefix = [IO.Path]::GetFullPath($WptRoot).TrimEnd('\') + '\'
foreach ($test in $manifest.tests) {
    $path = [IO.Path]::GetFullPath((Join-Path $WptRoot $test.test))
    if (@('.xht','.xhtml') -contains [IO.Path]::GetExtension($path)) { [void]$paths.Add($path) }
    $source = Get-Content -LiteralPath $path -Raw
    foreach ($link in [regex]::Matches($source, '<link\b[^>]*>', 'IgnoreCase')) {
        if ($link.Value -notmatch '\brel\s*=\s*["''](?:match|mismatch)["'']') { continue }
        $hrefMatch = [regex]::Match($link.Value, '\bhref\s*=\s*["'']([^"'']+)["'']', 'IgnoreCase')
        if (-not $hrefMatch.Success) { continue }
        $href = $hrefMatch.Groups[1].Value
        if (@('.xht','.xhtml') -notcontains [IO.Path]::GetExtension($href)) { continue }
        $reference = if ($href.StartsWith('/')) { Join-Path $WptRoot $href.TrimStart('/') } else { Join-Path (Split-Path $path) $href }
        $reference = [IO.Path]::GetFullPath($reference)
        if (-not $reference.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Reference escapes WPT root' }
        [void]$paths.Add($reference)
    }
}
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$results = @()
foreach ($path in $paths) {
    $relative = $path.Substring($rootPrefix.Length).Replace('\','/')
    $id = $relative -replace '[^a-zA-Z0-9_.-]', '_'
    $out = Join-Path $OutputDirectory "$id.stdout"
    $err = Join-Path $OutputDirectory "$id.stderr"
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $ExecutablePath
    $start.Arguments = '"' + $path + '"'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'Could not start XML parser' }
    # Drain both pipes while the parser runs (large DOM dumps must not deadlock).
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $completed = $process.WaitForExit(20000)
    if (-not $completed) { $process.Kill(); $process.WaitForExit(); $status = 'timeout' }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $detail = $stderrTask.GetAwaiter().GetResult()
    [IO.File]::WriteAllText($out, $stdout)
    [IO.File]::WriteAllText($err, $detail)
    if ($completed) {
        if ($process.ExitCode -eq 0) { $status = 'parsed' }
        elseif ($detail -match 'XML Unsupported') { $status = 'unsupported' }
        else { $status = 'error' }
    }
    $process.Dispose()
    $results += [ordered]@{
        path = $relative; status = $status
        source_sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        output_sha256 = (Get-FileHash -LiteralPath $out -Algorithm SHA256).Hash.ToLowerInvariant()
        detail = [IO.File]::ReadAllText($err).Trim()
    }
}
$counts = [ordered]@{ parsed=0; error=0; timeout=0; unsupported=0 }
foreach ($result in $results) { $counts[$result.status]++ }
$report = [ordered]@{ schema=1; kind='parser-only-cascade-xml'; wpt_revision=$revision; counts=$counts; results=$results }
$report | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $OutputDirectory 'results.json') -Encoding UTF8
$counts | ConvertTo-Json -Compress
