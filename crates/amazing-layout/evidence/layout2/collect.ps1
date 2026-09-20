# Read-only evidence collection on win-001, after both repeated selections finish.
param(
    [string]$EvidenceRoot = 'C:\Users\server\target\layout\layout2-evidence',
    [string]$BaselinePhase = 'baseline-verified',
    [string]$FinalPhase = 'final-verified',
    [string]$BaselineExecutable = 'C:\Users\server\target\layout\layout2-baseline-133.exe',
    [string]$FinalExecutable = 'C:\Users\server\target\layout\layout2-final-verified.exe',
    [string]$SourceManifestPath = 'C:\Users\server\target\layout\layout2-expected-sources.json',
    [Parameter(Mandatory=$true)][string]$OutputPath
)
$ErrorActionPreference='Stop'
$repositoryRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..\..'))
$expected=Get-Content $SourceManifestPath -Raw | ConvertFrom-Json
foreach ($entry in $expected.PSObject.Properties) {
    $path=Join-Path $repositoryRoot $entry.Name
    if ((Get-FileHash $path).Hash.ToLowerInvariant() -ne $entry.Value) {
        throw "Source changed since gate: $($entry.Name)"
    }
}
$reports=@{}
foreach ($phase in @($BaselinePhase,$FinalPhase)) {
    Get-ChildItem (Join-Path $EvidenceRoot $phase) -Recurse -Filter report.json | ForEach-Object {
        $key="$phase/$($_.Directory.Parent.Name)"
        if ($reports.ContainsKey($key)) { throw "Ambiguous repeated run directory: $key" }
        $reports[$key]=Get-Content $_.FullName -Raw | ConvertFrom-Json
    }
}
$sources=@{}
Get-ChildItem (Join-Path $repositoryRoot 'crates\amazing-layout\src') -Filter *.rs | ForEach-Object {
    if (-not $_.Name.StartsWith('._')) { $sources[$_.Name]=(Get-FileHash $_.FullName).Hash.ToLowerInvariant() }
}
@{
    reports=$reports
    sources=$sources
    build_source_manifest_sha256=(Get-FileHash $SourceManifestPath).Hash.ToLowerInvariant()
    baseline_phase=$BaselinePhase
    final_phase=$FinalPhase
    baseline_executable=$BaselineExecutable
    final_executable=$FinalExecutable
    baseline_executable_sha256=(Get-FileHash $BaselineExecutable).Hash.ToLowerInvariant()
    final_executable_sha256=(Get-FileHash $FinalExecutable).Hash.ToLowerInvariant()
    lockfile_sha256=(Get-FileHash (Join-Path $repositoryRoot 'Cargo.lock')).Hash.ToLowerInvariant()
} | ConvertTo-Json -Depth 100 -Compress | Set-Content $OutputPath
