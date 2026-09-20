param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [string]$WptRoot = ""
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$fixtures = Join-Path $PSScriptRoot 'wpt_harness_fixtures'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$utf8 = New-Object System.Text.UTF8Encoding($false)
$cases = @(
    @{name='pass'; status='pass'; counts=@(2,0,0,0,0)},
    @{name='fail'; status='fail'; counts=@(0,1,0,0,0)},
    @{name='subtest-timeout'; status='fail'; counts=@(0,0,1,0,0)},
    @{name='hang'; status='timeout'; counts=@(1,0,0,0,0)},
    @{name='error'; status='error'; counts=@(1,0,0,0,0)},
    @{name='escaping'; status='pass'; counts=@(1,0,0,0,0)},
    @{name='other-statuses'; status='fail'; counts=@(0,0,0,1,1)},
    @{name='harness-timeout'; status='timeout'; counts=@(0,0,0,0,0)},
    @{name='harness-error'; status='error'; counts=@(0,0,0,0,0)},
    @{name='ignored-script'; status='pass'; counts=@(1,0,0,0,0)},
    @{name='escape-root'; status='error'; counts=@(0,0,0,0,0)},
    @{name='missing-script'; status='error'; counts=@(0,0,0,0,0)},
    @{name='missing-harness'; status='error'; counts=@(0,0,0,0,0)},
    @{name='unsupported'; status='unsupported'; counts=@(0,0,0,0,0)}
)
$keys = @('pass','fail','timeout','notrun','error')
$hashes = @()
foreach ($round in 1..2) {
    $records = @()
    foreach ($case in $cases) {
        $line = & $Executable wpt-harness --scaffold $fixtures ($case.name + '.html') 800 600 1000
        $exit = $LASTEXITCODE
        $result = $line | ConvertFrom-Json
        if ($result.suite -ne 'scaffolding-only' -or $null -ne $result.wpt_revision) { throw 'Scaffold incorrectly labeled as WPT' }
        if ($result.status -ne $case.status) { throw "$($case.name): expected $($case.status), got $line" }
        if (($result.status -eq 'pass') -ne ($exit -eq 0)) { throw "$($case.name): incorrect exit code $exit" }
        foreach ($index in 0..4) {
            if ($result.counts.($keys[$index]) -ne $case.counts[$index]) { throw "$($case.name): incorrect counts: $line" }
        }
        if ($result.subtests.Count -ne ($case.counts | Measure-Object -Sum).Sum) { throw 'Lost subtest result' }
        if ($result.file_counts.($case.status) -ne 1) { throw 'Incorrect file count' }
        $records += $line
    }
    $path = Join-Path $OutputDirectory "scaffold-$round.jsonl"
    [IO.File]::WriteAllText($path, ($records -join "`n") + "`n", $utf8)
    $hashes += (Get-FileHash -Algorithm SHA256 $path).Hash
}
if ($hashes[0] -ne $hashes[1]) { throw 'Scaffold classifications changed on repeat' }
$summary = [ordered]@{suite='scaffolding-only'; cases=$cases.Count; repeats=2; sha256=$hashes[0]; wpt_credit=0}
if ($WptRoot) {
    # Explicit small selection, not a claim about all dom/ tests.
    $selection = @('dom/nodes/CharacterData-data.html', 'dom/nodes/CharacterData-appendData.html', 'dom/nodes/CharacterData-deleteData.html')
    $wptHashes = @()
    foreach ($round in 1..2) {
        $records = @()
        foreach ($test in $selection) {
            $line = & $Executable wpt-harness $WptRoot $test 800 600 10000
            $result = $line | ConvertFrom-Json
            if ($result.suite -ne 'wpt' -or $result.wpt_revision -ne 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293') { throw 'Missing upstream provenance' }
            $records += $line
        }
        $path = Join-Path $OutputDirectory "dom-smoke-$round.jsonl"
        [IO.File]::WriteAllText($path, ($records -join "`n") + "`n", $utf8)
        $wptHashes += (Get-FileHash -Algorithm SHA256 $path).Hash
    }
    if ($wptHashes[0] -ne $wptHashes[1]) { throw 'DOM selection classifications changed on repeat' }
    $summary['dom_selection'] = $selection
    $summary['dom_sha256'] = $wptHashes[0]
}
$text = $summary | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'summary.json'), $text + "`n", $utf8)
$text
