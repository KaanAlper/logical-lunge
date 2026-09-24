# BİR KEZ yönetici olarak çalıştırılır: Ethernet'i açıp kapatan iki zamanlanmış görev oluşturur.
# Sonrasında sağ paneldeki Ethernet kutucuğu bunları izin sormadan tetikler (schtasks /run).
$script = Join-Path $PSScriptRoot 'eth.ps1'
$user = "$env:USERDOMAIN\$env:USERNAME"
foreach ($pair in @(@('Ethernet-On', 'enable'), @('Ethernet-Off', 'disable'))) {
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' `
        -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$script`" $($pair[1])"
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 1)
    Register-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName $pair[0] -Action $action -Principal $principal -Settings $settings -Force | Out-Null
}
"ok"
