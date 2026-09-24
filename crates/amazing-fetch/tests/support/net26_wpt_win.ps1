# Pinned installed-runner diagnostic for NET-26 on win-001.
param([string[]]$Tests = @(
  'fetch/api/headers/headers-basic.any.js',
  'fetch/api/headers/headers-casing.any.js',
  'fetch/api/headers/headers-combine.any.js',
  'fetch/api/headers/headers-no-cors.any.js',
  'fetch/api/headers/headers-normalize.any.js',
  'fetch/api/headers/header-values-normalize.any.js',
  'fetch/api/response/response-init-001.any.js',
  'fetch/api/response/response-consume-empty.any.js',
  'fetch/api/basic/request-headers.any.js',
  'fetch/api/headers/header-setcookie.any.js',
  'fetch/api/redirect/redirect-mode.any.js',
  'fetch/api/cors/cors-basic.any.js',
  'fetch/api/cors/cors-preflight.any.js',
  'fetch/api/credentials/cookies.any.js',
  'fetch/api/abort/general.any.js',
  'xhr/event-load.any.js',
  'xhr/event-progress.any.js',
  'xhr/event-timeout.any.js',
  'xhr/event-readystatechange-loaded.any.js',
  'xhr/responsetype.any.js',
  'xhr/getresponseheader.any.js',
  'xhr/getallresponseheaders.htm',
  'xhr/getallresponseheaders-cookies.htm',
  'xhr/status-basic.htm',
  'xhr/send-redirect.htm',
  'xhr/getresponseheader-chunked-trailer.htm',
  'xhr/setrequestheader-before-open.htm',
  'xhr/setrequestheader-after-send.htm',
  'xhr/setrequestheader-open-setrequestheader.htm',
  'xhr/setrequestheader-bogus-name.htm',
  'xhr/setrequestheader-bogus-value.htm',
  'xhr/setrequestheader-allow-empty-value.htm',
  'xhr/setrequestheader-allow-whitespace-in-value.htm',
  'xhr/setrequestheader-case-insensitive.htm',
  'xhr/setrequestheader-combining.window.js',
  'xhr/setrequestheader-content-type.htm',
  'xhr/setrequestheader-header-allowed.htm',
  'xhr/setrequestheader-header-forbidden.htm'
))
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-Location 'C:\Users\server\src\wt\net'
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\net'
$env:AMAZING_STORAGE_DIR = Join-Path $env:CARGO_TARGET_DIR ('net26-storage-' + $PID)
$root = 'C:\Users\server\target\net26-wpt'
$revision = (git -C $root rev-parse HEAD).Trim()
if ($revision -ne 'c2a2fce69e5c2509d9dc28440ec8b42532e5c293') { throw "Wrong WPT revision: $revision" }
cargo build -q -p amazing-net --example wpt_shim --locked
if ($LASTEXITCODE -ne 0) { throw 'shim build failed' }
cargo build -q -p amazing --locked
if ($LASTEXITCODE -ne 0) { throw 'browser build failed' }
$shimExe = Join-Path $env:CARGO_TARGET_DIR 'debug\examples\wpt_shim.exe'
$browserExe = Join-Path $env:CARGO_TARGET_DIR 'debug\amazing.exe'
$out = Join-Path $env:CARGO_TARGET_DIR 'net26-shim-url.txt'
$err = Join-Path $env:CARGO_TARGET_DIR 'net26-shim-stderr.txt'
Remove-Item $out,$err -ErrorAction SilentlyContinue
$shim = Start-Process -FilePath $shimExe -ArgumentList '--serve' -PassThru -NoNewWindow -RedirectStandardOutput $out -RedirectStandardError $err
try {
  $url = ''
  for ($i = 0; $i -lt 100; $i++) {
    if (Test-Path $out) { $url = (Get-Content $out -First 1 -ErrorAction SilentlyContinue) }
    if ($url) { break }
    Start-Sleep -Milliseconds 50
  }
  if (-not $url) { throw 'shim did not print URL' }
  $env:AMAZING_WPT_SHIM_ADDR = $url -replace '^http://',''
  "SHIM $env:AMAZING_WPT_SHIM_ADDR"
  foreach ($test in $Tests) {
    "TEST $test"
    & $browserExe wpt-harness $root $test 800 600 10000
    "EXIT $LASTEXITCODE"
  }
  'NET26-WPT-DONE'
} finally {
  Remove-Item Env:AMAZING_WPT_SHIM_ADDR -ErrorAction SilentlyContinue
  Stop-Process -Id $shim.Id -Force -ErrorAction SilentlyContinue
  Remove-Item $out,$err -ErrorAction SilentlyContinue
  Remove-Item $env:AMAZING_STORAGE_DIR -Recurse -Force -ErrorAction SilentlyContinue
}
