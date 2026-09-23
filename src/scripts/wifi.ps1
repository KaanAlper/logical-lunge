# ii WifiDialog karşılığı (netsh wlan). Çıktı JSON.
#   wifi.ps1 list                     -> {"connected":"SSID","networks":[{"ssid","signal","secure","known"}]}
#   wifi.ps1 connect "<ssid>" ["<şifre>"] -> kayıtlı profil varsa bağlanır, yoksa şifreyle profil oluşturup bağlanır
#   wifi.ps1 disconnect
param([string]$Action = 'list', [string]$Ssid = '', [string]$Password = '')

[Console]::OutputEncoding = [Text.Encoding]::UTF8
function Out($o) { $o | ConvertTo-Json -Depth 4 -Compress; exit }

$profiles = @(netsh wlan show profiles | ForEach-Object { if ($_ -match '(Profile|Profil)[^:]*:\s*(.+)$') { $Matches[2].Trim() } })

switch ($Action) {
    'list' {
        # Taze tarama iste (sessizce), sonra listele
        $null = netsh wlan show networks mode=bssid
        $connected = ''
        netsh wlan show interfaces | ForEach-Object {
            if ($_ -match '^\s*SSID\s*:\s*(.+)$') { $connected = $Matches[1].Trim() }
        }
        $nets = @(); $cur = $null
        netsh wlan show networks mode=bssid | ForEach-Object {
            if ($_ -match '^SSID \d+\s*:\s*(.*)$') {
                if ($cur -and $cur.ssid) { $nets += $cur }
                $cur = [ordered]@{ ssid = $Matches[1].Trim(); signal = 0; secure = $true; known = $false }
            } elseif ($cur -and $_ -match '(Authentication|Kimlik doğrulama)\s*:\s*(.+)$') {
                $cur.secure = $Matches[2].Trim() -notmatch '^(Open|Açık)$'
            } elseif ($cur -and $_ -match '(Signal|Sinyal)\s*:\s*(\d+)%') {
                $cur.signal = [Math]::Max($cur.signal, [int]$Matches[2])
            }
        }
        if ($cur -and $cur.ssid) { $nets += $cur }
        foreach ($n in $nets) { $n.known = $profiles -contains $n.ssid }
        Out ([ordered]@{ connected = $connected; networks = @($nets | Sort-Object { - $_.signal }) })
    }
    'disconnect' {
        $null = netsh wlan disconnect
        Out @{ ok = $true }
    }
    'connect' {
        if (-not ($profiles -contains $Ssid)) {
            if (-not $Password) { Out @{ ok = $false; needPassword = $true } }
            # WPA2-Personal profili oluştur
            $esc = [Security.SecurityElement]::Escape($Ssid)
            $hex = -join ([Text.Encoding]::UTF8.GetBytes($Ssid) | ForEach-Object { $_.ToString('X2') })
            $key = [Security.SecurityElement]::Escape($Password)
            $xml = @"
<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>$esc</name>
  <SSIDConfig><SSID><hex>$hex</hex><name>$esc</name></SSID></SSIDConfig>
  <connectionType>ESS</connectionType>
  <connectionMode>auto</connectionMode>
  <MSM><security>
    <authEncryption><authentication>WPA2PSK</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption>
    <sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>$key</keyMaterial></sharedKey>
  </security></MSM>
</WLANProfile>
"@
            $file = Join-Path $env:TEMP 'll-wifi-profile.xml'
            [IO.File]::WriteAllText($file, $xml, (New-Object Text.UTF8Encoding $false))
            $null = netsh wlan add profile filename="$file" user=current
            Remove-Item $file -Force -ErrorAction SilentlyContinue
        }
        $null = netsh wlan connect name="$Ssid"
        # Bağlanmayı bekle (en fazla ~10 sn)
        for ($i = 0; $i -lt 20; $i++) {
            Start-Sleep -Milliseconds 500
            $state = ''; $now = ''
            netsh wlan show interfaces | ForEach-Object {
                if ($_ -match '^\s*(State|Durum)\s*:\s*(.+)$') { $state = $Matches[2].Trim() }
                if ($_ -match '^\s*SSID\s*:\s*(.+)$') { $now = $Matches[1].Trim() }
            }
            if ($now -eq $Ssid -and $state -match '^(connected|bağlı)$') { Out @{ ok = $true } }
        }
        Out @{ ok = $false; error = 'Bağlanılamadı (şifre yanlış olabilir)' }
    }
}
