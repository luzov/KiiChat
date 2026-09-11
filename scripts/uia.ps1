param([string]$Process = "kiichat", [int]$Depth = 6, [int]$MaxLines = 120)

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$proc = Get-Process -Name $Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Output "no window for '$Process'"; exit 1 }

$root = [System.Windows.Automation.AutomationElement]::FromHandle($proc.MainWindowHandle)
if (-not $root) { Write-Output "no automation root"; exit 1 }

$walker = [System.Windows.Automation.TreeWalker]::RawViewWalker
$lines = New-Object System.Collections.Generic.List[string]
$script:count = 0

function Walk($element, [int]$level) {
    if (-not $element -or $level -gt $Depth -or $script:count -gt $MaxLines) { return }
    $script:count++
    $pad = " " * ($level * 2)
    try {
        $name = $element.Current.Name
        $type = $element.Current.ControlType.ProgrammaticName -replace "ControlType\.", ""
        $class = $element.Current.ClassName
        $rect = $element.Current.BoundingRectangle
        $geom = "{0},{1} {2}x{3}" -f [int]$rect.X, [int]$rect.Y, [int]$rect.Width, [int]$rect.Height
    } catch {
        $name = "<stale>"; $type = "?"; $class = ""; $geom = ""
    }
    $lines.Add(("{0}{1} [{2}] class='{3}' name='{4}' {5}" -f $pad, $type, "", $class, $name, $geom))
    $child = $walker.GetFirstChild($element)
    while ($child) {
        Walk $child ($level + 1)
        $child = $walker.GetNextSibling($child)
    }
}

Walk $root 0
Write-Output ("elements: {0}" -f $script:count)
$lines | ForEach-Object { Write-Output $_ }