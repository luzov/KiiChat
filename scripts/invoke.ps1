param([string]$NameB64 = "", [string]$Name = "")

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class Fg3 { [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); }
'@

if ($NameB64 -ne "") {
    $Name = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String($NameB64))
}

$proc = Get-Process -Name kiichat -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Output "no window"; exit 1 }
[Fg3]::SetForegroundWindow($proc.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 400

$root = [System.Windows.Automation.AutomationElement]::FromHandle($proc.MainWindowHandle)
$cond = New-Object System.Windows.Automation.AndCondition(
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button)),
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::NameProperty, $Name)))
$button = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
if (-not $button) {
    Write-Output ("button not found: " + $Name)
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        (New-Object System.Windows.Automation.PropertyCondition(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Button)))
    for ($i = 0; $i -lt $all.Count; $i++) { Write-Output ("  available: " + $all[$i].Current.Name) }
    exit 1
}

$pattern = $null
if (-not $button.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
    Write-Output "no InvokePattern"; exit 1
}
$pattern.Invoke()
Write-Output ("invoked: " + $Name)