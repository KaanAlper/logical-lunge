# Wi-Fi / Bluetooth radyoları (WinRT Windows.Devices.Radios) — ii quick toggles için.
# Kullanım: radios.ps1 get            -> {"wifi":"On","bluetooth":"Off"}
#           radios.ps1 set wifi On|Off
param([string]$Action = 'get', [string]$Kind = '', [string]$State = '')

Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
})[0]
function Await($op, [Type]$t) { $asTask.MakeGenericMethod($t).Invoke($null, @($op)).Result }

[Windows.Devices.Radios.Radio, Windows.System.Devices, ContentType = WindowsRuntime] | Out-Null
$null = Await ([Windows.Devices.Radios.Radio]::RequestAccessAsync()) ([Windows.Devices.Radios.RadioAccessStatus])
$radios = Await ([Windows.Devices.Radios.Radio]::GetRadiosAsync()) ([System.Collections.Generic.IReadOnlyList[Windows.Devices.Radios.Radio]])

$map = @{ wifi = 'WiFi'; bluetooth = 'Bluetooth' }

if ($Action -eq 'get') {
    $o = [ordered]@{ wifi = $null; bluetooth = $null }
    foreach ($r in $radios) {
        if ($r.Kind -eq 'WiFi') { $o.wifi = "$($r.State)" }
        if ($r.Kind -eq 'Bluetooth') { $o.bluetooth = "$($r.State)" }
    }
    $o | ConvertTo-Json -Compress
    exit
}

$radio = $radios | Where-Object { "$($_.Kind)" -eq $map[$Kind] } | Select-Object -First 1
if ($radio) {
    $null = Await ($radio.SetStateAsync([Windows.Devices.Radios.RadioState]::$State)) ([Windows.Devices.Radios.RadioAccessStatus])
}
