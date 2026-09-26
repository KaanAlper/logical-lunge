# Logical Lunge - main installer (runs elevated, launched by install.ps1 or scripts\update-install.ps1).
# Everything the desktop needs is installed and configured here with a single UAC prompt. Every Windows setting that
# is changed is backed up first so uninstall.ps1 can restore it. If anything fails or the install is cancelled, the
# previous state is put back (app files, config, registry, tasks) and the previous desktop is started again.
#
# Layout:  %ProgramFiles%\LogicalLunge            app (package app\ folder + downloaded tools); protected, because the
#                                                core and the window manager run elevated (task "Start", highest privileges)
#          ~\.config\logical-lunge                user settings (config.yaml, keybinds.json, prefs.json)
#          %LOCALAPPDATA%\LogicalLunge            data (state, logs, webview, clipboard, update)
# Installs of 0.1.x (~\.glzr\logical-lunge, ~\.glzr\zebar, ~\.glzr\glazewm, %LOCALAPPDATA%\logical-lunge) are
# migrated and removed once the new install has succeeded.
param(
    [Parameter(Mandatory = $true)][string]$Source,      # extracted release folder
    [Parameter(Mandatory = $true)][string]$UserProfile, # profile of the user who ran install.ps1
    [Parameter(Mandatory = $true)][string]$UserSid,
    [Parameter(Mandatory = $true)][string]$UserName,    # DOMAIN\user
    [switch]$NoTerminal,                                # skip WezTerm + MSYS2 fish
    [switch]$NoSensors,                                 # skip PawnIO driver (CPU temperature)
    [string]$Choices,                                   # first-install choices (JSON: focusColor, language, clock)
    [string]$ProgressFile,                              # progress for the installer UI (JSON, rewritten per step)
    [string]$CancelFile                                 # the installer UI creates it to cancel (rolled back)
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# ---- pinned upstream versions (tested together) ----
$LHM_VER = 'v0.9.6'
$PAWNIO_VER = '2.2.0'
$NERDFONT_VER = 'v3.5.1'
$STARSHIP_VER = 'v1.26.0'
$EZA_VER = 'v0.23.5'
$FZF_VER = 'v0.58.0'

$LOCAL = Join-Path $UserProfile 'AppData\Local'
$APP = Join-Path $env:ProgramFiles 'LogicalLunge'
# early 0.2 builds installed per user; moved to the protected location
$OLD_APP = Join-Path $LOCAL 'Programs\LogicalLunge'
$DATA = Join-Path $LOCAL 'LogicalLunge'
$STATE = Join-Path $DATA 'state'
$CONF = Join-Path $UserProfile '.config\logical-lunge'
$PACK = Join-Path $APP 'ui\logical-lunge'
$RB = Join-Path $DATA 'rollback'
$OLD_LL = Join-Path $UserProfile '.glzr\logical-lunge'
$OLD_ZB = Join-Path $UserProfile '.glzr\zebar'
$OLD_GW = Join-Path $UserProfile '.glzr\glazewm'
$OLD_STATE = Join-Path $LOCAL 'logical-lunge'
$OLD_WEB = Join-Path $UserProfile 'AppData\Roaming\zebar\webview-cache\logical-lunge'
$HKU = "Registry::HKEY_USERS\$UserSid"
$LOG = Join-Path $env:TEMP 'logical-lunge-install.log'
$DL = Join-Path $env:TEMP 'll-downloads'
$UTF8 = New-Object Text.UTF8Encoding $false
# Files that come from the package (app\): moved aside before the copy, moved back on rollback
$OWNED = 'lunge.exe', 'lunge-tiling.exe', 'lunge-tiling-cli.exe', 'lunge-tiling-watcher.exe', 'lunge-shell.exe', 'VERSION',
'uninstall.ps1', 'ui', 'scripts', 'tools\lunge-media.exe', 'tools\temps\lunge-temps.exe', 'tools\termcolors', 'tools\songrec'
$TOTAL_STEPS = 12

$appExisted = Test-Path (Join-Path $APP 'lunge.exe')
$legacy = Test-Path (Join-Path $OLD_LL 'helper\ll-helper.exe')
$perUser = Test-Path (Join-Path $OLD_APP 'lunge.exe')
New-Item -ItemType Directory -Force $APP, $STATE, $CONF, $DL | Out-Null

function Log([string]$m) { $line = (Get-Date -Format 'HH:mm:ss ') + $m; Add-Content -Path $LOG -Value $line -Encoding UTF8; Write-Host $m }
function Progress([string]$state, [hashtable]$extra) {
    if (-not $ProgressFile) { return }
    $j = [ordered]@{ state = $state; step = $script:stepId; n = $script:stepNo; total = $TOTAL_STEPS }
    if ($extra) { foreach ($k in $extra.Keys) { $j[$k] = $extra[$k] } }
    try { [IO.File]::WriteAllText($ProgressFile, ($j | ConvertTo-Json -Compress), $UTF8) } catch {}
}
function Assert-NotCancelled { if ($CancelFile -and (Test-Path $CancelFile)) { throw (New-Object OperationCanceledException 'Installation cancelled.') } }
$script:stepNo = 0; $script:stepId = 'start'
# id: a stable name the installer UI translates; m: English text for the log
function Step([string]$id, [string]$m) {
    Assert-NotCancelled
    $script:stepNo++; $script:stepId = $id
    Log ''; Log "==> $m"
    Progress 'running' $null
}
# Streams the download so the installer UI can draw a progress bar; cached in %TEMP%\ll-downloads
function Get-File([string]$url, [string]$name) {
    $dst = Join-Path $DL $name
    if (Test-Path $dst) { return $dst }
    Log "    download $url"
    $req = [Net.HttpWebRequest]::Create($url)
    $req.UserAgent = 'LogicalLunge-Setup'; $req.Timeout = 30000; $req.ReadWriteTimeout = 60000
    $res = $req.GetResponse()
    try {
        $total = $res.ContentLength
        $in = $res.GetResponseStream(); $out = [IO.File]::Create("$dst.part")
        try {
            $buf = New-Object byte[] 262144; $done = 0; $sw = [Diagnostics.Stopwatch]::StartNew()
            while (($n = $in.Read($buf, 0, $buf.Length)) -gt 0) {
                $out.Write($buf, 0, $n); $done += $n
                if ($sw.ElapsedMilliseconds -ge 150) { $sw.Restart(); Progress 'running' @{ file = $name; done = $done; size = $total }; Assert-NotCancelled }
            }
        }
        finally { $out.Dispose(); $in.Dispose() }
    }
    finally { $res.Dispose() }
    Move-Item "$dst.part" $dst -Force
    Progress 'running' $null
    return $dst
}
function Gh-Asset([string]$repo, [string]$tag, [string]$pattern) {
    $rel = Invoke-RestMethod -UseBasicParsing -Headers @{ 'User-Agent' = 'LogicalLunge-Setup' } "https://api.github.com/repos/$repo/releases/tags/$tag"
    $a = $rel.assets | Where-Object { $_.name -match $pattern } | Select-Object -First 1
    if (-not $a) { throw "asset '$pattern' not found in $repo $tag" }
    return Get-File $a.browser_download_url $a.name
}
function Hash([string]$t) { $sha = [Security.Cryptography.SHA256]::Create(); try { [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($t))).Replace('-', '') } finally { $sha.Dispose() } }

# ---- rollback bookkeeping ----
$script:runReg = New-Object Collections.ArrayList    # registry values changed by this run (old value)
$script:created = New-Object Collections.ArrayList   # files / folders this run created
$script:moves = New-Object Collections.ArrayList     # (from, to) folders moved from the old layout
$script:newTasks = New-Object Collections.ArrayList  # tasks this run registered that did not exist before
$script:ownedSaved = $false
$script:configSaved = $false
$script:explorerRestarted = $false
function Remember-Created([string]$p) { if (-not (Test-Path $p)) { [void]$script:created.Add($p) } }
function Move-Tracked([string]$from, [string]$to) {
    New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
    Move-Item $from $to
    [void]$script:moves.Add(@($from, $to))
}

# ---- backup of every setting we touch (restored by uninstall.ps1) ----
$backupFile = Join-Path $STATE 'install-backup.json'
$oldBackup = Join-Path $OLD_STATE 'install-backup.json'
if (-not (Test-Path $backupFile) -and (Test-Path $oldBackup)) { Copy-Item $oldBackup $backupFile }
$backup = @{ registry = @(); installed = @(); version = $null }
if (Test-Path $backupFile) { $backup = Get-Content $backupFile -Raw | ConvertFrom-Json | ForEach-Object { @{ registry = @($_.registry); installed = @($_.installed); version = $_.version } } }
$backup.version = (Get-Content (Join-Path $Source 'VERSION') -ErrorAction SilentlyContinue)  # an update must not keep the old version number
function Save-Backup { $backup | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 $backupFile }
function Set-Reg([string]$path, [string]$name, $value, [string]$type = 'DWord') {
    if (-not (Test-Path $path)) { New-Item -Path $path -Force | Out-Null }
    $exists = $null -ne (Get-ItemProperty -Path $path -Name $name -ErrorAction SilentlyContinue)
    $old = if ($exists) { (Get-ItemProperty -Path $path -Name $name).$name } else { $null }
    if (-not ($backup.registry | Where-Object { $_.path -eq $path -and $_.name -eq $name })) {
        $oldOut = if ($old -is [byte[]]) { [Convert]::ToBase64String($old) } else { $old }
        $backup.registry += @{ path = $path; name = $name; existed = $exists; old = $oldOut; type = $type; binary = ($old -is [byte[]]) }
        Save-Backup
    }
    [void]$script:runReg.Add(@{ path = $path; name = $name; existed = $exists; old = $old; type = $type })
    Set-ItemProperty -Path $path -Name $name -Value $value -Type $type
}
function Mark-Installed([string]$what) { if ($backup.installed -notcontains $what) { $backup.installed += $what; Save-Backup } }
# PATH: only our own entries are added (and later removed) - never restore the whole value
function Add-UserPath([string]$dir) {
    $cur = (Get-ItemProperty "$HKU\Environment" -Name Path -ErrorAction SilentlyContinue).Path
    if ($cur -and ($cur -split ';') -contains $dir) { Mark-Installed ("path:" + $dir); return }
    $new = ((@($cur -split ';' | Where-Object { $_ }) + $dir) -join ';')
    Set-ItemProperty "$HKU\Environment" -Name Path -Value $new -Type ExpandString
    Mark-Installed ("path:" + $dir)
}
function Remove-UserPath([string]$dir) {
    $cur = (Get-ItemProperty "$HKU\Environment" -Name Path -ErrorAction SilentlyContinue).Path
    if ($cur -and ($cur -split ';') -contains $dir) { Set-ItemProperty "$HKU\Environment" -Name Path -Value (($cur -split ';' | Where-Object { $_ -and $_ -ne $dir }) -join ';') -Type ExpandString }
    $backup.installed = @($backup.installed | Where-Object { $_ -ne "path:$dir" }); Save-Backup
}
function Register-LLTask([string]$name, $action, $trigger, $principal, $settings) {
    if (-not (Get-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName $name -ErrorAction SilentlyContinue)) { [void]$script:newTasks.Add($name) }
    $p = @{ TaskPath = '\LogicalLunge\'; TaskName = $name; Action = $action; Principal = $principal; Settings = $settings; Force = $true }
    if ($trigger) { $p.Trigger = $trigger }
    Register-ScheduledTask @p | Out-Null
}
function Set-FocusColor([string]$text, [string]$hex) {
    if ($hex -notmatch '^#[0-9a-fA-F]{6}$') { return $text }
    $re = [regex]'(?m)^(\s*active_color:\s*")#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?(")'
    return $re.Replace($text, [Text.RegularExpressions.MatchEvaluator] { param($m) $m.Groups[1].Value + $hex.ToLower() + $m.Groups[2].Value + $m.Groups[3].Value }, 1)
}
function Get-FocusColor([string]$text) { $m = [regex]::Match($text, '(?m)^\s*active_color:\s*"(#[0-9a-fA-F]{6})'); if ($m.Success) { $m.Groups[1].Value } else { $null } }
function Stop-Parts {
    foreach ($n in 'lunge', 'lunge-tiling', 'lunge-tiling-watcher', 'lunge-shell', 'lunge-temps', 'glazewm', 'glazewm-watcher', 'zebar', 'll-helper', 'tacky-borders', 'll-temps') {
        Get-Process $n -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    }
}

# Puts back what this run changed and starts the previous desktop
function Undo-Install {
    Log ''; Log '==> Rolling back'
    Progress 'rollback' $null
    Stop-Parts
    Start-Sleep -Milliseconds 300
    if ($script:ownedSaved) {
        foreach ($rel in $OWNED) { $p = Join-Path $APP $rel; if (Test-Path $p) { Remove-Item $p -Recurse -Force -ErrorAction SilentlyContinue } }
        foreach ($rel in $OWNED) {
            $src = Join-Path "$RB\app" $rel
            if (Test-Path $src) { $dst = Join-Path $APP $rel; New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null; try { Move-Item $src $dst -Force } catch { Log "    could not restore $rel : $($_.Exception.Message)" } }
        }
        Log '    previous app files restored'
    }
    for ($i = $script:moves.Count - 1; $i -ge 0; $i--) {
        $mv = $script:moves[$i]
        try { if (Test-Path $mv[1]) { Move-Item $mv[1] $mv[0] -Force } } catch { Log "    could not move back $($mv[1]): $($_.Exception.Message)" }
    }
    # a first install leaves nothing behind (the folders moved from 0.1.x went back above)
    if (-not $appExisted) { Remove-Item $APP -Recurse -Force -ErrorAction SilentlyContinue }
    if ($script:configSaved -and (Test-Path "$RB\config.yaml")) { Copy-Item "$RB\config.yaml" (Join-Path $CONF 'config.yaml') -Force; Log '    previous config restored' }
    for ($i = $script:created.Count - 1; $i -ge 0; $i--) { Remove-Item $script:created[$i] -Recurse -Force -ErrorAction SilentlyContinue }
    for ($i = $script:runReg.Count - 1; $i -ge 0; $i--) {
        $r = $script:runReg[$i]
        try {
            if ($r.existed) { Set-ItemProperty -Path $r.path -Name $r.name -Value $r.old -Type $r.type }
            else { Remove-ItemProperty -Path $r.path -Name $r.name -ErrorAction SilentlyContinue }
        }
        catch { Log "    registry $($r.path)\$($r.name): $($_.Exception.Message)" }
    }
    if ($script:runReg.Count) { Log "    $($script:runReg.Count) Windows settings restored" }
    foreach ($t in $script:newTasks) { Unregister-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName $t -Confirm:$false -ErrorAction SilentlyContinue }
    if ($script:runReg.Count -or $script:explorerRestarted) { Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue; Start-Sleep 2 }
    foreach ($m in (Join-Path $STATE 'maintenance'), (Join-Path $OLD_STATE 'maintenance')) { Remove-Item $m -Force -ErrorAction SilentlyContinue }
    # the previous desktop
    if ($appExisted) { Start-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Start' -ErrorAction SilentlyContinue }
    elseif ($legacy) { Start-ScheduledTask -TaskPath '\LL\' -TaskName 'GlazeWM' -ErrorAction SilentlyContinue }
    # Explorer starts it as the user (this installer is elevated)
    elseif ($perUser) { Start-Process explorer.exe "`"$(Join-Path $OLD_APP 'lunge.exe')`"" -ErrorAction SilentlyContinue }
    Log '    rollback finished'
}

Log "Logical Lunge installer - $(Get-Date)"
Log "user: $UserName ($UserSid)  profile: $UserProfile"
Log "source: $Source  existing install: $appExisted  0.1.x install: $legacy"
$ok = $false; $cancelled = $false; $failure = $null
try {
    # ------------------------------------------------------------ checks
    Step 'check' 'Checking Windows'
    if (-not (Test-Path (Join-Path $Source 'app\lunge.exe'))) { throw "The package is incomplete: app\lunge.exe is missing in $Source." }
    $build = [int](Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuildNumber
    if ($build -lt 19041) { throw "Windows 10 2004 (build 19041) or newer is required; this is build $build." }
    if (-not [Environment]::Is64BitOperatingSystem) { throw '64-bit Windows is required.' }
    $win11 = $build -ge 22000
    Log "    Windows build $build ($(if ($win11) { 'Windows 11' } else { 'Windows 10' }))"
    $choice = $null
    if ($Choices -and (Test-Path $Choices)) { $choice = Get-Content $Choices -Raw | ConvertFrom-Json; Log "    choices: $(Get-Content $Choices -Raw)" }

    Step 'runtimes' 'Checking the WebView2 and Visual C++ runtimes'
    $wv2 = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
    if (-not $wv2 -or -not $wv2.pv -or $wv2.pv -eq '0.0.0.0') {
        Log '    installing Microsoft Edge WebView2 runtime (needed by the shell)'
        $b = Get-File 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' 'MicrosoftEdgeWebview2Setup.exe'
        Start-Process $b -ArgumentList '/silent', '/install' -Wait
    }
    # The window manager and the shell (Rust, MSVC) need the Visual C++ 2015-2022 runtime
    if (-not (Test-Path "$env:WINDIR\System32\vcruntime140_1.dll")) {
        Log '    installing Microsoft Visual C++ runtime'
        $vc = Get-File 'https://aka.ms/vs/17/release/vc_redist.x64.exe' 'vc_redist.x64.exe'
        Start-Process $vc -ArgumentList '/install', '/quiet', '/norestart' -Wait
    }

    # ------------------------------------------------------------ stop running parts
    Step 'stop' 'Stopping the desktop'
    # Normally already stopped gracefully as the user (lunge.exe --stop-desktop); this catches anything left
    Set-Content (Join-Path $STATE 'maintenance') (Get-Date -Format o)
    Stop-Parts
    # Older versions hid the taskbar with a PowerShell loop
    Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" -ErrorAction SilentlyContinue | Where-Object { $_.CommandLine -like '*hide-taskbar.ps1*' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 800

    # ------------------------------------------------------------ app files
    Step 'files' 'Copying Logical Lunge'
    if (Test-Path $RB) { Remove-Item $RB -Recurse -Force }
    foreach ($rel in $OWNED) {
        $src = Join-Path $APP $rel
        if (Test-Path $src) { $dst = Join-Path "$RB\app" $rel; New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null; Move-Item $src $dst }
    }
    $script:ownedSaved = $true
    Copy-Item (Join-Path $Source 'app\*') $APP -Recurse -Force
    # Super menu app list (generated on first run, kept across updates): it lives in the user's data folder now; older
    # versions kept it next to the widgets
    $appsList = Join-Path $STATE 'apps.json'
    if (-not (Test-Path $appsList)) {
        foreach ($old in (Join-Path "$RB\app" 'ui\logical-lunge\apps.json'), (Join-Path $OLD_APP 'ui\logical-lunge\apps.json'), (Join-Path $OLD_ZB 'logical-lunge\apps.json')) {
            if (Test-Path $old) { Remember-Created $appsList; Copy-Item $old $appsList; break }
        }
    }
    # placeholders -> this user's paths
    $esc = $UserProfile.Replace('\', '\\'); $appEsc = $APP.Replace('\', '\\')
    Get-ChildItem $PACK -File -Include *.html, *.js, *.json, *.css -Recurse | ForEach-Object {
        $t = [IO.File]::ReadAllText($_.FullName)
        $n = $t.Replace('{{INSTALL_ESC}}', $appEsc).Replace('{{INSTALL}}', $APP).Replace('{{USERPROFILE_ESC}}', $esc).Replace('{{USERPROFILE}}', $UserProfile)
        if ($n -ne $t) { [IO.File]::WriteAllText($_.FullName, $n, $UTF8) }
    }
    # The shell starts only our widgets (no BOM: serde_json rejects it)
    $zsettings = [ordered]@{ startupConfigs = @(foreach ($w in 'bar', 'overview', 'sidebar-right', 'settings', 'toast', 'osk', 'update', 'session') { [ordered]@{ pack = 'logical-lunge'; widget = $w; preset = 'default' } }) }
    [IO.File]::WriteAllText((Join-Path $APP 'ui\settings.json'), ($zsettings | ConvertTo-Json -Depth 5), $UTF8)

    # ------------------------------------------------------------ settings (config, keybinds, prefs)
    Step 'config' 'Writing the settings'
    $cfg = Join-Path $CONF 'config.yaml'
    $tpl = [IO.File]::ReadAllText((Join-Path $Source 'config\config.yaml'))
    $hashFile = Join-Path $STATE 'config.sha256'
    $focus = if ($choice -and $choice.focusColor) { [string]$choice.focusColor } else { $null }
    if (Test-Path $cfg) {
        $cur = [IO.File]::ReadAllText($cfg)
        New-Item -ItemType Directory -Force $RB | Out-Null
        Copy-Item $cfg "$RB\config.yaml" -Force; $script:configSaved = $true
        $unchanged = (Test-Path $hashFile) -and ((Get-Content $hashFile -Raw).Trim() -eq (Hash $cur))
        if ($unchanged) {
            # the config we wrote last time, untouched: the new one replaces it (the focus color is kept)
            if (-not $focus) { $focus = Get-FocusColor $cur }
            $new = Set-FocusColor $tpl $focus
            [IO.File]::WriteAllText($cfg, $new, $UTF8); Set-Content $hashFile (Hash $new)
            Log '    config.yaml updated'
        }
        else {
            # edited by the user: kept; the new default is next to it for comparison
            if ($focus) { [IO.File]::WriteAllText($cfg, (Set-FocusColor $cur $focus), $UTF8) }
            [IO.File]::WriteAllText((Join-Path $CONF 'config.default.yaml'), $tpl, $UTF8)
            Log '    config.yaml was edited by the user: kept (the new default is config.default.yaml)'
        }
    }
    else {
        Remember-Created $cfg
        $oldCfg = Join-Path $OLD_GW 'config.yaml'
        if (-not $focus -and (Test-Path $oldCfg)) { $focus = Get-FocusColor ([IO.File]::ReadAllText($oldCfg)) }
        $new = Set-FocusColor $tpl $focus
        [IO.File]::WriteAllText($cfg, $new, $UTF8); Set-Content $hashFile (Hash $new)
        Log "    config.yaml written$(if ($focus) { " (focus color $focus)" })"
    }
    # keyboard shortcuts of the core (0.1.x kept them in %LOCALAPPDATA%\logical-lunge)
    $kb = Join-Path $CONF 'keybinds.json'
    if (-not (Test-Path $kb) -and (Test-Path (Join-Path $OLD_STATE 'keybinds.json'))) { Remember-Created $kb; Copy-Item (Join-Path $OLD_STATE 'keybinds.json') $kb }
    # interface preferences chosen in the installer (language, clock); the UI reads a copy next to the widgets
    $prefs = Join-Path $CONF 'prefs.json'
    if ($choice -and ($choice.language -or $choice.clock)) {
        $p = [ordered]@{ language = $(if ($choice.language) { [string]$choice.language } else { 'system' }); clock = $(if ($choice.clock) { [string]$choice.clock } else { '24' }) }
        if (-not (Test-Path $prefs)) { Remember-Created $prefs }
        [IO.File]::WriteAllText($prefs, ($p | ConvertTo-Json), $UTF8)
    }
    if (Test-Path $prefs) { Copy-Item $prefs (Join-Path $PACK 'prefs.json') -Force }

    # ------------------------------------------------------------ data of 0.1.x
    if ($legacy -or $perUser -or (Test-Path $OLD_STATE)) {
        Step 'migrate' 'Moving data from the previous version'
        foreach ($pair in @(@('nightlight', 'nightlight'), @('gamma', 'gamma'), @('wallpaper.txt', 'wallpaper.txt'), @('splash-wall.jpg', 'splash-wall.jpg'), @('glazewm-keybindings.default.json', 'tiling-keybindings.default.json'))) {
            $from = Join-Path $OLD_STATE $pair[0]; $to = Join-Path $STATE $pair[1]
            if ((Test-Path $from) -and -not (Test-Path $to)) { Remember-Created $to; Copy-Item $from $to }
        }
        $clip = Join-Path $DATA 'clipboard'
        if ((Test-Path (Join-Path $OLD_STATE 'clipboard')) -and -not (Test-Path $clip)) { Remember-Created $clip; Copy-Item (Join-Path $OLD_STATE 'clipboard') $clip -Recurse }
        # widget storage (to-dos, pinned apps, theme...): only what the widgets saved, not the browser caches
        $webDst = Join-Path $DATA 'webview\logical-lunge\EBWebView\Default'
        $webSrc = Join-Path $OLD_WEB 'EBWebView\Default'
        if ((Test-Path $webSrc) -and -not (Test-Path (Join-Path $webDst 'Local Storage'))) {
            New-Item -ItemType Directory -Force $webDst | Out-Null
            foreach ($d in 'Local Storage', 'IndexedDB', 'WebStorage') {
                if (Test-Path (Join-Path $webSrc $d)) { Remember-Created (Join-Path $webDst $d); Copy-Item (Join-Path $webSrc $d) (Join-Path $webDst $d) -Recurse }
            }
            Log '    widget storage moved'
        }
        # big downloaded tools: moved instead of downloaded again
        foreach ($old in $OLD_APP, $OLD_LL) {
            foreach ($d in 'tools\wezterm', 'tools\bin') {
                $from = Join-Path $old $d; $to = Join-Path $APP $d
                if ((Test-Path $from) -and -not (Test-Path $to)) {
                    try { Move-Tracked $from $to; Log "    $d moved" }
                    catch { Remember-Created $to; Copy-Item $from $to -Recurse -Force; Log "    $d copied (in use: $($_.Exception.Message))" }
                }
            }
        }
    }
    else { $script:stepNo++ }

    # ------------------------------------------------------------ tools
    Step 'tools' 'Installing the brightness and sensor tools'
    $cmm = Join-Path $APP 'tools\ControlMyMonitor.exe'
    if (-not (Test-Path $cmm)) {
        $cz = Get-File 'https://www.nirsoft.net/utils/controlmymonitor.zip' 'controlmymonitor.zip'
        Expand-Archive $cz (Join-Path $DL 'cmm') -Force
        Remember-Created $cmm
        Copy-Item (Join-Path $DL 'cmm\ControlMyMonitor.exe') $cmm -Force
    }
    $temps = Join-Path $APP 'tools\temps'
    if (-not (Test-Path (Join-Path $temps 'LibreHardwareMonitorLib.dll'))) {
        $lz = Gh-Asset 'LibreHardwareMonitor/LibreHardwareMonitor' $LHM_VER '^LibreHardwareMonitor\.zip$'
        $tmp = Join-Path $DL 'lhm'; Expand-Archive $lz $tmp -Force
        Get-ChildItem $tmp -Filter *.dll | ForEach-Object { Remember-Created (Join-Path $temps $_.Name); Copy-Item $_.FullName $temps -Force }
    }
    if (-not $NoSensors -and -not (Get-Service PawnIO -ErrorAction SilentlyContinue)) {
        $pw = Gh-Asset 'namazso/PawnIO.Setup' $PAWNIO_VER 'PawnIO_setup\.exe$'
        Copy-Item $pw (Join-Path $temps 'PawnIO_setup.exe') -Force
        Start-Process $pw -ArgumentList '-install', '-silent' -Wait
        Mark-Installed 'pawnio'
    }

    # ------------------------------------------------------------ terminal
    if (-not $NoTerminal) {
        Step 'terminal' 'Installing the terminal (WezTerm, fish, starship)'
        $wdst = Join-Path $APP 'tools\wezterm'
        if (-not (Test-Path (Join-Path $wdst 'wezterm-gui.exe'))) {
            $wz = Get-File 'https://github.com/wezterm/wezterm/releases/download/nightly/WezTerm-windows-nightly.zip' 'wezterm-nightly.zip'
            $tmp = Join-Path $DL 'wez'; Expand-Archive $wz $tmp -Force
            $inner = Get-ChildItem $tmp -Directory | Select-Object -First 1
            Remember-Created $wdst; New-Item -ItemType Directory -Force $wdst | Out-Null
            Copy-Item (Join-Path $inner.FullName '*') $wdst -Recurse -Force
        }
        $wl = Join-Path $UserProfile '.wezterm.lua'
        if ((Test-Path $wl) -and -not (Test-Path "$wl.before-ll")) { Copy-Item $wl "$wl.before-ll" }
        Copy-Item (Join-Path $Source 'config\wezterm\wezterm.lua') $wl -Force
        New-Item -ItemType Directory -Force (Join-Path $UserProfile '.config\wezterm') | Out-Null

        $fontsKey = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts'
        $wfonts = Join-Path $UserProfile '.config\wezterm\fonts'
        if (-not (Get-ChildItem $wfonts -Filter 'JetBrainsMonoNerdFont-*.ttf' -ErrorAction SilentlyContinue)) {
            $fz = Gh-Asset 'ryanoasis/nerd-fonts' $NERDFONT_VER '^JetBrainsMono\.zip$'
            $tmp = Join-Path $DL 'font'; Expand-Archive $fz $tmp -Force
            # WezTerm yalnız kendi font klasörüne bakar (sistemdeki yüzlerce fontu taramak açılışı 1.6 s yavaşlatıyordu)
            New-Item -ItemType Directory -Force $wfonts | Out-Null
            Get-ChildItem $tmp -Filter 'JetBrainsMonoNerdFont-*.ttf' | Copy-Item -Destination $wfonts -Force
            Get-ChildItem $tmp -Filter 'JetBrainsMonoNerdFont-*.ttf' | ForEach-Object {
                $dst = Join-Path $env:WINDIR "Fonts\$($_.Name)"
                if (-not (Test-Path $dst)) { Copy-Item $_.FullName $dst; Set-ItemProperty $fontsKey "$($_.BaseName) (TrueType)" $_.Name }
            }
            Mark-Installed 'fonts'
        }

        $msys = 'C:\msys64'
        if (-not (Test-Path "$msys\usr\bin\bash.exe")) {
            $sfx = Get-File 'https://github.com/msys2/msys2-installer/releases/latest/download/msys2-base-x86_64-latest.sfx.exe' 'msys2-base.sfx.exe'
            Start-Process $sfx -ArgumentList '-y', '-oC:\' -Wait
            & "$msys\usr\bin\bash.exe" -lc 'true' | Out-Null   # first run initialises keys
            Mark-Installed 'msys2'
        }
        if (-not (Test-Path "$msys\usr\bin\fish.exe")) {
            Assert-NotCancelled
            & "$msys\usr\bin\bash.exe" -lc 'pacman -Syu --noconfirm' 2>&1 | Out-Null
            & "$msys\usr\bin\bash.exe" -lc 'pacman -Syu --noconfirm' 2>&1 | Out-Null
            & "$msys\usr\bin\bash.exe" -lc 'pacman -S --noconfirm --needed fish' 2>&1 | Out-Null
        }
        $ns = "$msys\etc\nsswitch.conf"
        if (Test-Path $ns) { (Get-Content $ns) -replace '^db_home:.*$', 'db_home: windows' | Set-Content -Encoding ASCII $ns }
        $bin = Join-Path $APP 'tools\bin'
        if (-not (Test-Path $bin)) { Remember-Created $bin; New-Item -ItemType Directory -Force $bin | Out-Null }
        if (-not (Test-Path (Join-Path $bin 'starship.exe'))) { Expand-Archive (Gh-Asset 'starship/starship' $STARSHIP_VER 'starship-x86_64-pc-windows-msvc\.zip$') $bin -Force }
        if (-not (Test-Path (Join-Path $bin 'eza.exe'))) { Expand-Archive (Gh-Asset 'eza-community/eza' $EZA_VER 'eza\.exe_x86_64-pc-windows-gnu\.zip$') $bin -Force }
        if (-not (Test-Path (Join-Path $bin 'fzf.exe'))) { Expand-Archive (Gh-Asset 'junegunn/fzf' $FZF_VER 'fzf-.*-windows_amd64\.zip$') $bin -Force }   # themecolor seçicisi
        Add-UserPath $bin
        New-Item -ItemType Directory -Force (Join-Path $UserProfile '.config\fish\functions') | Out-Null
        Copy-Item (Join-Path $Source 'config\fish\functions\*.fish') (Join-Path $UserProfile '.config\fish\functions') -Force
        foreach ($pair in @(@('config\fish\config.fish', '.config\fish\config.fish'), @('config\starship.toml', '.config\starship.toml'))) {
            $dst = Join-Path $UserProfile $pair[1]
            New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null
            if ((Test-Path $dst) -and -not (Test-Path "$dst.before-ll")) { Copy-Item $dst "$dst.before-ll" }
            Copy-Item (Join-Path $Source $pair[0]) $dst -Force
        }
        Mark-Installed 'terminal'
    }
    else { $script:stepNo++ }

    # ------------------------------------------------------------ Windows settings (all backed up)
    Step 'windows' 'Applying Windows settings'
    $cu = "$HKU\Software\Microsoft\Windows\CurrentVersion"
    Set-Reg "$HKU\Control Panel\Desktop" 'WindowArrangementActive' '0' 'String'          # Aero Snap off (the WM tiles)
    Set-Reg "$HKU\Control Panel\Desktop" 'MouseWheelRouting' 2                           # scroll the window under the cursor
    Set-Reg "$HKU\Control Panel\Desktop\WindowMetrics" 'MinAnimate' '0' 'String'         # no minimize animation (we animate)
    Set-Reg "$cu\Explorer\Advanced" 'TaskbarAnimations' 0
    Set-Reg "$cu\Explorer\Advanced" 'HideIcons' 1                                          # clean desktop like Hyprland
    Set-Reg "$cu\Explorer\Advanced" 'SnapAssist' 0
    Set-Reg "$cu\Explorer\Advanced" 'EnableSnapAssistFlyout' 0
    Set-Reg "$cu\Explorer\Advanced" 'JointResize' 0
    Set-Reg "$cu\Explorer\Advanced" 'SnapFill' 0
    if ($win11) {
        Set-Reg "$cu\Explorer\Advanced" 'EnableSnapBar' 0                                   # Windows 11: snap layouts bar when dragging to the top
    }
    Set-Reg "$cu\Explorer\Advanced" 'DisabledHotkeys' 'CEFIJMTWX1234567890' 'String'      # Win+key shortcuts the shell owns
    Set-Reg "$cu\Explorer\Serialize" 'StartupDelayInMSec' 0                                # start the shell without the 10 s delay
    # Taskbar: auto-hide (the bar replaces it)
    $sr = "$cu\Explorer\StuckRects3"
    if (Test-Path $sr) {
        $s = (Get-ItemProperty $sr).Settings
        if ($s -and $s.Length -gt 8 -and $s[8] -ne 3) { $n = [byte[]]$s.Clone(); $n[8] = 3; Set-Reg $sr 'Settings' $n 'Binary' }
    }
    # No taskbar at all on the other monitors ("Show taskbar on all displays" off)
    Set-Reg "$cu\Explorer\Advanced" 'MMTaskbarEnabled' 0

    # ------------------------------------------------------------ scheduled tasks
    Step 'tasks' 'Creating the startup tasks'
    $principalUser = New-ScheduledTaskPrincipal -UserId $UserName -LogonType Interactive -RunLevel Limited
    $principalHigh = New-ScheduledTaskPrincipal -UserId $UserName -LogonType Interactive -RunLevel Highest
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $UserName
    $forever = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 4
    # The core is the root of the desktop and starts the window manager as its child. Both run elevated (highest
    # privileges, no prompt at sign-in): hotkeys and window management also work while an administrator window such as
    # Task Manager or an installer is focused. The shell and every program the user opens run as the normal user.
    Register-LLTask 'Start' (New-ScheduledTaskAction -Execute (Join-Path $APP 'lunge.exe') -WorkingDirectory $UserProfile) $trigger $principalHigh $forever
    if (-not $NoSensors) {
        $te = Join-Path $APP 'tools\temps\lunge-temps.exe'
        Register-LLTask 'Temps' (New-ScheduledTaskAction -Execute $te -WorkingDirectory (Split-Path $te)) $trigger $principalHigh (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 7)
    }
    $eth = Join-Path $APP 'scripts\eth.ps1'
    foreach ($pair in @(@('Ethernet-On', 'enable'), @('Ethernet-Off', 'disable'))) {
        $a = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$eth`" $($pair[1])"
        Register-LLTask $pair[0] $a $null $principalHigh (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 1))
    }
    # GlazeWM must not also start from an older Run entry (backed up: the uninstaller puts it back)
    $runOld = (Get-ItemProperty "$cu\Run" -Name 'GlazeWM' -ErrorAction SilentlyContinue).GlazeWM
    if ($null -ne $runOld) { Set-Reg "$cu\Run" 'GlazeWM' ([string]$runOld) 'String'; Remove-ItemProperty "$cu\Run" -Name 'GlazeWM' -ErrorAction SilentlyContinue }

    Step 'owner' 'Preparing the first start'
    # the user's settings and data belong to the user, not to the elevated installer (the app folder stays protected)
    foreach ($p in $DATA, $CONF) { & icacls $p /setowner $UserName /T /C /Q | Out-Null }
    Assert-NotCancelled
    $ok = $true
}
catch [OperationCanceledException] { $cancelled = $true; Log ''; Log 'Cancelled.' }
catch { $failure = $_; Log "ERROR: $($_.Exception.Message)"; Log "    at $($_.InvocationInfo.PositionMessage)" }
finally {
    # also runs on Ctrl+C: nothing is left half-installed
    if (-not $ok) { try { Undo-Install } catch { Log "rollback error: $($_.Exception.Message)" } }
}

if (-not $ok) {
    if ($cancelled) { Progress 'cancelled' $null; exit 2 }
    Progress 'error' @{ error = [string]$failure.Exception.Message }
    if (-not $ProgressFile) { Write-Host "Setup failed: $($failure.Exception.Message)" -ForegroundColor Red; Write-Host 'Press Enter to close.'; [void](Read-Host) }
    exit 1
}

# ---------------------------------------------------------------- committed: registration and clean-up
# Nothing below can undo the install; failures are only logged.
Step 'finish' 'Registering and cleaning up'
try {
    $un = "$HKU\Software\Microsoft\Windows\CurrentVersion\Uninstall\LogicalLunge"
    New-Item -Path $un -Force | Out-Null
    Set-ItemProperty $un 'DisplayName' 'Logical Lunge'
    Set-ItemProperty $un 'DisplayVersion' ([string]$backup.version)
    Set-ItemProperty $un 'Publisher' 'Logical Lunge'
    Set-ItemProperty $un 'DisplayIcon' (Join-Path $APP 'lunge.exe')
    Set-ItemProperty $un 'InstallLocation' $APP
    Set-ItemProperty $un 'UninstallString' "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$(Join-Path $APP 'uninstall.ps1')`""
    Set-ItemProperty $un 'NoModify' 1 -Type DWord
    Set-ItemProperty $un 'NoRepair' 1 -Type DWord
}
catch { Log "    uninstall entry: $($_.Exception.Message)" }
# Start menu: a way back without a command line when something is stuck (while the shell's bar is down, the Win key
# opens Windows' own Start menu)
try {
    $sm = Join-Path $UserProfile 'AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Logical Lunge'
    New-Item -ItemType Directory -Force $sm | Out-Null
    Get-ChildItem $sm -Filter *.lnk | Remove-Item -Force
    $name = if ((Get-UICulture).Name -like 'tr*') { "Logical Lunge'u yeniden başlat" } else { 'Restart Logical Lunge' }
    $lnk = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path $sm "$name.lnk"))
    $lnk.TargetPath = Join-Path $APP 'lunge.exe'
    $lnk.Arguments = '--restart-desktop'
    $lnk.WorkingDirectory = $UserProfile
    $lnk.IconLocation = (Join-Path $APP 'lunge.exe') + ',0'
    $lnk.Description = 'Restarts the Logical Lunge desktop cleanly'
    $lnk.Save()
}
catch { Log "    could not create the Start menu shortcut: $($_.Exception.Message)" }
Save-Backup

# (also retried on later updates if a folder was in use, e.g. a terminal running from the old location)
if ($legacy -or (Test-Path $OLD_STATE) -or (Test-Path $OLD_LL)) {
    Log '    removing the 0.1.x install'
    foreach ($t in 'GlazeWM', 'Splash', 'Temps', 'Ethernet-On', 'Ethernet-Off') { Unregister-ScheduledTask -TaskPath '\LL\' -TaskName $t -Confirm:$false -ErrorAction SilentlyContinue }
    try { $svc = New-Object -ComObject Schedule.Service; $svc.Connect(); $svc.GetFolder('\').DeleteFolder('LL', 0) } catch {}
    foreach ($d in (Join-Path $OLD_LL 'bin'), (Join-Path $OLD_LL 'tools\bin')) { Remove-UserPath $d }
    # Older versions installed the upstream GlazeWM / Zebar MSIs; only copies Logical Lunge installed are removed
    foreach ($app in @(@('glazewm', 'GlazeWM'), @('zebar', 'Zebar'))) {
        if (@($backup.installed) -notcontains $app[0]) { continue }
        foreach ($root in 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall') {
            Get-ChildItem $root -ErrorAction SilentlyContinue | ForEach-Object {
                $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
                if ($p.DisplayName -like "$($app[1])*" -and $_.PSChildName -match '^\{') {
                    Log "    removing the upstream $($app[1]) an older version installed"
                    Start-Process msiexec.exe -ArgumentList '/x', $_.PSChildName, '/qn', '/norestart' -Wait
                }
            }
        }
        $backup.installed = @($backup.installed | Where-Object { $_ -ne $app[0] })
    }
    Remove-UserPath (Join-Path $env:ProgramFiles 'glzr.io\Zebar')
    # the user's own configs from before Logical Lunge come back, ours go
    foreach ($f in (Join-Path $OLD_GW 'config.yaml'), (Join-Path $OLD_ZB 'settings.json')) {
        if (Test-Path "$f.before-ll") { Move-Item "$f.before-ll" $f -Force } else { Remove-Item $f -Force -ErrorAction SilentlyContinue }
    }
    foreach ($d in $OLD_LL, (Join-Path $OLD_ZB 'logical-lunge'), $OLD_STATE, $OLD_WEB) {
        if (Test-Path $d) { try { Remove-Item $d -Recurse -Force } catch { Log "    could not remove $d (in use?): $($_.Exception.Message)" } }
    }
    foreach ($d in $OLD_GW, $OLD_ZB, (Split-Path $OLD_LL)) {
        if ((Test-Path $d) -and -not (Get-ChildItem $d -Force -ErrorAction SilentlyContinue | Where-Object { $_.Name -notmatch '\.(log|bak.*)$' })) { Remove-Item $d -Recurse -Force -ErrorAction SilentlyContinue }
    }
    Remove-Item (Join-Path $LOCAL 'Temp\ll-helper.log') -Force -ErrorAction SilentlyContinue
    Save-Backup
}
if ($perUser -or (Test-Path $OLD_APP)) {
    Log '    removing the per-user 0.2 install'
    Remove-UserPath (Join-Path $OLD_APP 'tools\bin')
    try { Remove-Item $OLD_APP -Recurse -Force } catch { Log "    could not remove $OLD_APP (in use?): $($_.Exception.Message)" }
}
if (Test-Path $RB) { Remove-Item $RB -Recurse -Force -ErrorAction SilentlyContinue }

Log 'Done. Starting the desktop...'
# taskbar auto-hide / DisabledHotkeys take effect after Explorer restarts (Windows restarts it by itself)
Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
Start-Sleep 2
Remove-Item (Join-Path $STATE 'maintenance') -Force -ErrorAction SilentlyContinue
Start-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Start'
if (-not $NoSensors) { Start-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Temps' -ErrorAction SilentlyContinue }
# a 0.1.x updater covered the screen with its own splash that waits for the old bar: the new desktop has its own
Start-Sleep -Milliseconds 1500
Get-Process ll-update-splash, ll-restart-splash -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Progress 'done' $null
exit 0
