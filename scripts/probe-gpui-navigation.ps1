param(
    [string]$Address = "google.com",
    [string]$OutputPath = "target\gpui-navigation-probe.png",
    [string]$ExecutablePath = "target\release\amazing-shell.exe",
    [int]$WaitSeconds = 12,
    [switch]$ReuseRunningShell,
    [switch]$KeepWidgets
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$executable = [System.IO.Path]::GetFullPath((Join-Path $root $ExecutablePath))
$output = [System.IO.Path]::GetFullPath((Join-Path $root $OutputPath))

$process = if ($ReuseRunningShell) {
    Get-Process amazing-shell -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne 0 } |
        Select-Object -First 1
} else {
    $null
}
if ($null -eq $process) {
    $process = Start-Process $executable -PassThru
    Start-Sleep -Seconds 4
}

Set-Clipboard -Value $Address
$shell = New-Object -ComObject WScript.Shell
if (-not $shell.AppActivate($process.Id)) {
    throw "Could not activate Amazing GPUI process $($process.Id)"
}
$shell.SendKeys("^l")
Start-Sleep -Milliseconds 300
$shell.SendKeys("^v")
Start-Sleep -Milliseconds 300
$shell.SendKeys("{ENTER}")
Start-Sleep -Seconds $WaitSeconds
$shell.SendKeys("{ESC}")
Start-Sleep -Milliseconds 500
if (-not $KeepWidgets) {
    Get-Process Widgets -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
}
if (-not $shell.AppActivate($process.Id)) {
    throw "Could not reactivate Amazing GPUI process $($process.Id) before capture"
}
Start-Sleep -Seconds 1

& (Join-Path $PSScriptRoot "capture-gpui-shell.ps1") -OutputPath $output
Write-Output "ADDRESS=$Address"
Write-Output "PROCESS_ID=$($process.Id)"
Write-Output "SCREENSHOT=$output"
