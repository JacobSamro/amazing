param(
    [string]$Address = "file:///C:/Users/server/src/amazing/examples/gpui-navigation.html",
    [string]$OutputPath = "target\gpui-address-navigation.png"
)

$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class AmazingAddressSmokeNative {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr handle, out Rect rect);
    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
"@

$process = Get-Process -Name "amazing-shell" -ErrorAction Stop |
    Where-Object { $_.MainWindowHandle -ne 0 } |
    Select-Object -First 1
if ($null -eq $process) {
    throw "Amazing GPUI shell has no visible top-level window"
}
(New-Object -ComObject WScript.Shell).AppActivate($process.Id) | Out-Null
Start-Sleep -Milliseconds 500

$rect = New-Object AmazingAddressSmokeNative+Rect
if (-not [AmazingAddressSmokeNative]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) {
    throw "GetWindowRect failed"
}

# The address field is the flexible center region of the fixed 44px toolbar.
$addressX = $rect.Left + [Math]::Min(430, ($rect.Right - $rect.Left) / 2)
$addressY = $rect.Top + 62
[AmazingAddressSmokeNative]::SetCursorPos($addressX, $addressY) | Out-Null
[AmazingAddressSmokeNative]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[AmazingAddressSmokeNative]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 300

$shell = New-Object -ComObject WScript.Shell
$shell.SendKeys($Address)
$shell.SendKeys("{ENTER}")
Start-Sleep -Seconds 3

& "$PSScriptRoot\capture-gpui-shell.ps1" -OutputPath $OutputPath
