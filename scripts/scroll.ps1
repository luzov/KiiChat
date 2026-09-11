param([int]$X, [int]$Y, [int]$Clicks = 3, [string]$Process = "kiichat")

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class Wheel2 {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern uint SendInput(uint count, INPUT[] inputs, int size);

  [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT {
    public int dx; public int dy; public uint mouseData; public uint dwFlags;
    public uint time; public IntPtr dwExtraInfo;
  }
  [StructLayout(LayoutKind.Sequential)] public struct INPUT {
    public uint type; public MOUSEINPUT mi;
  }

  public static void Wheel(int notches) {
    // MOUSEEVENTF_WHEEL = 0x0800; INPUT_MOUSE = 0
    INPUT[] inputs = new INPUT[1];
    inputs[0].type = 0;
    inputs[0].mi.dwFlags = 0x0800;
    inputs[0].mi.mouseData = unchecked((uint)(-120 * notches));
    inputs[0].mi.dx = 0; inputs[0].mi.dy = 0;
    inputs[0].mi.time = 0; inputs[0].mi.dwExtraInfo = IntPtr.Zero;
    SendInput(1, inputs, Marshal.SizeOf(typeof(INPUT)));
  }
}
'@

# A DPI-unaware caller aims at half the intended point on a scaled desktop.
[Wheel2]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null

$proc = Get-Process -Name $Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Output "no window"; exit 1 }
[Wheel2]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 400

[Wheel2]::SetCursorPos($X, $Y) | Out-Null
Start-Sleep -Milliseconds 250
for ($i = 0; $i -lt $Clicks; $i++) {
    [Wheel2]::Wheel(1)
    Start-Sleep -Milliseconds 60
}
Write-Output "sent $Clicks wheel notches down at $X,$Y via SendInput"