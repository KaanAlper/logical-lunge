# BİR KEZ yönetici olarak çalıştırılır: CPU sıcaklığı için PawnIO sürücüsünü kurar ve sıcaklık okuyucuyu
# (tools\lhm\ll-temps.exe) oturum açılışında yönetici yetkisiyle başlatan LL\Temps görevini oluşturur.
# Sonrasında hiçbir zaman izin sorulmaz.
$lhm = Join-Path (Split-Path $PSScriptRoot) 'tools\lhm'
if (-not (Get-Service PawnIO -ErrorAction SilentlyContinue)) {
    Start-Process (Join-Path $lhm 'PawnIO_setup.exe') -ArgumentList '-install', '-silent' -Wait
}
$user = "$env:USERDOMAIN\$env:USERNAME"
$action = New-ScheduledTaskAction -Execute (Join-Path $lhm 'll-temps.exe') -WorkingDirectory $lhm
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 7
Register-ScheduledTask -TaskPath '\LL\' -TaskName 'Temps' -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
Get-Process ll-temps -ErrorAction SilentlyContinue | Stop-Process -Force
Start-ScheduledTask -TaskPath '\LL\' -TaskName 'Temps'
"ok"
