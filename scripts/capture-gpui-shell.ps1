param(
    [string]$OutputPath = "target\gpui-shell-screen.png"
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
}
"@

$process = Get-Process -Name "amazing-shell" -ErrorAction Stop |
    Where-Object { $_.MainWindowHandle -ne 0 } |
    Select-Object -First 1
if ($null -eq $process) {
    throw "Amazing GPUI shell has no visible top-level window"
}
(New-Object -ComObject WScript.Shell).AppActivate($process.Id) | Out-Null
Start-Sleep -Seconds 2

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
    $graphics.CopyFromScreen(
        (New-Object System.Drawing.Point $rect.Left, $rect.Top),
        [System.Drawing.Point]::Empty,
        (New-Object System.Drawing.Size $width, $height)
    )
    $bitmap.Save($fullOutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}

Write-Output "WIDTH=$width"
Write-Output "HEIGHT=$height"
Write-Output "SCREENSHOT=$fullOutputPath"
