<#
.SYNOPSIS
Captures the Amazing GPUI window without requiring it to be foreground or
unobscured.

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\capture-gpui-shell.ps1

.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\capture-gpui-shell.ps1 `
  -ProcessId 10308 -OutputPath target\diagnostics\google-current.png

.NOTES
Run this in the interactive RDP user session. From SSH, invoke it through an
interactive-only scheduled task so MainWindowHandle refers to the RDP desktop.
#>
param(
    [string]$OutputPath = "target\gpui-shell-screen.png",
    [int]$ProcessId = 0,
    [string]$ProcessName = "amazing-shell",
    [switch]$Activate,
    [int]$SettleMilliseconds = 500
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class AmazingCaptureNative {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr handle, out Rect rect);
    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr handle, IntPtr targetDc, uint flags);
}
"@

$process = if ($ProcessId -gt 0) {
    Get-Process -Id $ProcessId -ErrorAction Stop
} else {
    Get-Process -Name $ProcessName -ErrorAction Stop |
        Where-Object { $_.MainWindowHandle -ne 0 } |
        Select-Object -First 1
}
if ($null -eq $process) {
    throw "Amazing GPUI shell has no visible top-level window"
}
if ($Activate) {
    (New-Object -ComObject WScript.Shell).AppActivate($process.Id) | Out-Null
    Start-Sleep -Milliseconds $SettleMilliseconds
}

$rect = New-Object AmazingCaptureNative+Rect
if (-not [AmazingCaptureNative]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) {
    throw "GetWindowRect failed"
}
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) {
    throw "Amazing GPUI shell has invalid window bounds"
}

$fullOutputPath = [System.IO.Path]::GetFullPath($OutputPath)
$directory = [System.IO.Path]::GetDirectoryName($fullOutputPath)
[System.IO.Directory]::CreateDirectory($directory) | Out-Null
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $targetDc = $graphics.GetHdc()
    try {
        # PW_RENDERFULLCONTENT captures the target even when another window
        # obscures it, which keeps remote iterations deterministic.
        if (-not [AmazingCaptureNative]::PrintWindow($process.MainWindowHandle, $targetDc, 2)) {
            throw "PrintWindow failed for process $($process.Id)"
        }
    } finally {
        $graphics.ReleaseHdc($targetDc)
    }
    $bitmap.Save($fullOutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}

Write-Output "WIDTH=$width"
Write-Output "HEIGHT=$height"
Write-Output "PROCESS_ID=$($process.Id)"
Write-Output "SCREENSHOT=$fullOutputPath"
