param([string]$Process = "kiichat", [int]$MinY = 0)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$proc = Get-Process -Name $Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Output "no window"; exit 1 }
$root = [System.Windows.Automation.AutomationElement]::FromHandle($proc.MainWindowHandle)
$buttons = $root.FindAll(
    [System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button)))
$out = Join-Path $env:TEMP "kiichat-buttons.txt"

$lines = @()
for ($i = 0; $i -lt $buttons.Count; $i++) {
    $button = $buttons[$i]
    $rect = $button.Current.BoundingRectangle
    if ($rect.Y -lt $MinY) { continue }
    $name = $button.Current.Name
    $b64 = [System.Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($name))
    $lines += ("{0} b64={1} rect={2},{3} {4}x{5}" -f $name, $b64, [int]$rect.X, [int]$rect.Y, [int]$rect.Width, [int]$rect.Height)
}
$lines | Set-Content -Path $out -Encoding utf8
Write-Output "wrote $out ($($lines.Count) buttons)"
