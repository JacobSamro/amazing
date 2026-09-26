$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Write-Output 'LAYOUT-18 loss repair candidate on 19d856cc'
Get-FileHash Cargo.lock, crates\amazing-layout\src\flex.rs, crates\amazing-layout\src\grid.rs, crates\amazing-layout\src\grid_repeat.rs, crates\amazing-layout\src\position.rs
& cargo build -p amazing --release --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$exe = Join-Path $env:CARGO_TARGET_DIR 'layout18-repaired.exe'
Copy-Item (Join-Path $env:CARGO_TARGET_DIR 'release\amazing.exe') $exe
Get-FileHash $exe
& powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\run-fast.ps1" -ManifestPath "$PSScriptRoot\loss-manifest.json" -ExecutablePath $exe -OutputDirectory (Join-Path $env:CARGO_TARGET_DIR 'layout18-loss-repaired')
exit $LASTEXITCODE
