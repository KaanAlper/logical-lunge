# Sağ panel için tek seferde durum: uyanık tut açık mı, uygulama teması açık mı.
#   status.ps1            -> {"awake":true,"light":false}
#   status.ps1 stop-awake -> keep-awake.ps1 süreçlerini durdurur
param([string]$Action = '')

$awakeProcs = Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" |
    Where-Object { $_.CommandLine -like '*keep-awake.ps1*' }

if ($Action -eq 'stop-awake') {
    $awakeProcs | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    exit
}

$light = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' -ErrorAction SilentlyContinue).AppsUseLightTheme
[ordered]@{ awake = [bool]$awakeProcs; light = ($light -eq 1) } | ConvertTo-Json -Compress
