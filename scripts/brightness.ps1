# Parlaklık: dahili ekranda WMI, harici monitörde DDC/CI (NirSoft ControlMyMonitor, VCP 10).
# Kullanım: brightness.ps1 get [monitör] | brightness.ps1 set <0-100> [monitör]
# monitör: "Primary" (varsayılan) ya da "\\.\DISPLAY2\Monitor0" gibi.
param([string]$Action = 'get', [string]$Arg1 = '', [string]$Arg2 = '')

$cmm = Join-Path $PSScriptRoot '..\tools\ControlMyMonitor.exe'
$wmi = Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue

if ($Action -eq 'get') {
    $mon = if ($Arg1) { $Arg1 } else { 'Primary' }
    if ($wmi) { ($wmi | Select-Object -First 1).CurrentBrightness; exit }
    $p = Start-Process $cmm -ArgumentList '/GetValue', $mon, '10' -Wait -PassThru -WindowStyle Hidden
    $p.ExitCode
    exit
}

$value = [Math]::Max(0, [Math]::Min(100, [int]$Arg1))
$mon = if ($Arg2) { $Arg2 } else { 'Primary' }
if ($wmi) {
    Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods |
        Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{ Timeout = 1; Brightness = [byte]$value } | Out-Null
} else {
    Start-Process $cmm -ArgumentList '/SetValue', $mon, '10', $value -Wait -WindowStyle Hidden
}
