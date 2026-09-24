# Logical Lunge masaüstünü temiz biçimde yeniden başlatır. Asıl işi çekirdek yapar (lunge.exe --restart-desktop:
# parçaları kapatır, gizli kalan pencereleri geri getirir, kökü yeniden açar); oturum menüsü ve Başlat menüsündeki
# kısayol onu doğrudan çağırır. Bu betik elle / eski kısayollardan çağrılabilsin diye duruyor.
$core = Join-Path (Split-Path $PSScriptRoot) 'lunge.exe'
Start-Process $core -ArgumentList '--restart-desktop' -WorkingDirectory $env:USERPROFILE
