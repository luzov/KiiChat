param([string]$Name, [string]$Value, [string]$Process = "kiichat")

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$proc = Get-Process -Name $Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Output "no window"; exit 1 }
$root = [System.Windows.Automation.AutomationElement]::FromHandle($proc.MainWindowHandle)

$edits = $root.FindAll(
    [System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit)))
$target = $null
for ($i = 0; $i -lt $edits.Count; $i++) {
    if ($edits[$i].Current.Name -like "*$Name*") { $target = $edits[$i]; break }
}
if (-not $target) {
    Write-Output "no edit matching '$Name'"
    for ($i = 0; $i -lt $edits.Count; $i++) { Write-Output ("  edit: " + $edits[$i].Current.Name) }
    exit 1
}

$pattern = $null
if (-not $target.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) {
    Write-Output "no ValuePattern on '$Name'"
    exit 1
}
$pattern.SetValue($Value)
Write-Output ("set '$Name' = '$Value' (value now: {0})" -f $pattern.Current.Value)