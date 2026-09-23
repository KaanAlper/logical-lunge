# ii overview'ün komut / web eylemleri. Hiçbir zaman konsol penceresi açmaz; sonucu JSON olarak
# yazar, overview bunu ii tarzı bildirim (toast) olarak gösterir.
#   run.ps1 run  "<metin>"  -> Win+R gibi: program / dosya / klasör / adres / "program argüman";
#                              konsol komutları gizli çalışır, çıktısı bildirimde görünür
#   run.ps1 term "<komut>"  -> ($ öneki) doğrudan gizli cmd'de çalıştırır
#   run.ps1 url  "<adres>"  -> varsayılan tarayıcıda açar
param([string]$Mode = 'run', [string]$Text = '')

[Console]::OutputEncoding = [Text.Encoding]::UTF8

function Result([string]$kind, [string]$title, [string]$body, [string]$icon) {
    [ordered]@{ kind = $kind; title = $title; body = $body; icon = $icon } | ConvertTo-Json -Compress
    exit
}

# PE başlığından alt sistem: 2 = pencereli (GUI), 3 = konsol
function Test-ConsoleExe([string]$path) {
    try {
        $fs = [IO.File]::OpenRead($path)
        try {
            $br = New-Object IO.BinaryReader $fs
            $fs.Position = 0x3C; $pe = $br.ReadInt32()
            $fs.Position = $pe + 0x5C
            return $br.ReadUInt16() -eq 3
        } finally { $fs.Dispose() }
    } catch { return $false }
}

function Invoke-Hidden([string]$cmd) {
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = "$env:SystemRoot\System32\cmd.exe"
    $psi.Arguments = "/d /s /c `"$cmd`""
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    # Konsol programları OEM kod sayfasıyla yazar (Türkçe: 857)
    $oem = [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.OEMCodePage)
    $psi.StandardOutputEncoding = $oem
    $psi.StandardErrorEncoding = $oem
    $p = [Diagnostics.Process]::Start($psi)
    $outTask = $p.StandardOutput.ReadToEndAsync()
    $errTask = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit(8000)) {
        Result 'info' 'Arka planda çalışıyor' $cmd 'hourglass_top'
    }
    $out = $outTask.Result.Trim(); $err = $errTask.Result.Trim()
    if ($p.ExitCode -eq 9009 -or $err -match 'tanınmıyor|is not recognized') {
        $first = ($cmd -split '\s+', 2)[0]
        Result 'error' 'Komut bulunamadı' "'$first' diye bir program ya da komut yok." 'search_off'
    }
    $text = if ($p.ExitCode -ne 0 -and $err) { $err } elseif ($out) { $out } else { $err }
    $lines = @($text -split "`r?`n" | Where-Object { $_.Trim() })
    $body = ($lines | Select-Object -First 8) -join "`n"
    if ($lines.Count -gt 8) { $body += "`n… (+$($lines.Count - 8) satır)" }
    if ($p.ExitCode -ne 0) { Result 'error' "Hata ($($p.ExitCode))" ($(if ($body) { $body } else { $cmd })) 'error' }
    Result 'ok' $cmd ($(if ($body) { $body } else { 'Tamamlandı' })) 'terminal'
}

$Text = $Text.Trim()
if (-not $Text) { exit }

switch ($Mode) {
    'url' {
        try { Start-Process $Text -ErrorAction Stop } catch { Result 'error' 'Açılamadı' $Text 'link_off' }
        exit
    }
    'term' { Invoke-Hidden $Text }
}

# ---- run: Win+R mantığı ----
$expanded = [Environment]::ExpandEnvironmentVariables($Text)
$parts = $expanded -split '\s+', 2
$exeArg = if ($parts.Count -gt 1) { $parts[1] } else { $null }

# PATH'te bulunan program: konsolsa gizli çalıştır, pencereliyse normal aç
$cmdInfo = Get-Command $parts[0] -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if ($cmdInfo) {
    if (Test-ConsoleExe $cmdInfo.Source) { Invoke-Hidden $Text }
    if ($exeArg) { Start-Process -FilePath $cmdInfo.Source -ArgumentList $exeArg } else { Start-Process -FilePath $cmdInfo.Source }
    exit
}

# Dosya, klasör, adres, App Paths (ör. "chrome"), shell: yolları
try { Start-Process -FilePath $expanded -ErrorAction Stop; exit } catch {}
if ($exeArg) {
    try { Start-Process -FilePath $parts[0] -ArgumentList $exeArg -ErrorAction Stop; exit } catch {}
}

# cmd yerleşik komutları (dir, echo, set...) gizli; o da yoksa "bulunamadı" bildirimi
Invoke-Hidden $Text
