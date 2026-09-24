# Logical Lunge masaüstünü (GlazeWM, Zebar, ll-helper) temiz biçimde yeniden başlatır: takılan ya da kapanan bir
# parçayı kullanıcı komut satırı bilmeden toparlayabilsin. Oturum menüsündeki "Masaüstünü yenile" ve Başlat
# menüsündeki "Logical Lunge'u yeniden başlat" kısayolu bunu gizli olarak çalıştırır (ll-helper --ps-bg).
# Açık pencereler kapanmaz; yeniden başlayan GlazeWM onları yeniden yerleştirir. Geçişi duvar kağıdı örtüsü kapatır.
$ErrorActionPreference = 'Continue'
$LL = Join-Path $env:USERPROFILE '.glzr\logical-lunge'
$helper = Join-Path $LL 'helper\ll-helper.exe'
$state = Join-Path $env:LOCALAPPDATA 'logical-lunge'
$marker = Join-Path $state 'maintenance'
New-Item -ItemType Directory -Force $state | Out-Null
# Nöbetçiler (ll-helper) bu sırada hiçbir şeyi kendileri yeniden başlatmasın
Set-Content $marker (Get-Date -Format o)

# Örtü: kopyadan (GlazeWM kapanırken "ll-helper" adlı her süreç kapatılıyor)
try {
    $copy = Join-Path $env:TEMP 'll-restart-splash.exe'
    Copy-Item $helper $copy -Force
    $env:LL_SPLASH_WAIT_RESTART = '1'
    Start-Process $copy -ArgumentList '--splash' | Out-Null
    $env:LL_SPLASH_WAIT_RESTART = $null
}
catch {}

try {
    $gw = Get-Process glazewm -ErrorAction SilentlyContinue | Sort-Object StartTime | Select-Object -First 1
    if ($gw) {
        # Nazik çıkış: GlazeWM diğer workspace'lerin gizli pencerelerini geri getirir, Zebar'ı ve helper'ı kapatır
        try { & $gw.Path command wm-exit 2>$null | Out-Null } catch {}
        if (-not $gw.WaitForExit(6000)) {
            # Takılmış: önce helper (GlazeWM'i yeniden başlatmasın), sonra GlazeWM
            Get-Process ll-helper -ErrorAction SilentlyContinue | Stop-Process -Force
            Stop-Process -Id $gw.Id -Force -ErrorAction SilentlyContinue
            Start-Sleep -Milliseconds 500
        }
    }
    # Kalanlar (GlazeWM'siz kalmış ya da takılmış parçalar)
    Get-Process zebar, ll-helper -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 300
    # Görünmez kalmış pencereleri geri getir
    & $helper --uncloak-orphans | Out-Null
}
finally { Remove-Item $marker -Force -ErrorAction SilentlyContinue }

# Masaüstü görevinden (GlazeWM; o da Zebar'ı ve helper'ı açar). Helper ayrıca başlatılır: GlazeWM açılamazsa onu
# bekleyip yeniden deneyen o (GlazeWM'in açtığı kopya zaten çalışan varken kendiliğinden kapanır).
try { Start-ScheduledTask -TaskPath '\LL\' -TaskName 'GlazeWM' -ErrorAction Stop }
catch { Start-Process (Join-Path $LL 'bin\glazewm.exe') -WorkingDirectory $env:USERPROFILE }
Start-Sleep -Seconds 3
Start-Process $helper -WorkingDirectory (Split-Path $helper)
