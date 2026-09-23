param(
    [string]$ProfilePath = "",
    [string]$ManifestPath = "",
    [string]$WptRoot = "",
    [string]$RevisionMarker = "",
    [string]$OutputDirectory = "",
    [string]$ExecutablePath = "",
    [string]$FontSet = "Amazing text provider: WPT @font-face via HarfRust 0.13.2 + Swash 0.2.10; deterministic fallback otherwise",
    [int]$ViewportWidth = 800,
    [int]$ViewportHeight = 600,
    [int]$TimeoutSeconds = 20,
    [switch]$SkipFetch,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
if ($ProfilePath) {
    if (-not [IO.Path]::IsPathRooted($ProfilePath)) {
        $ProfilePath = Join-Path $repositoryRoot $ProfilePath
    }
    $ProfilePath = [IO.Path]::GetFullPath($ProfilePath)
    $profile = Get-Content -LiteralPath $ProfilePath -Raw | ConvertFrom-Json
    if ([int]$profile.schema -ne 1) {
        throw "Unsupported render WPT profile schema: $($profile.schema)"
    }
    $profileProperties = @($profile.PSObject.Properties.Name)
    if (-not $PSBoundParameters.ContainsKey("ManifestPath") -and
        $profileProperties -contains "manifest_path") {
        $ManifestPath = [string]$profile.manifest_path
    }
    if (-not $PSBoundParameters.ContainsKey("WptRoot") -and
        $profileProperties -contains "wpt_root") {
        $WptRoot = [string]$profile.wpt_root
    }
    if (-not $PSBoundParameters.ContainsKey("OutputDirectory") -and
        $profileProperties -contains "output_directory") {
        $OutputDirectory = [string]$profile.output_directory
    }
    if (-not $PSBoundParameters.ContainsKey("ExecutablePath") -and
        $profileProperties -contains "executable_path") {
        $ExecutablePath = [string]$profile.executable_path
    }
    if (-not $PSBoundParameters.ContainsKey("FontSet") -and
        $profileProperties -contains "font_set") {
        $FontSet = [string]$profile.font_set
    }
    if (-not $PSBoundParameters.ContainsKey("ViewportWidth") -and
        $profileProperties -contains "viewport_width") {
        $ViewportWidth = [int]$profile.viewport_width
    }
    if (-not $PSBoundParameters.ContainsKey("ViewportHeight") -and
        $profileProperties -contains "viewport_height") {
        $ViewportHeight = [int]$profile.viewport_height
    }
    if (-not $PSBoundParameters.ContainsKey("TimeoutSeconds") -and
        $profileProperties -contains "timeout_seconds") {
        $TimeoutSeconds = [int]$profile.timeout_seconds
    }
}
if (-not $ManifestPath) {
    $ManifestPath = Join-Path $repositoryRoot "tests\wpt-render\smoke.json"
}
if (-not $WptRoot) {
    $WptRoot = Join-Path $repositoryRoot "target\wpt"
}
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repositoryRoot "target\wpt-results"
}
if (-not $ExecutablePath) {
    $ExecutablePath = Join-Path $repositoryRoot "target\debug\amazing.exe"
}
if ($ViewportWidth -lt 1 -or $ViewportWidth -gt 8192 -or
    $ViewportHeight -lt 1 -or $ViewportHeight -gt 8192) {
    throw "ViewportWidth and ViewportHeight must be between 1 and 8192."
}
if ($TimeoutSeconds -lt 1 -or $TimeoutSeconds -gt 600) {
    throw "TimeoutSeconds must be between 1 and 600."
}

if (-not [IO.Path]::IsPathRooted($ManifestPath)) { $ManifestPath = Join-Path $repositoryRoot $ManifestPath }
if (-not [IO.Path]::IsPathRooted($WptRoot)) { $WptRoot = Join-Path $repositoryRoot $WptRoot }
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory = Join-Path $repositoryRoot $OutputDirectory }
if (-not [IO.Path]::IsPathRooted($ExecutablePath)) { $ExecutablePath = Join-Path $repositoryRoot $ExecutablePath }
$ManifestPath = [IO.Path]::GetFullPath($ManifestPath)
$WptRoot = [IO.Path]::GetFullPath($WptRoot)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$ExecutablePath = [IO.Path]::GetFullPath($ExecutablePath)
$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
$revision = [string]$manifest.revision
if ($revision -notmatch '^[0-9a-f]{40}$') {
    throw "Manifest revision must be a full 40-character Git object id."
}

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with code $LASTEXITCODE."
    }
}

if (-not $SkipBuild) {
    Invoke-Checked "cargo" @("build", "--package", "amazing", "--locked")
}
if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
    throw "Amazing executable not found: $ExecutablePath"
}

# Git stays on the editing machine. Provision the pinned WPT checkout there
# and transfer it to the build box. -SkipFetch remains accepted for callers;
# this runner never invokes Git, even when that switch is omitted.
# A detached .git/HEAD is itself a revision marker. For an archive without Git
# metadata, pass a marker written from `git rev-parse HEAD` on the editing host.
if (-not $RevisionMarker) { $RevisionMarker = Join-Path $WptRoot ".git\HEAD" }
if (-not [IO.Path]::IsPathRooted($RevisionMarker)) { $RevisionMarker = Join-Path $WptRoot $RevisionMarker }
if (-not (Test-Path -LiteralPath $RevisionMarker -PathType Leaf)) {
    throw "Missing WPT revision marker. Transfer a verified checkout/marker from the editing host and pass -RevisionMarker."
}
$actualRevision = (Get-Content -LiteralPath $RevisionMarker -Raw).Trim().ToLowerInvariant()
if ($actualRevision -notmatch '^[0-9a-f]{40}$' -or $actualRevision -ne $revision) {
    throw "WPT revision marker is '$actualRevision'; expected detached revision $revision. Verify the checkout on the editing host."
}

function ConvertTo-CommandArgument {
    param([string]$Value)
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Invoke-FixedRender {
    param(
        [string]$InputPath,
        [string]$OutputPath,
        [string]$LogStem
    )
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $ExecutablePath
    $arguments = @(
        "render-wpt",
        $InputPath,
        $OutputPath,
        [string]$ViewportWidth,
        [string]$ViewportHeight
    ) | ForEach-Object { ConvertTo-CommandArgument ([string]$_) }
    $start.Arguments = $arguments -join " "
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    # Explicit URL root for resources loaded by the renderer child.
    $start.EnvironmentVariables["AMAZING_WPT_ROOT"] = $WptRoot
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    if (-not $process.Start()) {
        return [pscustomobject]@{ classification = "crash"; detail = "process did not start" }
    }
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill()
        $process.WaitForExit()
        $process.StandardOutput.ReadToEnd() | Set-Content -LiteralPath "$LogStem.stdout.txt"
        $process.StandardError.ReadToEnd() | Set-Content -LiteralPath "$LogStem.stderr.txt"
        return [pscustomobject]@{ classification = "timeout"; detail = "exceeded ${TimeoutSeconds}s" }
    }
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $stdout | Set-Content -LiteralPath "$LogStem.stdout.txt"
    $stderr | Set-Content -LiteralPath "$LogStem.stderr.txt"
    # Driver protocol: XML document errors are distinct from unimplemented XML
    # features and ordinary renderer/JavaScript/process failures.
    if ($process.ExitCode -eq 2 -or $process.ExitCode -eq 3) {
        return [pscustomobject]@{
            classification = $(if ($process.ExitCode -eq 3) { "unsupported" } else { "error" })
            detail = $stderr.Trim()
        }
    }
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $OutputPath -PathType Leaf)) {
        return [pscustomobject]@{
            classification = "crash"
            detail = "renderer exit code $($process.ExitCode): $($stderr.Trim())"
        }
    }
    return [pscustomobject]@{ classification = "rendered"; detail = $stdout.Trim() }
}

function Read-Ppm {
    param([string]$Path)
    [byte[]]$bytes = [IO.File]::ReadAllBytes($Path)
    $tokens = New-Object Collections.Generic.List[string]
    $index = 0
    while ($tokens.Count -lt 4) {
        while ($index -lt $bytes.Length -and
            ($bytes[$index] -eq 9 -or $bytes[$index] -eq 10 -or $bytes[$index] -eq 13 -or $bytes[$index] -eq 32)) {
            $index++
        }
        if ($index -lt $bytes.Length -and $bytes[$index] -eq 35) {
            while ($index -lt $bytes.Length -and $bytes[$index] -ne 10) { $index++ }
            continue
        }
        $start = $index
        while ($index -lt $bytes.Length -and
            $bytes[$index] -ne 9 -and $bytes[$index] -ne 10 -and
            $bytes[$index] -ne 13 -and $bytes[$index] -ne 32) {
            $index++
        }
        if ($start -eq $index) { throw "Invalid PPM header in $Path" }
        $tokens.Add([Text.Encoding]::ASCII.GetString($bytes, $start, $index - $start))
    }
    if ($tokens[0] -ne "P6" -or $tokens[3] -ne "255") {
        throw "Only binary 8-bit P6 PPM is supported: $Path"
    }
    if ($index -ge $bytes.Length) { throw "PPM has no pixel separator: $Path" }
    $index++
    $width = [int]$tokens[1]
    $height = [int]$tokens[2]
    $length = $width * $height * 3
    if ($length -lt 0 -or $index + $length -ne $bytes.Length) {
        throw "PPM payload length does not match ${width}x${height}: $Path"
    }
    [byte[]]$pixels = New-Object byte[] $length
    [Array]::Copy($bytes, $index, $pixels, 0, $length)
    return [pscustomobject]@{ width = $width; height = $height; pixels = $pixels }
}

function Get-FuzzyLimits {
    param([string]$TestPath)
    $limits = [ordered]@{ max_difference = 0; total_pixels = 0 }
    $source = Get-Content -LiteralPath $TestPath -Raw
    $tagMatch = [regex]::Match($source, '(?is)<meta\b[^>]*\bname\s*=\s*["'']fuzzy["''][^>]*>')
    if (-not $tagMatch.Success) { return [pscustomobject]$limits }
    $contentMatch = [regex]::Match($tagMatch.Value, '(?is)\bcontent\s*=\s*["'']([^"'']+)["'']')
    if (-not $contentMatch.Success) { return [pscustomobject]$limits }
    $maxMatch = [regex]::Match($contentMatch.Groups[1].Value, 'maxDifference\s*=\s*\d+-(\d+)')
    $pixelMatch = [regex]::Match($contentMatch.Groups[1].Value, 'totalPixels\s*=\s*\d+-(\d+)')
    if ($maxMatch.Success) { $limits.max_difference = [int]$maxMatch.Groups[1].Value }
    if ($pixelMatch.Success) { $limits.total_pixels = [int]$pixelMatch.Groups[1].Value }
    return [pscustomobject]$limits
}

function Get-HtmlAttribute {
    param([string]$Tag, [string]$Name)
    $escapedName = [regex]::Escape($Name)
    $match = [regex]::Match(
        $Tag,
        '(?is)\b' + $escapedName + '\s*=\s*(?:"([^"]*)"|''([^'']*)''|([^\s>]+))'
    )
    if (-not $match.Success) { return $null }
    for ($group = 1; $group -le 3; $group++) {
        if ($match.Groups[$group].Success) { return $match.Groups[$group].Value }
    }
    return $null
}

function Get-ReftestLink {
    param([string]$TestPath)
    $source = Get-Content -LiteralPath $TestPath -Raw
    foreach ($tag in [regex]::Matches($source, '(?is)<link\b[^>]*>')) {
        $rel = Get-HtmlAttribute $tag.Value "rel"
        $href = Get-HtmlAttribute $tag.Value "href"
        if (-not $rel -or -not $href) { continue }
        $tokens = @($rel.ToLowerInvariant() -split '\s+' | Where-Object { $_ })
        $relation = if ($tokens -contains "match") {
            "match"
        } elseif ($tokens -contains "mismatch") {
            "mismatch"
        } else {
            continue
        }
        if ($href -match '^[A-Za-z][A-Za-z0-9+.-]*:' -or $href.StartsWith("//")) {
            return [pscustomobject]@{ supported = $false; detail = "remote reftest links are unsupported" }
        }
        $cleanHref = [Uri]::UnescapeDataString(($href -split '[?#]', 2)[0]) -replace '/', '\'
        $baseDirectory = if ($href.StartsWith('/')) { $WptRoot } else { Split-Path -Parent $TestPath }
        $referencePath = [IO.Path]::GetFullPath((Join-Path $baseDirectory $cleanHref.TrimStart('\')))
        $wptPrefix = $WptRoot.TrimEnd('\') + '\'
        if (-not $referencePath.StartsWith($wptPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            return [pscustomobject]@{ supported = $false; detail = "reference escapes the pinned WPT checkout" }
        }
        return [pscustomobject]@{
            supported = $true
            relation = $relation
            reference_path = $referencePath
            href = $href
        }
    }
    return [pscustomobject]@{ supported = $false; detail = "no upstream rel=match or rel=mismatch link" }
}

function Compare-Ppm {
    param(
        [string]$ActualPath,
        [string]$ReferencePath,
        [string]$DiffPath,
        [int]$AllowedDifference,
        [int]$AllowedPixels
    )
    $actual = Read-Ppm $ActualPath
    $reference = Read-Ppm $ReferencePath
    if ($actual.width -ne $reference.width -or $actual.height -ne $reference.height) {
        return [pscustomobject]@{
            matched = $false; differing_pixels = $actual.width * $actual.height
            max_difference = 255; detail = "image dimensions differ"
        }
    }
    [byte[]]$diff = New-Object byte[] $actual.pixels.Length
    $differentPixels = 0
    $maximumDifference = 0
    for ($pixel = 0; $pixel -lt $actual.pixels.Length; $pixel += 3) {
        $pixelDifferent = $false
        for ($channel = 0; $channel -lt 3; $channel++) {
            $difference = [Math]::Abs([int]$actual.pixels[$pixel + $channel] - [int]$reference.pixels[$pixel + $channel])
            if ($difference -gt 0) { $pixelDifferent = $true }
            if ($difference -gt $maximumDifference) { $maximumDifference = $difference }
        }
        if ($pixelDifferent) {
            $differentPixels++
            $diff[$pixel] = 255; $diff[$pixel + 1] = 0; $diff[$pixel + 2] = 255
        } else {
            $diff[$pixel] = 255; $diff[$pixel + 1] = 255; $diff[$pixel + 2] = 255
        }
    }
    $header = [Text.Encoding]::ASCII.GetBytes("P6`n$($actual.width) $($actual.height)`n255`n")
    [byte[]]$diffFile = New-Object byte[] ($header.Length + $diff.Length)
    [Array]::Copy($header, 0, $diffFile, 0, $header.Length)
    [Array]::Copy($diff, 0, $diffFile, $header.Length, $diff.Length)
    [IO.File]::WriteAllBytes($DiffPath, $diffFile)
    return [pscustomobject]@{
        matched = ($maximumDifference -le $AllowedDifference -and $differentPixels -le $AllowedPixels)
        differing_pixels = $differentPixels
        max_difference = $maximumDifference
        detail = "maxDifference=$maximumDifference/$AllowedDifference; totalPixels=$differentPixels/$AllowedPixels"
    }
}

$runName = "run-{0}-{1}" -f [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ"), $PID
$runDirectory = Join-Path $OutputDirectory $runName
New-Item -ItemType Directory -Force -Path $runDirectory | Out-Null
$results = New-Object Collections.Generic.List[object]
$unexpected = 0

foreach ($test in @($manifest.tests)) {
    $id = ([string]$test.id) -replace '[^A-Za-z0-9_.-]', '_'
    # Keep the reported upstream ID; shorten only its artifact directory so
    # long WPT paths remain usable under Windows MAX_PATH.
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $directoryId = [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($id))).Replace('-', '').Substring(0, 16) } finally { $sha.Dispose() }
    $testDirectory = Join-Path $runDirectory $directoryId
    New-Item -ItemType Directory -Force -Path $testDirectory | Out-Null
    $expected = if ($test.PSObject.Properties.Name -contains "expected") {
        [string]$test.expected
    } else {
        ""
    }
    if ($test.PSObject.Properties.Name -contains "unsupported_reason") {
        $classification = "unsupported"
        $result = [ordered]@{
            id = $id; classification = $classification; expected = $expected
            detail = [string]$test.unsupported_reason
        }
    } else {
        $testPath = Join-Path $WptRoot ([string]$test.test -replace '/', '\')
        if (-not (Test-Path -LiteralPath $testPath -PathType Leaf)) {
            $classification = "unsupported"
            $result = [ordered]@{
                id = $id; classification = $classification; expected = $expected
                detail = "test is absent from the pinned checkout"
            }
        } else {
            $reftest = Get-ReftestLink $testPath
            if (-not $reftest.supported -or -not (Test-Path -LiteralPath $reftest.reference_path -PathType Leaf)) {
                $classification = "unsupported"
                $detail = if (-not $reftest.supported) { $reftest.detail } else { "upstream reference is absent from the pinned checkout" }
                $result = [ordered]@{ id = $id; classification = $classification; expected = $expected; detail = $detail }
            } else {
                $actualPath = Join-Path $testDirectory "actual.ppm"
                $referenceOutputPath = Join-Path $testDirectory "reference.ppm"
                $diffPath = Join-Path $testDirectory "diff.ppm"
                $actualRun = Invoke-FixedRender $testPath $actualPath (Join-Path $testDirectory "actual")
                if ($actualRun.classification -ne "rendered") {
                    $classification = $actualRun.classification
                    $result = [ordered]@{ id = $id; classification = $classification; expected = $expected; detail = $actualRun.detail }
                } else {
                    $referenceRun = Invoke-FixedRender $reftest.reference_path $referenceOutputPath (Join-Path $testDirectory "reference")
                    if ($referenceRun.classification -ne "rendered") {
                        $classification = $referenceRun.classification
                        $result = [ordered]@{
                            id = $id; classification = $classification; expected = $expected
                            actual_sha256 = (Get-FileHash $actualPath -Algorithm SHA256).Hash.ToLowerInvariant()
                            detail = "reference: $($referenceRun.detail)"
                        }
                    } else {
                        $fuzzy = Get-FuzzyLimits $testPath
                        $comparison = Compare-Ppm $actualPath $referenceOutputPath $diffPath $fuzzy.max_difference $fuzzy.total_pixels
                        $passed = if ($reftest.relation -eq "mismatch") { -not $comparison.matched } else { $comparison.matched }
                        $classification = if ($passed) { "pass" } else { "mismatch" }
                        $result = [ordered]@{
                            id = $id
                            classification = $classification
                            expected = $expected
                            relation = $reftest.relation
                            reference_href = $reftest.href
                            differing_pixels = $comparison.differing_pixels
                            max_difference = $comparison.max_difference
                            allowed_max_difference = $fuzzy.max_difference
                            allowed_differing_pixels = $fuzzy.total_pixels
                            actual_sha256 = (Get-FileHash -LiteralPath $actualPath -Algorithm SHA256).Hash.ToLowerInvariant()
                            reference_sha256 = (Get-FileHash -LiteralPath $referenceOutputPath -Algorithm SHA256).Hash.ToLowerInvariant()
                            diff_sha256 = (Get-FileHash -LiteralPath $diffPath -Algorithm SHA256).Hash.ToLowerInvariant()
                            detail = $comparison.detail
                        }
                    }
                }
            }
        }
    }
    if ($expected -and $classification -ne $expected) { $unexpected++ }
    $results.Add([pscustomobject]$result)
    Write-Host ("{0,-10} {1}" -f $classification.ToUpperInvariant(), $id)
}

$counts = [ordered]@{ pass = 0; mismatch = 0; error = 0; timeout = 0; unsupported = 0; crash = 0 }
foreach ($result in $results) { $counts[$result.classification]++ }
$report = [ordered]@{
    schema = 1
    wpt_revision = $revision
    manifest = $ManifestPath
    viewport = [ordered]@{ width = $ViewportWidth; height = $ViewportHeight; device_scale = 1 }
    font_set = $FontSet
    run_directory = $runDirectory
    counts = $counts
    unexpected = $unexpected
    tests = $results
}
$reportPath = Join-Path $runDirectory "report.json"
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8
Write-Host "Report: $reportPath"
Write-Host ("Counts: pass={0} mismatch={1} error={2} timeout={3} unsupported={4} crash={5}; unexpected={6}" -f
    $counts.pass, $counts.mismatch, $counts.error, $counts.timeout, $counts.unsupported, $counts.crash, $unexpected)
if ($unexpected -ne 0) { exit 1 }
