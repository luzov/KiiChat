param([int]$FromX, [int]$FromY, [int]$ToX, [int]$ToY, [string]$Process = "kiichat")

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class Drag {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, IntPtr extra);
}
'@

# A DPI-unaware caller aims at half the intended point on a scaled desktop.
[Drag]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

$proc = Get-Process -Name $Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Output "no window"; exit 1 }
[Drag]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 500

[Drag]::SetCursorPos($FromX, $FromY) | Out-Null
Start-Sleep -Milliseconds 250
[Drag]::mouse_event(0x02, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 150

# Move in steps: a single jump can be read as a click by the platform.
$steps = 8
for ($i = 1; $i -le $steps; $i++) {
    $x = [int]($FromX + ($ToX - $FromX) * $i / $steps)
    $y = [int]($FromY + ($ToY - $FromY) * $i / $steps)
    [Drag]::SetCursorPos($x, $y) | Out-Null
    Start-Sleep -Milliseconds 60
}
[Drag]::mouse_event(0x04, 0, 0, 0, [IntPtr]::Zero)
Write-Output "dragged $FromX,$FromY -> $ToX,$ToY"