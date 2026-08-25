param(
    [string]$WptRoot = "",
    [string]$Revision = "",
    [string[]]$Categories = @(
        "css-syntax",
        "css-cascade",
        "selectors",
        "css-values",
        "css-color",
        "css-fonts",
        "css-variables"
    ),
    [string]$CategoryCsv = "",
    [int]$PerCategory = 12,
    [string]$InventoryPath = "",
    [string]$ManifestPath = "",
    [string]$IncludePathRegex = "",
    [string]$ExcludePathRegex = "",
    [string]$BlockedContentRegex = "",
    [switch]$SkipFetch
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
if ($CategoryCsv) {
    $Categories = @($CategoryCsv.Split(',') | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}
if (-not $WptRoot) { $WptRoot = Join-Path $repositoryRoot "target\wpt" }
if (-not $InventoryPath) { $InventoryPath = Join-Path $repositoryRoot "target\wpt-cascade-inventory.json" }
if (-not $ManifestPath) { $ManifestPath = Join-Path $repositoryRoot "target\wpt-cascade-baseline.json" }
if ($PerCategory -lt 1 -or $PerCategory -gt 1000) {
    throw "PerCategory must be between 1 and 1000."
}

$WptRoot = [IO.Path]::GetFullPath($WptRoot)
$InventoryPath = [IO.Path]::GetFullPath($InventoryPath)
$ManifestPath = [IO.Path]::GetFullPath($ManifestPath)
if (-not $Revision) {
    $smokeManifest = Get-Content -LiteralPath (Join-Path $repositoryRoot "tests\wpt-render\smoke.json") -Raw | ConvertFrom-Json
    $Revision = [string]$smokeManifest.revision
}
if ($Revision -notmatch '^[0-9a-f]{40}$') {
    throw "Revision must be a full 40-character Git object id."
}

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program exited with code $LASTEXITCODE." }
}

if (-not $SkipFetch) {
    if (-not (Test-Path -LiteralPath (Join-Path $WptRoot ".git") -PathType Container)) {
        New-Item -ItemType Directory -Force -Path $WptRoot | Out-Null
        Invoke-Checked "git" @("-C", $WptRoot, "init")
        Invoke-Checked "git" @("-C", $WptRoot, "remote", "add", "origin", "https://github.com/web-platform-tests/wpt.git")
        Invoke-Checked "git" @("-C", $WptRoot, "sparse-checkout", "init", "--cone")
    }
    $sparseArguments = @("-C", $WptRoot, "sparse-checkout", "set", "--cone")
    $sparseArguments += @($Categories | ForEach-Object { "css/$_" })
    Invoke-Checked "git" $sparseArguments
    Invoke-Checked "git" @("-C", $WptRoot, "fetch", "--depth=1", "origin", $Revision)
    Invoke-Checked "git" @("-C", $WptRoot, "checkout", "--detach", "--force", "FETCH_HEAD")
} elseif (-not (Test-Path -LiteralPath (Join-Path $WptRoot ".git") -PathType Container)) {
    throw "WPT checkout not found: $WptRoot"
}
$actualRevision = (& git -C $WptRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $actualRevision -ne $Revision) {
    throw "WPT checkout is $actualRevision; expected $Revision."
}

function Get-HtmlAttribute {
    param([string]$Tag, [string]$Name)
    $match = [regex]::Match(
        $Tag,
        '(?is)\b' + [regex]::Escape($Name) + '\s*=\s*(?:"([^"]*)"|''([^'']*)''|([^\s>]+))'
    )
    if (-not $match.Success) { return $null }
    for ($group = 1; $group -le 3; $group++) {
        if ($match.Groups[$group].Success) { return $match.Groups[$group].Value }
    }
    return $null
}

function Get-ReftestLink {
    param([string]$TestPath, [string]$Source)
    foreach ($tag in [regex]::Matches($Source, '(?is)<link\b[^>]*>')) {
        $rel = Get-HtmlAttribute $tag.Value "rel"
        $href = Get-HtmlAttribute $tag.Value "href"
        if (-not $rel -or -not $href) { continue }
        $tokens = @($rel.ToLowerInvariant() -split '\s+' | Where-Object { $_ })
        if ($tokens -notcontains "match" -and $tokens -notcontains "mismatch") { continue }
        if ($href -match '^[A-Za-z][A-Za-z0-9+.-]*:' -or $href.StartsWith("//")) {
            return [pscustomobject]@{ supported = $false; reason = "remote-reference" }
        }
        $cleanHref = ($href -split '[?#]', 2)[0] -replace '/', '\'
        $referencePath = [IO.Path]::GetFullPath((Join-Path (Split-Path -Parent $TestPath) $cleanHref))
        $wptPrefix = $WptRoot.TrimEnd('\') + '\'
        if (-not $referencePath.StartsWith($wptPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            return [pscustomobject]@{ supported = $false; reason = "escaping-reference" }
        }
        return [pscustomobject]@{ supported = $true; reference_path = $referencePath }
    }
    return [pscustomobject]@{ supported = $false; reason = "not-reftest" }
}

function Get-StableHash {
    param([string]$Value)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes))).Replace("-", "").ToLowerInvariant()
    } finally {
        $algorithm.Dispose()
    }
}

function Add-Reason {
    param([Collections.Specialized.OrderedDictionary]$Reasons, [string]$Reason)
    if (-not $Reasons.Contains($Reason)) { $Reasons[$Reason] = 0 }
    $Reasons[$Reason]++
}

$documentExtensions = @(".html", ".htm", ".xht", ".xhtml")
$categoryRows = New-Object Collections.Generic.List[object]
$selectedTests = New-Object Collections.Generic.List[object]

foreach ($category in $Categories) {
    if ($category -notmatch '^[A-Za-z0-9-]+$') { throw "Invalid category: $category" }
    $categoryRoot = Join-Path (Join-Path $WptRoot "css") $category
    if (-not (Test-Path -LiteralPath $categoryRoot -PathType Container)) {
        throw "Sparse checkout does not contain css/$category"
    }
    $files = @(Get-ChildItem -LiteralPath $categoryRoot -Recurse -File | Where-Object {
        $documentExtensions -contains $_.Extension.ToLowerInvariant()
    })
    $reftestCount = 0
    $testharnessCount = 0
    $eligible = New-Object Collections.Generic.List[object]
    $reasons = [ordered]@{}

    foreach ($file in $files) {
        $relative = $file.FullName.Substring($WptRoot.TrimEnd('\').Length + 1).Replace('\', '/')
        if ($IncludePathRegex -and $relative -notmatch $IncludePathRegex) {
            Add-Reason $reasons "outside-included-path"
            continue
        }
        if ($ExcludePathRegex -and $relative -match $ExcludePathRegex) {
            Add-Reason $reasons "excluded-path"
            continue
        }
        $source = Get-Content -LiteralPath $file.FullName -Raw
        if ($source -match '(?is)testharness\.js') { $testharnessCount++ }
        $link = Get-ReftestLink $file.FullName $source
        if (-not $link.supported) {
            Add-Reason $reasons $link.reason
            continue
        }
        $reftestCount++
        if (@(".xht", ".xhtml") -contains $file.Extension.ToLowerInvariant()) {
            Add-Reason $reasons "xml-parser-mode"
            continue
        }
        if (-not (Test-Path -LiteralPath $link.reference_path -PathType Leaf)) {
            Add-Reason $reasons "missing-reference"
            continue
        }
        if (@(".xht", ".xhtml") -contains [IO.Path]::GetExtension($link.reference_path).ToLowerInvariant()) {
            Add-Reason $reasons "xml-reference-mode"
            continue
        }
        $referenceSource = Get-Content -LiteralPath $link.reference_path -Raw
        $combined = $source + "`n" + $referenceSource
        $blockedReason = if ($BlockedContentRegex -and $combined -match $BlockedContentRegex) {
            "excluded-content"
        } elseif ($combined -match '(?is)<script\b') {
            "scripted"
        } elseif ($combined -match '(?is)<meta\b[^>]*\bname\s*=\s*["'']flags["'']') {
            "flagged-environment"
        } elseif ($combined -match '(?is)<(?:img|object|iframe|video|audio|canvas|svg)\b') {
            "embedded-resource"
        } elseif ($combined -match '(?is)url\s*\(|@font-face|@import\b') {
            "css-resource"
        } elseif ($combined -match '(?is)<link\b[^>]*\brel\s*=\s*["'']stylesheet["'']') {
            "external-stylesheet"
        } else {
            $null
        }
        if ($blockedReason) {
            Add-Reason $reasons $blockedReason
            continue
        }
        $eligible.Add([pscustomobject]@{
            path = $relative
            hash = Get-StableHash $relative
        })
    }

    $selected = @($eligible | Sort-Object hash, path | Select-Object -First $PerCategory)
    foreach ($candidate in $selected) {
        $id = ($candidate.path -replace '^css/', '') -replace '[^A-Za-z0-9_.-]', '__'
        $selectedTests.Add([pscustomobject]@{ id = $id; test = $candidate.path })
    }
    $categoryRows.Add([pscustomobject]@{
        category = $category
        document_files = $files.Count
        testharness_files = $testharnessCount
        reftests = $reftestCount
        eligible_static_self_contained = $eligible.Count
        selected = $selected.Count
        excluded = $reasons
        selected_paths = @($selected | ForEach-Object { $_.path })
    })
}

$inventory = [ordered]@{
    schema = 1
    wpt_revision = $Revision
    selection_policy = "For each category, SHA-256 rank the normalized path of every HTML-parser-compatible, local-reference, script-free, flag-free, resource-free reftest; select the lowest hashes."
    exclude_path_regex = $ExcludePathRegex
    blocked_content_regex = $BlockedContentRegex
    per_category = $PerCategory
    include_path_regex = $IncludePathRegex
    categories = $categoryRows
}
$manifest = [ordered]@{
    schema = 1
    revision = $Revision
    selection = [ordered]@{ kind = "sha256-path-stratified"; per_category = $PerCategory }
    sparse_paths = @($Categories | ForEach-Object { "css/$_" })
    tests = $selectedTests
}

foreach ($path in @($InventoryPath, $ManifestPath)) {
    $parent = Split-Path -Parent $path
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
}
$inventory | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $InventoryPath -Encoding UTF8
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $ManifestPath -Encoding UTF8
Write-Host "Inventory: $InventoryPath"
Write-Host "Manifest:  $ManifestPath"
Write-Host "Selected:  $($selectedTests.Count)"
$categoryRows | Format-Table category, document_files, testharness_files, reftests, eligible_static_self_contained, selected
