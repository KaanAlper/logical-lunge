# BİR KEZ yönetici olarak çalıştırılır: CPU sıcaklığı için PawnIO sürücüsünü kurar ve sıcaklık okuyucuyu
# (tools\temps\lunge-temps.exe) oturum açılışında yönetici yetkisiyle başlatan LogicalLunge\Temps görevini oluşturur.
# Sonrasında hiçbir zaman izin sorulmaz.
$lhm = Join-Path (Split-Path $PSScriptRoot) 'tools\temps'
if (-not (Get-Service PawnIO -ErrorAction SilentlyContinue)) {
    Start-Process (Join-Path $lhm 'PawnIO_setup.exe') -ArgumentList '-install', '-silent' -Wait
}
$user = "$env:USERDOMAIN\$env:USERNAME"
$action = New-ScheduledTaskAction -Execute (Join-Path $lhm 'lunge-temps.exe') -WorkingDirectory $lhm
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 7
Register-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Temps' -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
Get-Process lunge-temps -ErrorAction SilentlyContinue | Stop-Process -Force
Start-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Temps'
"ok"
