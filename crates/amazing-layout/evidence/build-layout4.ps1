# win-001 only. Force first-party recompilation, verify sources and pin the result.
param(
    [Parameter(Mandatory=$true)][string]$Phase,
    [Parameter(Mandatory=$true)][string]$SourceManifestPath,
    [switch]$FullWorkspace,
    [switch]$BuildImages,
    [string]$TargetDirectory = 'C:\Users\server\target\layout'
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$repositoryRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
Set-Location $repositoryRoot
$env:CARGO_TARGET_DIR=$TargetDirectory
$env:CARGO_INCREMENTAL='0'
$target=$env:CARGO_TARGET_DIR
$expected=Get-Content $SourceManifestPath -Raw | ConvertFrom-Json
function Verify-Source {
    foreach ($entry in $expected.PSObject.Properties) {
        if ((Get-FileHash (Join-Path $repositoryRoot $entry.Name)).Hash.ToLowerInvariant() -ne $entry.Value) {
            throw "Source drift: $($entry.Name)"
        }
    }
}
Verify-Source
foreach ($profile in @('debug','release')) {
    $fingerprints=Join-Path $target "$profile\.fingerprint"
    $deps=Join-Path $target "$profile\deps"
    if (Test-Path $fingerprints) {
        Get-ChildItem $fingerprints -Directory -Filter 'amazing-*' | Remove-Item -Recurse -Force
        if (@(Get-ChildItem $fingerprints -Directory -Filter 'amazing-*').Count) { throw 'Fingerprint cleanup failed' }
    }
    if (Test-Path $deps) {
        Get-ChildItem $deps -Filter '*amazing*' | Remove-Item -Force
        if (@(Get-ChildItem $deps -Filter '*amazing*').Count) { throw 'Artifact cleanup failed' }
    }
}
$started=[DateTime]::UtcNow.ToString('o')
$logs=@()
function Invoke-Cargo([string]$Arguments,[string]$LogName) {
    $format=if($Arguments.StartsWith("build ")){" --message-format=json"}else{""}
    $log=Join-Path $target $LogName
    # cmd redirection avoids Windows PowerShell's native-stderr ErrorRecord behavior.
    & cmd /c "cargo $Arguments --locked$format > `"$log`" 2>&1"
    $code=$LASTEXITCODE
    $code | Set-Content "$log.exit"
    if ($code -ne 0) {
        Get-Content $log -Tail 40
        throw "cargo $Arguments failed ($code)"
    }
    Verify-Source
}
$rendererLog="layout4-$Phase-renderer.log"
Invoke-Cargo 'build -p amazing' $rendererLog
$logs += $rendererLog
$unitLog="layout4-$Phase-tests.log"
Invoke-Cargo 'nextest run -p amazing-layout' $unitLog
$docLog="layout4-$Phase-doc.log"
Invoke-Cargo 'test -p amazing-layout --doc' $docLog
$logs += $docLog
$logs += $unitLog
if ($FullWorkspace) {
    $workspaceLog="layout4-$Phase-workspace.log"
    Invoke-Cargo 'nextest run --workspace' $workspaceLog
    $workspaceDocLog="layout4-$Phase-workspace-doc.log"
    Invoke-Cargo 'test --workspace --doc' $workspaceDocLog
    $logs += $workspaceDocLog
    $logs += $workspaceLog
}
$imageExecutable=$null
if ($BuildImages) {
    $imageLog="layout4-$Phase-images-build.log"
    Invoke-Cargo 'build --manifest-path crates\amazing-shell-gpui\Cargo.toml -p amazing-image --example render_reftest' $imageLog
    $logs += $imageLog
    $imageExecutable=Join-Path $target "layout4-$Phase-images.exe"
    Copy-Item (Join-Path $target 'debug\examples\render_reftest.exe') $imageExecutable
}
$artifacts=@()
foreach ($log in $logs) {
    foreach ($line in Get-Content (Join-Path $target $log)) {
        if (-not $line.StartsWith('{')) { continue }
        try { $item=$line | ConvertFrom-Json } catch { continue }
        if ($item.reason -eq 'compiler-artifact' -and $item.package_id.Contains('/crates/amazing')) {
            $artifacts += @{
                package=$item.package_id
                name=$item.target.name
                kind=$item.target.kind
                test_profile=$item.profile.test
                fresh=$item.fresh
                log=$log
            }
        }
    }
}
foreach ($required in @('amazing_layout','amazing')) {
    if (-not @($artifacts | Where-Object { $_.name -eq $required -and -not $_.fresh -and -not $_.test_profile }).Count) {
        throw "No fresh non-test compilation recorded for $required"
    }
}
$executable=Join-Path $target "layout4-$Phase.exe"
Copy-Item (Join-Path $target 'debug\amazing.exe') $executable
$logHashes=@{}
foreach ($log in $logs) { $logHashes[$log]=(Get-FileHash (Join-Path $target $log)).Hash.ToLowerInvariant() }
@{
    phase=$Phase
    started_utc=$started
    completed_utc=[DateTime]::UtcNow.ToString('o')
    first_party_fingerprints_and_artifacts_removed=$true
    incremental_compilation=$false
    source_manifest_sha256=(Get-FileHash $SourceManifestPath).Hash.ToLowerInvariant()
    verified_source_files=@($expected.PSObject.Properties).Count
    sources_verified_before_build_and_after_every_command=$true
    image_executable=$imageExecutable
    image_executable_sha256=$(if($imageExecutable){(Get-FileHash $imageExecutable).Hash.ToLowerInvariant()}else{$null})
    executable=$executable
    executable_sha256=(Get-FileHash $executable).Hash.ToLowerInvariant()
    compiler_artifacts=$artifacts
    logs=$logHashes
} | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $target "layout4-$Phase-build.json")
"Fresh build pinned: $executable"
Get-FileHash $executable
