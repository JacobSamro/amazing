param([string]$Executable, [string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$cases = @(
 @('version'), @('unknown-command'),
 @('parse','examples/sample.html'), @('parse-fragment','table','examples/sample.html'), @('parse-fragment'),
 @('style','examples/sample.html'), @('layout','examples/sample.html','320'),
 @('render','examples/sample.html','target/diagnostics/ia-parity.bmp','320'),
 @('render-wpt','examples/sample.html','target/diagnostics/ia-parity.ppm','320','240'),
 @('fetch','examples/sample.html'), @('fetch-parse'),
 @('run','target/diagnostics/ia-parity.js'), @('domrun','examples/sample.html','target/diagnostics/ia-parity.js'),
 @('audio-info','examples/phase9-audio.wav'), @('play-audio'), @('webrtc-loopback'), @('turn-probe'), @('rtp-jitter-probe'),
 @('opus-probe'), @('video-frame-probe'), @('vp8-decode-probe'), @('h264-roundtrip-probe'),
 @('h264-rtp-probe'), @('secure-media-loopback'), @('secure-media-processes'),
 @('secure-video-stream'), @('__secure-media-peer'), @('gpu-info'), @('gpu-present-smoke'),
 @('sdp-roundtrip','examples/phase9-chromium-offer.sdp'), @('sdp-browser-answer'), @('browser-peer','invalid'),
 @('wpt-harness'), @('wpt-harness-worker'),
 @('wpt-harness','--scaffold','crates/amazing/src/cmd/wpt_harness_fixtures','pass.html','800','600','1000'),
 @('gui','target/diagnostics/ia-missing.html')
)
New-Item -ItemType Directory -Force target/diagnostics | Out-Null
[IO.File]::WriteAllText((Join-Path (Get-Location) 'target/diagnostics/ia-parity.js'), 'console.log(6 * 7); 42;')
$records = @()
foreach ($a in $cases) {
 $info = New-Object Diagnostics.ProcessStartInfo
 $info.FileName = $Executable
 $info.Arguments = (($a | ForEach-Object { '"' + $_ + '"' }) -join ' ')
 $info.UseShellExecute = $false
 $info.RedirectStandardOutput = $true
 $info.RedirectStandardError = $true
 $info.RedirectStandardInput = $true
 $p = New-Object Diagnostics.Process
 $p.StartInfo = $info
 [void]$p.Start()
 $p.StandardInput.Close()
 $stdout = $p.StandardOutput.ReadToEndAsync()
 $stderr = $p.StandardError.ReadToEndAsync()
 $limit = if ($a[0] -eq 'gui') { 1500 } else { 30000 }
 $timeout = -not $p.WaitForExit($limit)
 if ($timeout) { & taskkill /PID $p.Id /T /F | Out-Null; $p.WaitForExit() }
 $out = $stdout.Result; $err = $stderr.Result
 $artifact = $null
 if ($a[0] -in @('render','render-wpt') -and (Test-Path $a[2])) { $artifact = (Get-FileHash $a[2] -Algorithm SHA256).Hash }
 $records += [ordered]@{args=$a; exit=$p.ExitCode; timeout=$timeout; stdout=$out; stderr=$err; artifact=$artifact}
 $p.Dispose()
}
$utf8 = New-Object Text.UTF8Encoding($false)
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'cli.json'), (ConvertTo-Json -InputObject $records -Depth 6), $utf8)
Write-Output "Captured $($records.Count) command cases"
