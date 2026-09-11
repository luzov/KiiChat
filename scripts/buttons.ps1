param([string]$Process = "kiichat", [int]$MinY = 0, [switch]$All)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

# Every branch writes this file, so a caller can never read a previous run's
# dump as if it were current.
$out = Join-Path $env:TEMP "kiichat-buttons.txt"


$proc = Get-Process -Name $Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) {
    # Overwrite the report even on failure: a caller that reads the file
    # without checking this script's own output would otherwise re-read the
    # previous run's dump and believe it is current.
    "no window for process '$Process'" | Set-Content -Path $out -Encoding utf8
    Write-Output "no window for process '$Process' (report cleared)"
    exit 1
}
$root = [System.Windows.Automation.AutomationElement]::FromHandle($proc.MainWindowHandle)
# -All lists every element, not just buttons: the app's labels and previews
# are the only way to read text back without vision.
$filter = if ($All) {
    [System.Windows.Automation.Condition]::TrueCondition
} else {
    New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button)
}
$buttons = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $filter)

$lines = @()
for ($i = 0; $i -lt $buttons.Count; $i++) {
    $button = $buttons[$i]
    $rect = $button.Current.BoundingRectangle
    if ($rect.Y -lt $MinY) { continue }
    $name = $button.Current.Name
    $b64 = [System.Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($name))
    $type = $button.Current.ControlType.ProgrammaticName -replace "ControlType\.", ""
    $lines += ("[{0}] {1} b64={2} rect={3},{4} {5}x{6}" -f $type, $name, $b64, [int]$rect.X, [int]$rect.Y, [int]$rect.Width, [int]$rect.Height)
}
$lines | Set-Content -Path $out -Encoding utf8
Write-Output "wrote $out ($($lines.Count) buttons)"
