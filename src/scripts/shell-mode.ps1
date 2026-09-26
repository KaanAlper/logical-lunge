# Logical Lunge kabuk modu: explorer.exe yerine Logical Lunge (native bar, tray, açılış programları).
#   shell-mode.ps1 on               yalnızca bu kullanıcı için (HKCU; yönetici gerekmez)
#   shell-mode.ps1 on -Machine      bu bilgisayardaki herkes için (HKLM; yönetici; lab imajı)
#   shell-mode.ps1 off [-Machine]   Explorer'a geri dön
#   shell-mode.ps1 status
# Değişiklik bir sonraki oturum açılışında geçerli olur. Kurtarma: oturum açarken Shift'e basılı tut ya da
# %LOCALAPPDATA%\logical-lunge\use-explorer dosyasını oluştur (Ctrl+Shift+Esc > Yeni görev > explorer.exe de olur).
param([ValidateSet('on', 'off', 'status')][string]$Mode = 'status', [switch]$Machine)
$ErrorActionPreference = 'Stop'

$helper = Join-Path $env:USERPROFILE '.glzr\logical-lunge\helper\ll-helper.exe'
$key = if ($Machine) { 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon' } else { 'HKCU:\Software\Microsoft\Windows NT\CurrentVersion\Winlogon' }
$zs = Join-Path $env:USERPROFILE '.glzr\zebar\settings.json'

# Zebar'daki (WebView2) bar kabuk modunda açılmaz: bar'ı native bar çizer, diğer widget'lar (sağ panel, overview...) kalır
function Set-ZebarBar([bool]$enabled) {
    if (-not (Test-Path $zs)) { return }
    $j = Get-Content $zs -Raw | ConvertFrom-Json
    $rest = @($j.startupConfigs | Where-Object { $_.widget -ne 'bar' })
    if ($enabled) { $rest = @([pscustomobject]@{ pack = 'logical-lunge'; widget = 'bar'; preset = 'default' }) + $rest }
    $j.startupConfigs = $rest
    # BOM'suz: Zebar settings.json'u BOM'la okuyamıyor
    [IO.File]::WriteAllText($zs, ($j | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
}

switch ($Mode) {
    'on' {
        if (-not (Test-Path $helper)) { throw "ll-helper.exe bulunamadı: $helper (önce Logical Lunge'u kur)" }
        # -Machine: kurulumu yapanın profilindeki helper; lab imajında bu yolun her hesapta aynı olması için
        # Logical Lunge'u ortak bir klasöre (ör. C:\LogicalLunge) kurup yolu buna göre değiştir.
        Set-ItemProperty $key -Name Shell -Value "`"$helper`" --shell"
        Set-ZebarBar $false
        Remove-Item (Join-Path $env:LOCALAPPDATA 'logical-lunge\use-explorer') -ErrorAction SilentlyContinue
        Write-Host 'Kabuk modu açık: bir sonraki oturumda Explorer yerine Logical Lunge başlar.'
        Write-Host 'Sorun olursa oturum açarken Shift basılı tut: Explorer açılır.'
    }
    'off' {
        if ($Machine) { Set-ItemProperty $key -Name Shell -Value 'explorer.exe' }
        else { Remove-ItemProperty $key -Name Shell -ErrorAction SilentlyContinue } # kullanıcı değeri yoksa makineninki (explorer.exe)
        Set-ZebarBar $true
        Write-Host 'Kabuk modu kapalı: bir sonraki oturumda Explorer başlar.'
    }
    'status' {
        foreach ($k in 'HKCU:\Software\Microsoft\Windows NT\CurrentVersion\Winlogon', 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon') {
            $v = (Get-ItemProperty $k -Name Shell -ErrorAction SilentlyContinue).Shell
            Write-Host ("{0}: {1}" -f $k, $(if ($v) { $v } else { '(yok)' }))
        }
    }
}
