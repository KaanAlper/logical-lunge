# Logical Lunge - first-install wizard
#   irm https://raw.githubusercontent.com/KaanAlper/logical-lunge/main/install.ps1 | iex
# Asks for the focus color, the interface language and the clock, downloads the latest release (with a progress bar)
# and runs installer\setup.ps1 elevated (one UAC prompt) while showing its steps. On an error or Ctrl+C the installer
# puts everything back the way it was. Nothing is left behind in %TEMP%.
# Options (set before running):  $env:LL_NO_TERMINAL = 1   skip WezTerm + fish
#                                 $env:LL_NO_SENSORS = 1    skip the PawnIO driver (CPU temperature)
#                                 $env:LL_SOURCE = <folder> install from a local build (dist\LogicalLunge-x.y.z)
#                                 $env:LL_DEFAULTS = 1      no questions (default choices)
#                                 $env:LL_PLAIN = 1         simple prompts instead of gum
#                                 $env:LL_PREVIEW = 1       walk through the wizard, the download and a simulated install;
#                                                           nothing is stopped or changed, no UAC
#                                                           (= fail: a failed install, = warn: extras that failed)
& {
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$repo = 'KaanAlper/logical-lunge'
$GUM_VER = '2.0.2'
$E = [char]27

# ---------------------------------------------------------------- texts (Turkish / English)
$tr = (Get-UICulture).Name -like 'tr*'
$T = if ($tr) { @{
        tagline = "illogical-impulse'un Windows hali: tek parça, akıcı bir masaüstü"
        preparing = 'Hazırlanıyor'; release = 'Son sürüm aranıyor'
        welcome = "Logical Lunge'a hoş geldin"
        welcomeBody = "Birkaç kısa soru soracağız, sonra gerisini biz hallederiz. Kurulum sırasında Windows bir kez yönetici izni isteyecek.`nHer değişiklik yedeklenir; bir şey ters giderse ya da vazgeçersen her şey eski haline döner."
        version = 'Sürüm'; size = 'İndirme'
        qLang = 'Arayüz hangi dilde olsun?'; systemLang = 'Sistem dili'
        qColor = 'Odak rengi ne olsun? (etkin pencerenin kenarlığı)'; custom = 'Özel renk...'; qHex = 'Renk kodu (#rrggbb)'; badHex = 'Bu bir renk kodu gibi görünmüyor, örnek: #b69df8'
        qClock = 'Saat nasıl görünsün?'; h24 = '24 saat'; h12 = '12 saat'
        qExtras = 'Ek bileşenler (x ile seç / kaldır, Enter ile onayla)'; xTerm = 'Terminal: WezTerm + fish + starship'; xSensors = 'CPU sıcaklığı: PawnIO sürücüsü'
        summary = 'Özet'; sLang = 'Dil'; sColor = 'Odak rengi'; sClock = 'Saat'; sExtras = 'Ek bileşenler'; none = 'yok'
        qGo = 'Kuralım mı?'; go = 'Kur'; cancel = 'Vazgeç'
        downloading = 'Logical Lunge indiriliyor'; verifying = 'Paket doğrulanıyor'; extracting = 'Paket açılıyor'; stopping = 'Açık masaüstü kapatılıyor'
        uac = "Windows'un izin penceresini onayla..."
        installing = 'Logical Lunge kuruluyor'; ctrlc = 'Ctrl+C: vazgeç'; rollingBack = 'Değişiklikler geri alınıyor...'
        qStop = 'Kurulumu durdurup her şeyi eski haline getirelim mi?'; stopYes = 'Evet, durdur'; stopNo = 'Devam et'
        steps = @{ check = 'Windows denetleniyor'; runtimes = 'Gerekli bileşenler'; stop = 'Masaüstü durduruluyor'; files = 'Dosyalar kopyalanıyor'; config = 'Ayarların yazılıyor'; migrate = 'Önceki sürümden taşınıyor'; tools = 'Parlaklık ve sıcaklık araçları'; terminal = 'Terminal kuruluyor'; windows = 'Windows ayarları'; tasks = 'Başlangıç görevleri'; owner = 'İlk açılışa hazırlanıyor'; finish = 'Son dokunuşlar' }
        doneTitle = 'Hazır! Logical Lunge kuruldu'
        doneBody = "Masaüstün birkaç saniye içinde açılıyor.`n`n  Super              arama ve uygulamalar`n  Super + Enter      terminal`n  Super + Ctrl + ←/→ workspace değiştir`n  Sağ üst köşe       hızlı ayarlar ve bildirimler`n`nKaldırmak istersen: Ayarlar > Uygulamalar > Logical Lunge."
        errTitle = 'Olmadı, ama merak etme'
        errBody = "Kurulum '{0}' adımında takıldı. Bilgisayarında hiçbir şey yarım kalmadı: yapılan değişiklikler geri alındı ve önceki masaüstün yeniden açıldı."
        errDetail = 'Ayrıntı'; errLog = 'Günlük'
        errRetry = "Aynı komutu yeniden çalıştırarak tekrar deneyebilirsin. Sorun sürerse günlüğü bizimle paylaş:`nhttps://github.com/$repo/issues"
        netTitle = 'İnternete ulaşamadık'; netBody = 'Bağlantını kontrol edip aynı komutu yeniden çalıştır. Bilgisayarında hiçbir şey değişmedi.'
        cancelTitle = 'Kurulumdan vazgeçildi'; cancelBody = 'Her şey eski haline döndü, bilgisayarında hiçbir şey değişmedi.'
        uacTitle = 'İzin verilmedi'; uacBody = "Windows'un yönetici izni olmadan kurulum yapılamıyor. Hiçbir şey değişmedi; hazır olduğunda aynı komutu yeniden çalıştır."
        oldWin = "Logical Lunge Windows 10 2004 (19041) ya da daha yenisini istiyor; bu bilgisayar {0}."
        badPkg = 'İndirilen paket bozuk görünüyor (doğrulama tutmadı).'
        plainPick = 'Numara yaz ve Enter''a bas'; yes = 'e'
        retrying = 'bağlantı koptu, {0} sn sonra kaldığı yerden devam ({1}/5)'
        warnTitle = 'Birkaç ek parça kurulamadı'
        warnBody = 'Masaüstün tam çalışıyor, yalnızca bunlar eksik. Aynı komutu sonra yeniden çalıştırınca eksikler tamamlanır.'
        xBright = 'Harici monitör parlaklığı: ControlMyMonitor'
    } } else { @{
        tagline = 'illogical-impulse for Windows: one fluid desktop'
        preparing = 'Getting ready'; release = 'Looking for the latest release'
        welcome = 'Welcome to Logical Lunge'
        welcomeBody = "A few quick questions, then we take care of the rest. Windows will ask for administrator permission once.`nEvery change is backed up; if something goes wrong or you cancel, everything goes back to how it was."
        version = 'Version'; size = 'Download'
        qLang = 'Which language should the interface use?'; systemLang = 'System language'
        qColor = 'Pick a focus color (the border of the active window)'; custom = 'Custom color...'; qHex = 'Color code (#rrggbb)'; badHex = "That doesn't look like a color code, e.g. #b69df8"
        qClock = 'How should the clock look?'; h24 = '24-hour'; h12 = '12-hour'
        qExtras = 'Extras (x to toggle, Enter to confirm)'; xTerm = 'Terminal: WezTerm + fish + starship'; xSensors = 'CPU temperature: PawnIO driver'
        summary = 'Summary'; sLang = 'Language'; sColor = 'Focus color'; sClock = 'Clock'; sExtras = 'Extras'; none = 'none'
        qGo = 'Ready to install?'; go = 'Install'; cancel = 'Cancel'
        downloading = 'Downloading Logical Lunge'; verifying = 'Verifying the package'; extracting = 'Unpacking'; stopping = 'Closing the running desktop'
        uac = "Approve Windows' permission prompt..."
        installing = 'Installing Logical Lunge'; ctrlc = 'Ctrl+C: cancel'; rollingBack = 'Putting everything back...'
        qStop = 'Stop the install and put everything back?'; stopYes = 'Yes, stop'; stopNo = 'Keep going'
        steps = @{ check = 'Checking Windows'; runtimes = 'Required components'; stop = 'Stopping the desktop'; files = 'Copying files'; config = 'Writing your settings'; migrate = 'Moving data from the previous version'; tools = 'Brightness and temperature tools'; terminal = 'Installing the terminal'; windows = 'Windows settings'; tasks = 'Startup tasks'; owner = 'Preparing the first start'; finish = 'Finishing touches' }
        doneTitle = 'All set! Logical Lunge is installed'
        doneBody = "Your desktop opens in a few seconds.`n`n  Super              search and apps`n  Super + Enter      terminal`n  Super + Ctrl + ←/→ switch workspace`n  Top right corner   quick settings and notifications`n`nTo remove it: Settings > Apps > Logical Lunge."
        errTitle = "That didn't work, but don't worry"
        errBody = "The install got stuck at '{0}'. Nothing was left half-done: the changes were undone and your previous desktop was started again."
        errDetail = 'Details'; errLog = 'Log'
        errRetry = "You can try again by running the same command. If it keeps happening, please share the log with us:`nhttps://github.com/$repo/issues"
        netTitle = "We couldn't reach the internet"; netBody = 'Check your connection and run the same command again. Nothing on your computer was changed.'
        cancelTitle = 'Install cancelled'; cancelBody = 'Everything is back to how it was; nothing on your computer was changed.'
        uacTitle = 'Permission was not given'; uacBody = "The install needs Windows' administrator permission. Nothing was changed; run the same command again when you are ready."
        oldWin = 'Logical Lunge needs Windows 10 2004 (19041) or newer; this computer is {0}.'
        badPkg = 'The downloaded package looks damaged (the checksum does not match).'
        plainPick = 'Type a number and press Enter'; yes = 'y'
        retrying = 'connection dropped, resuming in {0} s ({1}/5)'
        warnTitle = "A few extras couldn't be installed"
        warnBody = 'Your desktop works fully; only these are missing. Run the same command again later to complete them.'
        xBright = 'External monitor brightness: ControlMyMonitor'
    } }
$STEP_IDS = 'check', 'runtimes', 'stop', 'files', 'config', 'migrate', 'tools', 'terminal', 'windows', 'tasks', 'owner', 'finish'

# ---------------------------------------------------------------- drawing
$C = @{ accent = '#b69df8'; text = '#e6e0e9'; dim = '#938f99'; ok = '#a8dab5'; err = '#f2b8b5'; warn = '#ffb77c' }
function Fg([string]$hex) { $h = $hex.TrimStart('#'); "$E[38;2;$([Convert]::ToInt32($h.Substring(0, 2), 16));$([Convert]::ToInt32($h.Substring(2, 2), 16));$([Convert]::ToInt32($h.Substring(4, 2), 16))m" }
$R = "$E[0m"
function Paint([string]$hex, [string]$s) { (Fg $hex) + $s + $R }
function Width { try { [Math]::Max(40, [Math]::Min(76, [Console]::WindowWidth - 4)) } catch { 72 } }
# Word-wrapped text lines for a box of inner width w
function Wrap([string]$text, [int]$w) {
    $out = New-Object Collections.Generic.List[string]
    foreach ($para in (($text -replace "`r", '').TrimEnd() -split "`n")) {
        if ($para.Length -le $w) { $out.Add($para); continue }
        $line = ''
        # a word longer than the box (a long path) is cut into pieces
        $words = foreach ($wd in ($para -split ' ')) { for ($k = 0; $k -lt [Math]::Max(1, $wd.Length); $k += $w) { $wd.Substring($k, [Math]::Min($w, [Math]::Max(0, $wd.Length - $k))) } }
        foreach ($word in $words) {
            if ($line.Length -eq 0) { $line = $word }
            elseif (($line.Length + 1 + $word.Length) -le $w) { $line += ' ' + $word }
            else { $out.Add($line); $line = $word }
        }
        $out.Add($line)
    }
    return $out
}
# Rounded box: title in the border color, body wrapped
function Box([string]$color, [string]$title, [string]$body) {
    $w = Width; $in = $w - 4
    $b = Fg $color
    Write-Host ''
    Write-Host ("  $b╭" + ('─' * ($w - 2)) + "╮$R")
    if ($title) {
        Write-Host ("  $b│$R " + "$E[1m" + (Fg $color) + $title.PadRight($in) + "$R $b│$R")
        Write-Host ("  $b│$R " + (' ' * $in) + " $b│$R")
    }
    foreach ($l in (Wrap $body $in)) { Write-Host ("  $b│$R " + (Fg $C.text) + $l.PadRight($in) + "$R $b│$R") }
    Write-Host ("  $b╰" + ('─' * ($w - 2)) + "╯$R")
}
function Banner {
    Clear-Host
    # figlet "Calvin S"; each line a shade of the accent
    $g = '#d0bcff', '#b69df8', '#977be6'
    $art = @(
        '╦  ┌─┐┌─┐┬┌─┐┌─┐┬    ╦  ┬ ┬┌┐┌┌─┐┌─┐',
        '║  │ ││ ┬││  ├─┤│    ║  │ │││││ ┬├┤ ',
        '╩═╝└─┘└─┘┴└─┘┴ ┴┴─┘  ╩═╝└─┘┘└┘└─┘└─┘')
    Write-Host ''
    for ($i = 0; $i -lt $art.Count; $i++) { Write-Host ('  ' + (Paint $g[$i] $art[$i])) }
    Write-Host ('  ' + (Paint $C.dim $T.tagline))
    Write-Host ''
}
function Say([string]$sym, [string]$color, [string]$text) { Write-Host ('  ' + (Paint $color $sym) + ' ' + (Paint $C.text $text)) }

# Spinner line redrawn in place while $work runs as a job-free polling loop
$SPIN = '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'
function Human([double]$b) { if ($b -ge 1MB) { '{0:0.0} MB' -f ($b / 1MB) } else { '{0:0} KB' -f ($b / 1KB) } }
# Animated bar: a soft highlight runs along the filled part
function Bar([double]$frac, [int]$width, [int]$tick) {
    $fill = [int][Math]::Floor($frac * $width)
    $s = ''
    for ($i = 0; $i -lt $width; $i++) {
        if ($i -lt $fill) { $s += $(if ((($i - $tick) % 24 + 24) % 24 -lt 3) { (Fg '#e8ddff') } else { (Fg $C.accent) }) + '━' }
        elseif ($i -eq $fill) { $s += (Fg $C.accent) + '╸' }
        else { $s += (Fg '#49454f') + '─' }
    }
    return $s + $R
}

# ---------------------------------------------------------------- input
$script:gum = $null
$script:cancelled = $false
function Assert-Answer([int]$code) { if ($code -eq 130) { $script:cancelled = $true; throw (New-Object OperationCanceledException) } }
# gum could not draw a prompt (an error, not an answer): this and every later question use the simple prompts
function Use-PlainPrompts { $script:gum = $null }
# items: @(@(label, value), ...); returns the chosen value
function Choose([string]$header, [object[]]$items, [string]$default) {
    if ($script:gum) {
        $args2 = @('choose', '--header', $header, '--label-delimiter', '|', '--cursor', '❯ ', '--cursor.foreground', $C.accent, '--header.foreground', $C.accent, '--selected.foreground', $C.accent, '--height', '16')
        $def = ($items | Where-Object { $_[1] -eq $default } | Select-Object -First 1)
        if ($def) { $args2 += @('--selected', $def[0]) }
        foreach ($it in $items) { $args2 += ($it[0] + '|' + $it[1]) }
        $out = & $script:gum @args2
        Assert-Answer $LASTEXITCODE
        if ($LASTEXITCODE -eq 0 -and $out) { return ([string]$out).Trim() }
        Use-PlainPrompts
    }
    Write-Host ('  ' + (Paint $C.accent $header))
    for ($i = 0; $i -lt $items.Count; $i++) { Write-Host ('    ' + (Paint $C.dim "$($i + 1))") + ' ' + $items[$i][0] + $(if ($items[$i][1] -eq $default) { Paint $C.accent '  ●' })) }
    $a = Read-Host ('  ' + $T.plainPick)
    $n = 0
    if ([int]::TryParse($a, [ref]$n) -and $n -ge 1 -and $n -le $items.Count) { return $items[$n - 1][1] }
    return $default
}
function Multi([string]$header, [object[]]$items, [string[]]$selected) {
    if ($script:gum) {
        $args2 = @('choose', '--no-limit', '--header', $header, '--cursor', '❯ ', '--cursor.foreground', $C.accent, '--header.foreground', $C.accent, '--selected.foreground', $C.accent, '--selected-prefix', '◆ ', '--unselected-prefix', '◇ ', '--cursor-prefix', '◇ ')
        if ($selected.Count) { $args2 += @('--selected', (($items | Where-Object { $selected -contains $_[1] } | ForEach-Object { $_[0] }) -join ',')) }
        foreach ($it in $items) { $args2 += $it[0] }
        $out = & $script:gum @args2
        Assert-Answer $LASTEXITCODE
        if ($LASTEXITCODE -eq 0) {
            $labels = @($out | Where-Object { $_ })
            return @($items | Where-Object { $labels -contains $_[0] } | ForEach-Object { $_[1] })
        }
        Use-PlainPrompts
    }
    $res = @()
    foreach ($it in $items) {
        $a = Read-Host ('  ' + $it[0] + ' [' + $T.yes + '/n]')
        if ($a -eq '' -or $a -like "$($T.yes)*" -or $a -like 'y*') { $res += $it[1] }
    }
    return $res
}
function Ask([string]$header, [string]$placeholder, [string]$value) {
    if ($script:gum) {
        # Windows PowerShell drops an empty argument, so --value goes only with a value (an empty one made gum
        # read --char-limit as the value, fail, and the color question repeat forever)
        $args2 = @('input', '--header', $header, '--placeholder', $placeholder, '--char-limit', '7', '--prompt', '❯ ', '--prompt.foreground', $C.accent, '--header.foreground', $C.accent, '--cursor.foreground', $C.accent)
        if ($value) { $args2 += @('--value', $value) }
        $out = & $script:gum @args2
        Assert-Answer $LASTEXITCODE
        if ($LASTEXITCODE -eq 0) { return ([string]$out).Trim() }
        Use-PlainPrompts
    }
    return (Read-Host ('  ' + $header)).Trim()
}
function Confirm([string]$prompt, [string]$yes, [string]$no, [bool]$default = $true) {
    if ($script:gum) {
        $d = if ($default) { '--default=true' } else { '--default=false' }
        & $script:gum confirm $prompt --affirmative $yes --negative $no $d --prompt.foreground $C.accent --selected.background $C.accent --selected.foreground '#21005d'
        if ($LASTEXITCODE -eq 130) { return $false }
        if ($LASTEXITCODE -le 1) { return ($LASTEXITCODE -eq 0) }
        Use-PlainPrompts
    }
    $a = Read-Host ("  $prompt [" + $T.yes + '/n]')
    return ($a -eq '' -or $a -like "$($T.yes)*" -or $a -like 'y*')
}

# ---------------------------------------------------------------- download with an animated progress bar
function Get-WithBar([string]$url, [string]$dst, [string]$label, [long]$sizeHint) {
    # a dropped connection continues where it stopped (HTTP Range) instead of starting over or giving up
    $tick = 0; $last = ''
    for ($try = 1; $try -le 5; $try++) {
        try {
            $have = if (Test-Path $dst) { (Get-Item $dst).Length } else { 0 }
            $req = [Net.HttpWebRequest]::Create($url)
            $req.UserAgent = 'LogicalLunge-Install'; $req.Timeout = 30000; $req.ReadWriteTimeout = 60000
            if ($have -gt 0) { $req.AddRange([long]$have) }
            $res = $req.GetResponse()
            try {
                if ([int]$res.StatusCode -ne 206) { $have = 0 }
                $total = if ($res.ContentLength -gt 0) { $have + $res.ContentLength } else { $sizeHint }
                $in = $res.GetResponseStream()
                $out = if ($have -gt 0) { New-Object IO.FileStream($dst, [IO.FileMode]::Append) } else { [IO.File]::Create($dst) }
                try {
                    $buf = New-Object byte[] 131072; $done = $have
                    $sw = [Diagnostics.Stopwatch]::StartNew(); $draw = [Diagnostics.Stopwatch]::StartNew()
                    while (($n = $in.Read($buf, 0, $buf.Length)) -gt 0) {
                        $out.Write($buf, 0, $n); $done += $n
                        if ($draw.ElapsedMilliseconds -ge 60) {
                            $draw.Restart(); $tick++
                            Poll-CtrlC
                            # 1.0, not 1: [Math]::Min(1, 0.74) picks the integer overload and gives 1 (the bar sat at 0 %, then jumped to 100 %)
                            $frac = if ($total -gt 0) { [Math]::Min(1.0, [double]$done / $total) } else { 0 }
                            $speed = if ($sw.Elapsed.TotalSeconds -gt 0.3) { (Human (($done - $have) / $sw.Elapsed.TotalSeconds)) + '/s' } else { '' }
                            $pct = if ($total -gt 0) { '{0,3:0}%' -f ($frac * 100) } else { '' }
                            Write-Host -NoNewline ("`r  " + (Paint $C.accent $SPIN[$tick % $SPIN.Count]) + ' ' + $label + '  ' + (Bar $frac 28 $tick) + ' ' + (Paint $C.text $pct) + '  ' + (Paint $C.dim ((Human $done) + $(if ($total -gt 0) { ' / ' + (Human $total) }) + '  ' + $speed)) + "$E[K")
                        }
                    }
                }
                finally { $out.Dispose(); $in.Dispose() }
            }
            finally { $res.Dispose() }
            if ($total -gt 0 -and $done -lt $total) { throw "the connection closed at $(Human $done) of $(Human $total)" }
            Write-Host ("`r  " + (Paint $C.ok '✓') + ' ' + $label + '  ' + (Paint $C.dim (Human (Get-Item $dst).Length)) + "$E[K")
            return
        }
        catch [OperationCanceledException] { throw }
        catch {
            $last = $_.Exception.Message
            # 416: what is on disk does not fit the file on the server; start over
            # (not $e: variable names ignore case, and $E is the escape character the drawing uses)
            for ($ex = $_.Exception; $ex; $ex = $ex.InnerException) {
                if ($ex -is [Net.WebException] -and $ex.Response -and [int]$ex.Response.StatusCode -eq 416) { Remove-Item $dst -Force -ErrorAction SilentlyContinue }
                # 404 / 403: the file is not there; asking again does not help
                if ($ex -is [Net.WebException] -and $ex.Response -and [int]$ex.Response.StatusCode -in 403, 404, 410) { $try = 5 }
            }
            if ($try -eq 5) { break }
            for ($w = 2 * $try; $w -gt 0; $w--) {
                Write-Host -NoNewline ("`r  " + (Paint $C.warn '↻') + ' ' + $label + '  ' + (Paint $C.dim ($T.retrying -f $w, ($try + 1))) + "$E[K")
                for ($k = 0; $k -lt 10; $k++) { Start-Sleep -Milliseconds 100; Poll-CtrlC }
            }
        }
    }
    throw $last
}
# A short step: "◌ label" while it runs, then ✓ / ✗ in place
function With-Spinner([string]$label, [scriptblock]$sb) {
    Write-Host -NoNewline ('  ' + (Paint $C.accent '◌') + ' ' + $label)
    # (not $r: variable names ignore case, and $R is the colour reset Paint appends)
    try { $result = & $sb; Write-Host ("`r  " + (Paint $C.ok '✓') + ' ' + $label + "$E[K"); return $result }
    catch { Write-Host ("`r  " + (Paint $C.err '✗') + ' ' + $label + "$E[K"); throw }
}
# Ctrl+C while we draw: treated as input so that it can be confirmed instead of killing the install half-way
function Poll-CtrlC {
    try {
        while ([Console]::KeyAvailable) {
            $k = [Console]::ReadKey($true)
            if ($k.Key -eq 'C' -and ($k.Modifiers -band [ConsoleModifiers]::Control)) { $script:ctrlC = $true }
        }
    }
    catch {}
    if ($script:ctrlC -and -not $script:setupStarted) { $script:cancelled = $true; throw (New-Object OperationCanceledException) }
}

# The desktop we stopped comes back when nothing was installed (cancel before setup, UAC declined)
function Start-Desktop-Again {
    foreach ($m in 'LogicalLunge\state\maintenance', 'logical-lunge\maintenance') { Remove-Item (Join-Path $env:LOCALAPPDATA $m) -Force -ErrorAction SilentlyContinue }
    # from its sign-in task: the core starts with its rights (elevated); 0.1.x had its own task
    if (Get-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Start' -ErrorAction SilentlyContinue) { Start-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Start' }
    else { Start-ScheduledTask -TaskPath '\LL\' -TaskName 'GlazeWM' -ErrorAction SilentlyContinue }
}

# ---------------------------------------------------------------- main
$work =Join-Path $env:TEMP ('lunge-install-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
$oldOut = [Console]::OutputEncoding
$script:setup = $null; $script:setupStarted = $false; $script:ctrlC = $false; $stopped = $false
$interactive = (-not $env:LL_DEFAULTS) -and [Environment]::UserInteractive -and -not [Console]::IsInputRedirected
$preview = [bool]$env:LL_PREVIEW
try {
    [Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
    New-Item -ItemType Directory -Force $work | Out-Null
    Banner

    $build = [int](Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuildNumber
    if ($build -lt 19041) { Box $C.err $T.errTitle ($T.oldWin -f $build); return }

    # gum (the prompts): downloaded to %TEMP%, removed at the end; without it the prompts are simple numbered menus
    if ($interactive -and -not $env:LL_PLAIN) {
        try {
            With-Spinner $T.preparing {
                $gz = Join-Path $work 'gum.zip'
                $base = "https://github.com/charmbracelet/gum/releases/download/v$GUM_VER"
                (New-Object Net.WebClient).DownloadFile("$base/gum_${GUM_VER}_Windows_x86_64.zip", $gz)
                $sums = (New-Object Net.WebClient).DownloadString("$base/checksums.txt")
                $want = ($sums -split "`n" | Where-Object { $_ -match "gum_${GUM_VER}_Windows_x86_64\.zip$" } | Select-Object -First 1) -split '\s+' | Select-Object -First 1
                if (-not $want -or $want.ToUpper() -ne (Get-FileHash $gz -Algorithm SHA256).Hash) { throw 'gum checksum' }
                Expand-Archive $gz (Join-Path $work 'gum') -Force
                $script:gum = (Get-ChildItem (Join-Path $work 'gum') -Recurse -Filter gum.exe | Select-Object -First 1).FullName
            } | Out-Null
        }
        catch { $script:gum = $null }
    }

    # the release to install
    $src = $env:LL_SOURCE; $zipUrl = $null; $shaUrl = $null; $zipSize = 0; $ver = $null
    if ($src) { $ver = try { [IO.File]::ReadAllText((Join-Path $src 'VERSION')).Trim() } catch { '?' } }
    else {
        try {
            $rel = With-Spinner $T.release { Invoke-RestMethod -UseBasicParsing -Headers @{ 'User-Agent' = 'LogicalLunge-Install' } "https://api.github.com/repos/$repo/releases/latest" }
        }
        catch { Box $C.err $T.netTitle $T.netBody; return }
        $asset = $rel.assets | Where-Object { $_.name -like 'LogicalLunge-*.zip' } | Select-Object -First 1
        if (-not $asset) { Box $C.err $T.netTitle $T.netBody; return }
        $zipUrl = $asset.browser_download_url; $zipSize = [long]$asset.size; $ver = $rel.tag_name
        $sha = $rel.assets | Where-Object { $_.name -eq "$($asset.name).sha256" } | Select-Object -First 1
        if ($sha) { $shaUrl = $sha.browser_download_url }
    }

    Box $C.accent $T.welcome ($T.welcomeBody + "`n`n" + "$($T.version): $ver" + $(if ($zipSize) { "   $($T.size): $(Human $zipSize)" }))
    Write-Host ''

    # ------------------------------------------------------------ choices
    $choice = [ordered]@{ language = 'system'; clock = '24'; focusColor = '#b69df8' }
    $extras = @('terminal', 'sensors')
    if ($env:LL_NO_TERMINAL) { $extras = @($extras | Where-Object { $_ -ne 'terminal' }) }
    if ($env:LL_NO_SENSORS) { $extras = @($extras | Where-Object { $_ -ne 'sensors' }) }
    if ($interactive) {
        $sysName = (Get-UICulture).NativeName
        $langs = @(@("$($T.systemLang) ($sysName)", 'system'), @('Türkçe', 'tr'), @('English', 'en'), @('Deutsch', 'de'), @('Français', 'fr'), @('Español', 'es'), @('Italiano', 'it'), @('Português', 'pt'),
            @('Русский (Russian)', 'ru'), @('Українська (Ukrainian)', 'uk'), @('Polski', 'pl'), @('日本語 (Japanese)', 'ja'), @('中文 (Chinese)', 'zh'), @('한국어 (Korean)', 'ko'), @('العربية (Arabic)', 'ar'))
        $choice.language = Choose $T.qLang $langs 'system'
        $langLabel = ($langs | Where-Object { $_[1] -eq $choice.language } | Select-Object -First 1)[0]
        Say '✓' $C.ok "$($T.sLang): $langLabel"

        $colors = if ($tr) { @(@('Mor (varsayılan)', '#b69df8'), @('Mavi', '#8ab4f8'), @('Camgöbeği', '#7fd4c9'), @('Yeşil', '#a6d189'), @('Pembe', '#f5a3c7'), @('Turuncu', '#ffb77c'), @('Kırmızı', '#f28b82')) }
                  else { @(@('Purple (default)', '#b69df8'), @('Blue', '#8ab4f8'), @('Teal', '#7fd4c9'), @('Green', '#a6d189'), @('Pink', '#f5a3c7'), @('Orange', '#ffb77c'), @('Red', '#f28b82')) }
        Write-Host ('  ' + (($colors | ForEach-Object { (Paint $_[1] '██') + ' ' + (Paint $C.dim $_[0]) }) -join '  '))
        $pick = Choose $T.qColor (@($colors | ForEach-Object { , @(($_[0] + '  ' + $_[1]), $_[1]) }) + , @($T.custom, 'custom')) '#b69df8'
        for ($try = 1; $pick -eq 'custom'; $try++) {
            $hex = Ask $T.qHex '#b69df8' ''
            if ($hex -notmatch '^#') { $hex = '#' + $hex }
            if ($hex -match '^#[0-9a-fA-F]{6}$') { $pick = $hex.ToLower() }
            elseif ($try -ge 3) { $pick = '#b69df8' }   # the default instead of asking forever
            else { Say '!' $C.warn $T.badHex }
        }
        $choice.focusColor = $pick
        Say '✓' $C.ok ("$($T.sColor): " + (Paint $pick '██') + ' ' + $pick)

        $now = Get-Date
        $choice.clock = Choose $T.qClock @(@("$($T.h24)   $($now.ToString('HH:mm'))", '24'), @("$($T.h12)   $($now.ToString('h:mm tt', [Globalization.CultureInfo]::InvariantCulture))", '12')) '24'
        Say '✓' $C.ok "$($T.sClock): $(if ($choice.clock -eq '12') { $T.h12 } else { $T.h24 })"

        $extras = @(Multi $T.qExtras @(@($T.xTerm, 'terminal'), @($T.xSensors, 'sensors')) $extras)
        $extraText = if ($extras.Count) { (@($extras | ForEach-Object { if ($_ -eq 'terminal') { $T.xTerm } else { $T.xSensors } }) -join "`n  ") } else { $T.none }

        Box $C.accent $T.summary ("$($T.sLang): $langLabel`n$($T.sColor): $($choice.focusColor)`n$($T.sClock): $(if ($choice.clock -eq '12') { $T.h12 } else { $T.h24 })`n$($T.sExtras):`n  $extraText")
        Write-Host ''
        if (-not (Confirm $T.qGo $T.go $T.cancel $true)) { $script:cancelled = $true; throw (New-Object OperationCanceledException) }
        Write-Host ''
    }

    # ------------------------------------------------------------ package
    # no console input (redirected, LL_DEFAULTS in a pipeline): Ctrl+C then simply ends the script
    try { [Console]::TreatControlCAsInput = $true } catch {}
    if (-not $src) {
        $zip = Join-Path $work 'LogicalLunge.zip'
        try { Get-WithBar $zipUrl $zip $T.downloading $zipSize }
        catch [OperationCanceledException] { throw }
        catch { Box $C.err $T.netTitle ($T.netBody + "`n`n$($T.errDetail): $($_.Exception.Message)"); return }
        if ($shaUrl) {
            $ok = With-Spinner $T.verifying {
                $raw = (New-Object Net.WebClient).DownloadString($shaUrl)
                $expected = ($raw -split '\s+')[0].Trim().ToUpper()
                (-not $expected) -or ($expected -eq (Get-FileHash $zip -Algorithm SHA256).Hash)
            }
            if (-not $ok) { Box $C.err $T.errTitle $T.badPkg; return }
        }
        With-Spinner $T.extracting { Expand-Archive $zip (Join-Path $work 'pkg') -Force } | Out-Null
        $src = (Get-ChildItem (Join-Path $work 'pkg') -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'installer\setup.ps1') } | Select-Object -First 1).FullName
        if (-not $src) { Box $C.err $T.errTitle $T.badPkg; return }
    }
    if (-not (Test-Path (Join-Path $src 'installer\setup.ps1'))) { Box $C.err $T.errTitle $T.badPkg; return }
    Poll-CtrlC

    # ------------------------------------------------------------ stop the running desktop (as the user)
    # A graceful exit brings back the windows of hidden workspaces; the package's core also knows the 0.1.x parts
    $running = (Get-Process lunge, lunge-tiling, lunge-shell, ll-helper -ErrorAction SilentlyContinue) -or
        ((Get-Process glazewm -ErrorAction SilentlyContinue) -and (Test-Path (Join-Path $env:USERPROFILE '.glzr\logical-lunge')))
    if ($running -and -not $preview) {
        With-Spinner $T.stopping { & (Join-Path $src 'app\lunge.exe') --stop-desktop | Out-Null } | Out-Null
        $stopped = $true
    }

    # ------------------------------------------------------------ elevated setup, hidden; we draw its progress
    $choicesFile = Join-Path $work 'choices.json'
    [IO.File]::WriteAllText($choicesFile, ($choice | ConvertTo-Json), (New-Object Text.UTF8Encoding $false))
    $progressFile = Join-Path $work 'progress.json'; $cancelFile = Join-Path $work 'cancel'
    $me = [Security.Principal.WindowsIdentity]::GetCurrent()
    $args2 = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-WindowStyle', 'Hidden', '-File', "`"$(Join-Path $src 'installer\setup.ps1')`"",
        '-Source', "`"$src`"", '-UserProfile', "`"$env:USERPROFILE`"", '-UserSid', $me.User.Value, '-UserName', "`"$($me.Name)`"",
        '-Choices', "`"$choicesFile`"", '-ProgressFile', "`"$progressFile`"", '-CancelFile', "`"$cancelFile`"")
    if ($extras -notcontains 'terminal') { $args2 += '-NoTerminal' }
    if ($extras -notcontains 'sensors') { $args2 += '-NoSensors' }
    if ($preview) {
        # the steps of setup.ps1 played by an ordinary hidden process: same progress file, same cancel file
        $sim = Join-Path $work 'preview-setup.ps1'
        [IO.File]::WriteAllText($sim, @'
param([string]$ProgressFile, [string]$CancelFile, [string]$FailAt)
$utf8 = New-Object Text.UTF8Encoding $false
function P([hashtable]$o) { [IO.File]::WriteAllText($ProgressFile, ($o | ConvertTo-Json -Compress), $utf8) }
$n = 0
foreach ($id in 'check', 'runtimes', 'stop', 'files', 'config', 'migrate', 'tools', 'terminal', 'windows', 'tasks', 'owner', 'finish') {
    $n++
    for ($t = 0; $t -le 100; $t += 5) {
        if (Test-Path $CancelFile) { P @{ state = 'rollback'; step = $id; n = $n }; Start-Sleep 2; exit 2 }
        if ($id -eq $FailAt -and $t -ge 50 -and $FailAt -ne 'terminal') { P @{ state = 'error'; step = $id; n = $n; error = 'Preview: simulated failure' }; exit 1 }
        $p = @{ state = 'running'; step = $id; n = $n }
        if ($id -eq 'files') { $p.file = 'app\lunge-shell.exe'; $p.done = $t; $p.size = 100 }
        P $p; Start-Sleep -Milliseconds $(if ($id -eq 'files') { 120 } else { 35 })
    }
}
if ($FailAt -eq 'terminal') { P @{ state = 'done'; step = 'finish'; n = 12; warn = @('terminal', 'sensors') } } else { P @{ state = 'done'; step = 'finish'; n = 12 } }
'@, (New-Object Text.UTF8Encoding $false))
        $failAt = switch ($env:LL_PREVIEW) { 'fail' { 'tools' } 'warn' { 'terminal' } default { '' } }
        $script:setup = Start-Process powershell.exe -WindowStyle Hidden -PassThru -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$sim`"",
            '-ProgressFile', "`"$progressFile`"", '-CancelFile', "`"$cancelFile`"", '-FailAt', $(if ($failAt) { $failAt } else { '""' }))
    }
    else {
        Say '●' $C.accent $T.uac
        try { $script:setup = Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -PassThru -ArgumentList $args2 }
        catch {
            if ($stopped) { Start-Desktop-Again }
            Box $C.warn $T.uacTitle $T.uacBody; return
        }
    }
    $script:setupStarted = $true

    Write-Host ''
    Write-Host ('  ' + "$E[1m" + (Paint $C.accent $T.installing))
    $lines = $STEP_IDS.Count + 2
    for ($i = 0; $i -lt $lines; $i++) { Write-Host '' }
    Write-Host -NoNewline "$E[?25l"
    $tick = 0; $clock = [Diagnostics.Stopwatch]::StartNew(); $p = $null; $asked = $false
    while ($true) {
        $done = $script:setup.HasExited
        try { if (Test-Path $progressFile) { $p = [IO.File]::ReadAllText($progressFile) | ConvertFrom-Json } } catch {}
        Poll-CtrlC
        if ($script:ctrlC -and -not $asked -and -not $done) {
            $asked = $true
            Write-Host -NoNewline "$E[?25h"; Write-Host ''
            if (Confirm $T.qStop $T.stopYes $T.stopNo $false) { New-Item -ItemType File -Force $cancelFile | Out-Null }
            else { $script:ctrlC = $false; $asked = $false }
            Write-Host -NoNewline "$E[?25l"
            for ($i = 0; $i -lt $lines; $i++) { Write-Host '' }
        }
        # redraw the step list in place
        $n = if ($p) { [int]$p.n } else { 0 }
        $state = if ($p) { [string]$p.state } else { 'running' }
        $out = "$E[$($lines)A"
        for ($i = 0; $i -lt $STEP_IDS.Count; $i++) {
            $label = $T.steps[$STEP_IDS[$i]]
            if ($i + 1 -lt $n -or ($state -eq 'done')) { $row = (Paint $C.ok '✓') + ' ' + (Paint $C.dim $label) }
            elseif ($i + 1 -eq $n -and $state -eq 'running') {
                $row = (Paint $C.accent $SPIN[$tick % $SPIN.Count]) + ' ' + (Paint $C.text $label)
                if ($p.file -and $p.size) { $row += '  ' + (Bar ([double]$p.done / [double]$p.size) 18 $tick) + ' ' + (Paint $C.dim ('{0,3:0}%' -f (100 * [double]$p.done / [double]$p.size))) }
            }
            elseif ($i + 1 -eq $n -and $state -eq 'error') { $row = (Paint $C.err '✗') + ' ' + (Paint $C.text $label) }
            else { $row = (Paint '#49454f' '·') + ' ' + (Paint '#6f6a75' $label) }
            $out += "`r  $row$E[K`n"
        }
        $foot = if ($state -eq 'rollback' -or (Test-Path $cancelFile)) { Paint $C.warn $T.rollingBack } else { Paint $C.dim $T.ctrlC }
        $out += "`r$E[K`n`r  " + (Paint $C.dim ('{0:mm\:ss}' -f $clock.Elapsed)) + '  ' + $foot + "$E[K`n"
        Write-Host -NoNewline $out
        if ($done) { break }
        Start-Sleep -Milliseconds 90; $tick++
    }
    Write-Host -NoNewline "$E[?25h"
    $code = $script:setup.ExitCode
    $script:setupStarted = $false
    $log = Join-Path $env:TEMP 'logical-lunge-install.log'
    if ($code -eq 0) {
        Box $C.ok $T.doneTitle $T.doneBody
        # optional parts that failed (terminal, sensors, brightness): the desktop is installed, these are reported
        $warn = @($p.warn | Where-Object { $_ })
        if ($warn.Count) {
            $names = @{ terminal = $T.xTerm; sensors = $T.xSensors; brightness = $T.xBright }
            $list = @($warn | ForEach-Object { '• ' + $(if ($names.ContainsKey([string]$_)) { $names[[string]$_] } else { [string]$_ }) }) -join "`n"
            Box $C.warn $T.warnTitle ($list + "`n`n" + $T.warnBody + "`n$($T.errLog): $log")
        }
    }
    elseif ($code -eq 2 -or (Test-Path $cancelFile)) { Box $C.warn $T.cancelTitle $T.cancelBody }
    else {
        $stepName = if ($p -and $T.steps.ContainsKey([string]$p.step)) { $T.steps[[string]$p.step] } else { '?' }
        $detail = if ($p -and $p.error) { [string]$p.error } else { "exit $code" }
        Box $C.err $T.errTitle (($T.errBody -f $stepName) + "`n`n$($T.errDetail): $detail`n$($T.errLog): $log`n`n" + $T.errRetry)
    }
}
catch [OperationCanceledException] {
    if ($stopped -and -not $script:setupStarted) { Start-Desktop-Again }
    Write-Host ''
    Box $C.warn $T.cancelTitle $T.cancelBody
}
catch {
    if ($stopped -and -not $script:setupStarted) { Start-Desktop-Again }
    Box $C.err $T.errTitle (($T.errBody -f $T.preparing) + "`n`n$($T.errDetail): $($_.Exception.Message)`n`n" + $T.errRetry)
}
finally {
    # Ctrl+C that got through (e.g. while Windows asked for permission): the elevated setup rolls back by itself
    if ($script:setupStarted -and $script:setup -and -not $script:setup.HasExited) {
        New-Item -ItemType File -Force (Join-Path $work 'cancel') -ErrorAction SilentlyContinue | Out-Null
        [void]$script:setup.WaitForExit(120000)
    }
    try { [Console]::TreatControlCAsInput = $false } catch {}
    Write-Host -NoNewline "$E[?25h$R"
    try { [Console]::OutputEncoding = $oldOut } catch {}
    # nothing stays in %TEMP%: gum, the package, the progress files
    if (Test-Path $work) { Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue }
    Write-Host ''
}
}
