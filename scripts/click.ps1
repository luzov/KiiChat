param([int]$X, [int]$Y, [string]$Process = "kiichat")

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class Clicky {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, IntPtr extra);
  [DllImport("user32.dll")] public static extern IntPtr GetCursorPos(out POINT p);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
}
'@

# Without this, a 200%-scaled desktop sends the click to half the intended place.
[Clicky]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

$proc = Get-Process -Name $Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Output "no window"; exit 1 }
[Clicky]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 500

[Clicky]::SetCursorPos($X, $Y) | Out-Null
Start-Sleep -Milliseconds 250
$p = New-Object Clicky+POINT
[Clicky]::GetCursorPos([ref]$p) | Out-Null
[Clicky]::mouse_event(0x02, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 120
[Clicky]::mouse_event(0x04, 0, 0, 0, [IntPtr]::Zero)
Write-Output "clicked $X,$Y (cursor at $($p.X),$($p.Y))"