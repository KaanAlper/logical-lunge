# ii BluetoothDialog karşılığı: eşleşmiş Bluetooth cihazlarını listeler. Çıktı JSON.
#   bt.ps1 list -> {"adapter":true,"devices":[{"name","connected","kind"}]}
[Console]::OutputEncoding = [Text.Encoding]::UTF8

$radio = $null
try {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    $asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
    [Windows.Devices.Radios.Radio, Windows.System.Devices, ContentType = WindowsRuntime] | Out-Null
    $radios = $asTask.MakeGenericMethod([System.Collections.Generic.IReadOnlyList[Windows.Devices.Radios.Radio]]).Invoke($null, @([Windows.Devices.Radios.Radio]::GetRadiosAsync())).Result
    $radio = $radios | Where-Object { "$($_.Kind)" -eq 'Bluetooth' } | Select-Object -First 1
} catch {}

$skip = '(?i)enumerator|adapter|radio|microsoft|generic|service|protocol|transport|rfcomm|avrcp|hizmet|ağ geçidi|erişim'
# PnP "OK" durumu yalnızca eşleşmiş/yüklü demek; gerçek bağlantı DEVPKEY_Bluetooth_IsConnected ({83DA6326-...} 15)
$connKey = '{83DA6326-97A6-4088-9453-A1923F573B29} 15'
$all = @(Get-PnpDevice -Class Bluetooth -ErrorAction SilentlyContinue |
    Where-Object { $_.FriendlyName -and $_.FriendlyName -notmatch $skip -and $_.InstanceId -match '^BTH' })
$connected = @{}
foreach ($d in $all) {
    $v = (Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName $connKey -ErrorAction SilentlyContinue).Data
    if ($v -eq $true) { $connected[$d.FriendlyName] = $true }
}
$devices = @($all |
    Sort-Object FriendlyName -Unique |
    ForEach-Object {
        $n = $_.FriendlyName
        $kind = if ($n -match '(?i)buds|air|headphone|headset|kulaklık|wh-|wf-') { 'headphones' }
                elseif ($n -match '(?i)mouse|fare') { 'mouse' }
                elseif ($n -match '(?i)keyboard|klavye') { 'keyboard' }
                elseif ($n -match '(?i)phone|galaxy|iphone|pixel|redmi|xiaomi') { 'smartphone' }
                elseif ($n -match '(?i)controller|gamepad|xbox|dualsense') { 'sports_esports' }
                else { 'bluetooth' }
        [ordered]@{ name = $n; connected = [bool]$connected[$n]; kind = $kind }
    })

[ordered]@{ adapter = [bool]$radio; on = ($radio -and "$($radio.State)" -eq 'On'); devices = $devices } | ConvertTo-Json -Depth 3 -Compress
