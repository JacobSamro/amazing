# win-001: retain exactly one report per pinned LAYOUT-5 suite and repetition.
$ErrorActionPreference='Stop'
$root='C:\Users\server\src\wt\layout'
$target='C:\Users\server\target\layout'
$output=Join-Path $target 'layout5-collection'
New-Item -ItemType Directory -Force $output | Out-Null
$expected=Get-Content (Join-Path $target 'layout5-final-sources.json') -Raw | ConvertFrom-Json
foreach($entry in $expected.PSObject.Properties){
    if((Get-FileHash (Join-Path $root $entry.Name)).Hash.ToLowerInvariant() -ne $entry.Value){throw "Source drift: $($entry.Name)"}
}
$suites=@('flex-grid-broad','flex-grid','geometry','tables','floats','block-collapse','position','text-flow','contents','paint-compositing','images')
foreach($repeat in @(1,2)){
    foreach($suite in $suites){
        $folder=Join-Path $target "layout5-final-evidence\final-$suite-$repeat"
        $reports=@(Get-ChildItem $folder -Recurse -Filter report.json -ErrorAction SilentlyContinue)
        if($reports.Count -ne 1){throw "Expected one report in $folder; got $($reports.Count)"}
        Copy-Item $reports[0].FullName (Join-Path $output "final-$suite-$repeat.json")
    }
    foreach($suite in @('flex-grid-broad','flex-grid')){
        $folder=Join-Path $target "layout5-slice-$suite-$repeat"
        $reports=@(Get-ChildItem $folder -Recurse -Filter report.json -ErrorAction SilentlyContinue)
        if($reports.Count -ne 1){throw "Expected one report in $folder; got $($reports.Count)"}
        Copy-Item $reports[0].FullName (Join-Path $output "slice-$suite-$repeat.json")
    }
}
foreach($name in @('layout4-layout5-final-v2-build.json','layout5-workspace-nextest-1.log','layout5-workspace-nextest-2.log','layout5-workspace-doc-1.log','layout5-workspace-doc-2.log','layout5-final-sources.json')){
    $file=Join-Path $target $name
    if(!(Test-Path $file)){throw "Missing $file"}
    Copy-Item $file $output
}
@{
    renderer_sha256=(Get-FileHash (Join-Path $target 'layout4-layout5-final-v2.exe')).Hash.ToLowerInvariant()
    image_adapter_sha256=(Get-FileHash (Join-Path $target 'layout4-layout5-final-v2-images.exe')).Hash.ToLowerInvariant()
    source_manifest_sha256=(Get-FileHash (Join-Path $target 'layout5-final-sources.json')).Hash.ToLowerInvariant()
    collected_utc=[DateTime]::UtcNow.ToString('o')
} | ConvertTo-Json | Set-Content (Join-Path $output 'provenance.json')
"Collected $(@(Get-ChildItem $output -Filter '*.json').Count) JSON files in $output"
