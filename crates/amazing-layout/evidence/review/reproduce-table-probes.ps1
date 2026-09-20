# Windows-only diagnostic copies, not modified upstream WPT or conformance results.
param(
    [string]$WptRoot = 'C:\Users\server\target\layout\wpt',
    [string]$Baseline = 'C:\Users\server\target\layout\baseline.exe',
    [string]$Final = 'C:\Users\server\target\layout\layout1-final.exe',
    [string]$Output = 'C:\Users\server\target\layout\evidence\review-probes'
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $Output | Out-Null
$original = Get-Content (Join-Path $WptRoot 'css\CSS2\floats\float-table-align-left-quirk.html') -Raw
$probes = @{
    'table-wide-probe' = $original.Replace('width:0', 'width:100px')
    'table-no-css-float-probe' = $original.Replace('float:left', 'float:none')
    'table-align-as-css-probe' = $original.Replace('align="left"', 'style="float:left"')
}
$renderers = @{ 'baseline' = $Baseline; 'layout1-final' = $Final }
foreach ($probe in $probes.Keys) {
    $inputFile = Join-Path $Output "$probe.html"
    [IO.File]::WriteAllText($inputFile, $probes[$probe], [Text.UTF8Encoding]::new($false))
    foreach ($renderer in $renderers.Keys) {
        $image = Join-Path $Output "$renderer-$probe.ppm"
        & ($renderers[$renderer]) render-wpt $inputFile $image 800 600
        if ($LASTEXITCODE -ne 0) { throw "render failed: $renderer/$probe" }
        $hash = (Get-FileHash $image).Hash.ToLowerInvariant()
        $reference = 'c7ba3123aba39ee1dec40df17c8b0e6240e5b3390d5695b15b407e7d92da7d0a'
        if ($renderer -eq 'baseline' -or $probe -ne 'table-wide-probe') {
            if ($hash -ne $reference) { throw "Inert-pass probe changed: $renderer/$probe $hash" }
        } elseif ($hash -eq $reference) { throw 'Final renderer did not respond to wider float space' }
        Write-Output "PASS $renderer/$probe $hash"
    }
}
# Compare hashes to regression-review.json. The baseline's three probes and the
# final no-CSS-float / align-as-CSS probes all equal the retained reference hash:
# c7ba3123aba39ee1dec40df17c8b0e6240e5b3390d5695b15b407e7d92da7d0a.
