# Logical Lunge - main installer (runs elevated, launched by install.ps1).
# Everything the desktop needs is installed and configured here with a single UAC prompt.
# Every Windows setting that is changed is backed up first so uninstall.ps1 can restore it.
param(
    [Parameter(Mandatory = $true)][string]$Source,      # extracted release folder
    [Parameter(Mandatory = $true)][string]$UserProfile, # profile of the user who ran install.ps1
    [Parameter(Mandatory = $true)][string]$UserSid,
    [Parameter(Mandatory = $true)][string]$UserName,    # DOMAIN\user
    [switch]$NoTerminal,                                # skip WezTerm + MSYS2 fish
    [switch]$NoSensors                                  # skip PawnIO driver (CPU temperature)
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# ---- pinned upstream versions (tested together) ----
$GLAZEWM_VER = 'v3.10.1'
$ZEBAR_VER = 'v3.3.1'
$TACKY_VER = 'v1.6.0'
$LHM_VER = 'v0.9.6'
$PAWNIO_VER = '2.2.0'
$NERDFONT_VER = 'v3.5.1'
$STARSHIP_VER = 'v1.26.0'
$EZA_VER = 'v0.23.5'
$FZF_VER = 'v0.58.0'

$LL = Join-Path $UserProfile '.glzr\logical-lunge'
$ZB = Join-Path $UserProfile '.glzr\zebar'
$GW = Join-Path $UserProfile '.glzr\glazewm'
$STATE = Join-Path $UserProfile 'AppData\Local\logical-lunge'
$HKU = "Registry::HKEY_USERS\$UserSid"
$LOG = Join-Path $env:TEMP 'logical-lunge-install.log'
$DL = Join-Path $env:TEMP 'll-downloads'
New-Item -ItemType Directory -Force $LL, $ZB, $GW, $STATE, $DL | Out-Null

function Log([string]$m) { $line = (Get-Date -Format 'HH:mm:ss ') + $m; Add-Content -Path $LOG -Value $line; Write-Host $m }
trap { Log "ERROR: $($_.Exception.Message)"; Log "    at $($_.InvocationInfo.PositionMessage)"; Write-Host 'Press Enter to close.'; [void](Read-Host); exit 1 }
function Step([string]$m) { Log ''; Log "==> $m" }
function Get-File([string]$url, [string]$name) {
    $dst = Join-Path $DL $name
    if (-not (Test-Path $dst)) { Log "    download $url"; Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile "$dst.part"; Move-Item "$dst.part" $dst -Force }
    return $dst
}
function Gh-Asset([string]$repo, [string]$tag, [string]$pattern) {
    $rel = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$repo/releases/tags/$tag"
    $a = $rel.assets | Where-Object { $_.name -match $pattern } | Select-Object -First 1
    if (-not $a) { throw "asset '$pattern' not found in $repo $tag" }
    return Get-File $a.browser_download_url $a.name
}

# ---- backup of every setting we touch (restored by uninstall.ps1) ----
$backupFile = Join-Path $STATE 'install-backup.json'
$backup = @{ registry = @(); installed = @(); version = (Get-Content (Join-Path $Source 'VERSION') -ErrorAction SilentlyContinue) }
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
    Set-ItemProperty -Path $path -Name $name -Value $value -Type $type
}
function Mark-Installed([string]$what) { if ($backup.installed -notcontains $what) { $backup.installed += $what; Save-Backup } }
# PATH: only our own entries are added (and later removed) - never restore the whole value
function Add-UserPath([string]$dir) {
    $cur = (Get-ItemProperty "$HKU\Environment" -Name Path -ErrorAction SilentlyContinue).Path
    if ($cur -and ($cur -split ';') -contains $dir) { return }
    $new = ((@($cur -split ';' | Where-Object { $_ }) + $dir) -join ';')
    Set-ItemProperty "$HKU\Environment" -Name Path -Value $new -Type ExpandString
    Mark-Installed ("path:" + $dir)
}

Log "Logical Lunge installer - $(Get-Date)"
Log "user: $UserName ($UserSid)  profile: $UserProfile"

# ---------------------------------------------------------------- checks
Step 'Checking Windows'
$os = [Environment]::OSVersion.Version
$build = [int](Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuildNumber
if ($build -lt 19041) { throw "Windows 10 2004 (build 19041) or newer is required; this is build $build." }
if (-not [Environment]::Is64BitOperatingSystem) { throw '64-bit Windows is required.' }
$win11 = $build -ge 22000
Log "    Windows build $build ($(if ($win11) { 'Windows 11' } else { 'Windows 10' }))"
$wv2 = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
if (-not $wv2 -or -not $wv2.pv -or $wv2.pv -eq '0.0.0.0') {
    Step 'Installing Microsoft Edge WebView2 runtime (needed by the shell)'
    $b = Get-File 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' 'MicrosoftEdgeWebview2Setup.exe'
    Start-Process $b -ArgumentList '/silent', '/install' -Wait
}
# GlazeWM / Zebar (Rust, MSVC) need the Visual C++ 2015-2022 runtime, which a clean Windows may not have.
if (-not (Test-Path "$env:WINDIR\System32\vcruntime140_1.dll")) {
    Step 'Installing Microsoft Visual C++ runtime'
    $vc = Get-File 'https://aka.ms/vs/17/release/vc_redist.x64.exe' 'vc_redist.x64.exe'
    Start-Process $vc -ArgumentList '/install', '/quiet', '/norestart' -Wait
}

# ---------------------------------------------------------------- stop running parts
Step 'Stopping running components'
foreach ($n in 'glazewm', 'zebar', 'll-helper', 'tacky-borders', 'll-temps') { Get-Process $n -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 800

# ---------------------------------------------------------------- GlazeWM + Zebar
# The package normally carries our own GlazeWM/Zebar builds (logical-lunge\bin); upstream MSIs are the fallback.
$bundled = Test-Path (Join-Path $Source 'logical-lunge\bin\glazewm.exe')
if ($bundled) {
    Step 'Using the bundled GlazeWM and Zebar (Logical Lunge builds)'
    New-Item -ItemType Directory -Force (Join-Path $LL 'bin') | Out-Null
    Copy-Item (Join-Path $Source 'logical-lunge\bin\*') (Join-Path $LL 'bin') -Force
    $gwExe = Join-Path $LL 'bin\glazewm.exe'
    $zbExe = Join-Path $LL 'bin\zebar.exe'
    Add-UserPath (Join-Path $LL 'bin')
}
else {
Step "Installing GlazeWM $GLAZEWM_VER"
$gwExe = Join-Path $env:ProgramFiles 'glzr.io\GlazeWM\glazewm.exe'
if (-not (Test-Path $gwExe)) {
    $msi = Gh-Asset 'glzr-io/glazewm' $GLAZEWM_VER 'standalone-glazewm-.*-x64\.msi$'
    Start-Process msiexec.exe -ArgumentList '/i', "`"$msi`"", '/qn', '/norestart' -Wait
    Mark-Installed 'glazewm'
}
if (-not (Test-Path $gwExe)) { $gwExe = (Get-ChildItem "$env:ProgramFiles\glzr.io" -Recurse -Filter glazewm.exe -ErrorAction SilentlyContinue | Select-Object -First 1).FullName }
if (-not $gwExe) { throw 'GlazeWM could not be installed.' }
Log "    $gwExe"

Step "Installing Zebar $ZEBAR_VER"
$zbExe = Join-Path $env:ProgramFiles 'glzr.io\Zebar\zebar.exe'
if (-not (Test-Path $zbExe)) {
    $msi = Gh-Asset 'glzr-io/zebar' $ZEBAR_VER 'zebar-.*-x64\.msi$'
    Start-Process msiexec.exe -ArgumentList '/i', "`"$msi`"", '/qn', '/norestart' -Wait
    Mark-Installed 'zebar'
}
if (-not (Test-Path $zbExe)) { throw 'Zebar could not be installed.' }
# GlazeWM's shell-exec cannot run quoted paths with spaces: make "zebar" resolvable through PATH
Add-UserPath (Split-Path $zbExe)
}

# ---------------------------------------------------------------- files
Step 'Copying Logical Lunge files'
Copy-Item (Join-Path $Source 'logical-lunge\*') $LL -Recurse -Force
# installed version (the update button compares it with the latest release)
Copy-Item (Join-Path $Source 'VERSION') (Join-Path $LL 'VERSION') -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $ZB 'logical-lunge') | Out-Null
Copy-Item (Join-Path $Source 'shell\*') (Join-Path $ZB 'logical-lunge') -Recurse -Force
# placeholders -> this user's profile path
$esc = $UserProfile.Replace('\', '\\')
Get-ChildItem (Join-Path $ZB 'logical-lunge') -File -Include *.html, *.json, *.css -Recurse | ForEach-Object {
    $t = [IO.File]::ReadAllText($_.FullName)
    $n = $t.Replace('{{USERPROFILE_ESC}}', $esc).Replace('{{USERPROFILE}}', $UserProfile)
    if ($n -ne $t) { [IO.File]::WriteAllText($_.FullName, $n, (New-Object Text.UTF8Encoding $false)) }
}
# Zebar: start only our widgets
$zsettings = [ordered]@{ '$schema' = "https://github.com/glzr-io/zebar/raw/$ZEBAR_VER/resources/settings-schema.json"; startupConfigs = @(
        foreach ($w in 'bar', 'overview', 'sidebar-right', 'toast', 'osk', 'update', 'session') { [ordered]@{ pack = 'logical-lunge'; widget = $w; preset = 'default' } }) }
$zs = Join-Path $ZB 'settings.json'
if ((Test-Path $zs) -and -not (Test-Path "$zs.before-ll")) { Copy-Item $zs "$zs.before-ll" }
# No BOM: Zebar parses settings.json with serde_json, which rejects the BOM that Set-Content -Encoding UTF8 writes
# in Windows PowerShell 5.1 (Zebar then fails at startup and there is no bar).
[IO.File]::WriteAllText($zs, ($zsettings | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
# GlazeWM config (the user's own config is kept as a backup)
$gc = Join-Path $GW 'config.yaml'
if ((Test-Path $gc) -and -not (Test-Path "$gc.before-ll")) { Copy-Item $gc "$gc.before-ll" }
Copy-Item (Join-Path $Source 'config\glazewm\config.yaml') $gc -Force
# Start Zebar by its full path: Task Scheduler may not see the folder this installer just added to PATH until the
# user signs out and in again, so "shell-exec zebar" would not be found. shell-exec resolves unquoted paths with spaces.
$zbCmd = $zbExe
if ($zbExe.StartsWith($UserProfile + '\', [StringComparison]::OrdinalIgnoreCase)) { $zbCmd = '%USERPROFILE%' + $zbExe.Substring($UserProfile.Length) }
$gcText = [IO.File]::ReadAllText($gc)
[IO.File]::WriteAllText($gc, $gcText.Replace("'shell-exec zebar'", "'shell-exec $zbCmd'"), (New-Object Text.UTF8Encoding $false))
# tacky-borders (rounded purple focus border)
$tb = Join-Path $UserProfile '.config\tacky-borders'
New-Item -ItemType Directory -Force $tb | Out-Null
Copy-Item (Join-Path $Source 'config\tacky-borders\config.yaml') (Join-Path $tb 'config.yaml') -Force

Step "Installing tacky-borders $TACKY_VER"
$tz = Gh-Asset 'lukeyou05/tacky-borders' $TACKY_VER 'tacky-borders-.*\.zip$'
$tmp = Join-Path $DL 'tacky'; Expand-Archive $tz $tmp -Force
Copy-Item (Get-ChildItem $tmp -Recurse -Filter tacky-borders.exe | Select-Object -First 1).FullName (Join-Path $LL 'tools\tacky-borders.exe') -Force

Step 'Installing DDC/CI brightness tool (NirSoft ControlMyMonitor)'
$cz = Get-File 'https://www.nirsoft.net/utils/controlmymonitor.zip' 'controlmymonitor.zip'
Expand-Archive $cz (Join-Path $DL 'cmm') -Force
Copy-Item (Join-Path $DL 'cmm\ControlMyMonitor.exe') (Join-Path $LL 'tools\ControlMyMonitor.exe') -Force

Step "Installing hardware sensors (LibreHardwareMonitor $LHM_VER)"
$lz = Gh-Asset 'LibreHardwareMonitor/LibreHardwareMonitor' $LHM_VER '^LibreHardwareMonitor\.zip$'
$lhm = Join-Path $LL 'tools\lhm'; New-Item -ItemType Directory -Force $lhm | Out-Null
$tmp = Join-Path $DL 'lhm'; Expand-Archive $lz $tmp -Force
Get-ChildItem $tmp -Filter *.dll | Copy-Item -Destination $lhm -Force
if (-not $NoSensors) {
    if (-not (Get-Service PawnIO -ErrorAction SilentlyContinue)) {
        $pw = Gh-Asset 'namazso/PawnIO.Setup' $PAWNIO_VER 'PawnIO_setup\.exe$'
        Copy-Item $pw (Join-Path $lhm 'PawnIO_setup.exe') -Force
        Start-Process $pw -ArgumentList '-install', '-silent' -Wait
        Mark-Installed 'pawnio'
    }
}

# ---------------------------------------------------------------- terminal
if (-not $NoTerminal) {
    Step 'Installing WezTerm (nightly) as the terminal'
    $wz = Get-File 'https://github.com/wezterm/wezterm/releases/download/nightly/WezTerm-windows-nightly.zip' 'wezterm-nightly.zip'
    $tmp = Join-Path $DL 'wez'; Expand-Archive $wz $tmp -Force
    $inner = Get-ChildItem $tmp -Directory | Select-Object -First 1
    $wdst = Join-Path $LL 'tools\wezterm'; New-Item -ItemType Directory -Force $wdst | Out-Null
    Copy-Item (Join-Path $inner.FullName '*') $wdst -Recurse -Force
    $wl = Join-Path $UserProfile '.wezterm.lua'
    if ((Test-Path $wl) -and -not (Test-Path "$wl.before-ll")) { Copy-Item $wl "$wl.before-ll" }
    Copy-Item (Join-Path $Source 'config\wezterm\wezterm.lua') $wl -Force
    New-Item -ItemType Directory -Force (Join-Path $UserProfile '.config\wezterm') | Out-Null

    Step "Installing JetBrainsMono Nerd Font $NERDFONT_VER"
    $fz = Gh-Asset 'ryanoasis/nerd-fonts' $NERDFONT_VER '^JetBrainsMono\.zip$'
    $tmp = Join-Path $DL 'font'; Expand-Archive $fz $tmp -Force
    $fontsKey = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts'
    # WezTerm yalnız kendi font klasörüne bakar (sistemdeki yüzlerce fontu taramak açılışı 1.6 s yavaşlatıyordu)
    $wfonts = Join-Path $UserProfile '.config\wezterm\fonts'; New-Item -ItemType Directory -Force $wfonts | Out-Null
    Get-ChildItem $tmp -Filter 'JetBrainsMonoNerdFont-*.ttf' | Copy-Item -Destination $wfonts -Force
    Get-ChildItem $tmp -Filter 'JetBrainsMonoNerdFont-*.ttf' | ForEach-Object {
        $dst = Join-Path $env:WINDIR "Fonts\$($_.Name)"
        if (-not (Test-Path $dst)) { Copy-Item $_.FullName $dst; Set-ItemProperty $fontsKey "$($_.BaseName) (TrueType)" $_.Name }
    }
    Mark-Installed 'fonts'

    Step 'Installing fish shell (MSYS2), starship and eza'
    $msys = 'C:\msys64'
    if (-not (Test-Path "$msys\usr\bin\bash.exe")) {
        $sfx = Get-File 'https://github.com/msys2/msys2-installer/releases/latest/download/msys2-base-x86_64-latest.sfx.exe' 'msys2-base.sfx.exe'
        Start-Process $sfx -ArgumentList '-y', '-oC:\' -Wait
        & "$msys\usr\bin\bash.exe" -lc 'true' | Out-Null   # first run initialises keys
        Mark-Installed 'msys2'
    }
    & "$msys\usr\bin\bash.exe" -lc 'pacman -Syu --noconfirm' 2>&1 | Out-Null
    & "$msys\usr\bin\bash.exe" -lc 'pacman -Syu --noconfirm' 2>&1 | Out-Null
    & "$msys\usr\bin\bash.exe" -lc 'pacman -S --noconfirm --needed fish' 2>&1 | Out-Null
    $ns = "$msys\etc\nsswitch.conf"
    if (Test-Path $ns) { (Get-Content $ns) -replace '^db_home:.*$', 'db_home: windows' | Set-Content -Encoding ASCII $ns }
    $bin = Join-Path $LL 'tools\bin'; New-Item -ItemType Directory -Force $bin | Out-Null
    $sz = Gh-Asset 'starship/starship' $STARSHIP_VER 'starship-x86_64-pc-windows-msvc\.zip$'
    Expand-Archive $sz $bin -Force
    $ez = Gh-Asset 'eza-community/eza' $EZA_VER 'eza\.exe_x86_64-pc-windows-gnu\.zip$'
    Expand-Archive $ez $bin -Force
    Add-UserPath $bin
    $fz = Gh-Asset 'junegunn/fzf' $FZF_VER 'fzf-.*-windows_amd64\.zip$'   # themecolor seçicisi
    Expand-Archive $fz $bin -Force
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

# ---------------------------------------------------------------- Windows settings (all backed up)
Step 'Applying Windows settings'
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
    Set-Reg "$cu\Explorer\Advanced" 'EnableSnapAssistFlyout' 0                          # Windows 11: snap layouts on the maximize button
}
Set-Reg "$cu\Explorer\Advanced" 'DisabledHotkeys' 'CEFIJMTWX1234567890' 'String'      # Win+key shortcuts the shell owns
Set-Reg "$cu\Explorer\Serialize" 'StartupDelayInMSec' 0                                # start the shell without the 10 s delay
# Taskbar: auto-hide (the bar replaces it)
$sr = "$cu\Explorer\StuckRects3"
if (Test-Path $sr) {
    $s = (Get-ItemProperty $sr).Settings
    if ($s -and $s.Length -gt 8 -and $s[8] -ne 3) { $n = [byte[]]$s.Clone(); $n[8] = 3; Set-Reg $sr 'Settings' $n 'Binary' }
}
# Old conflicting startup entries are not touched; only our own autostart is added (scheduled task below).

# ---------------------------------------------------------------- scheduled tasks
Step 'Creating startup tasks'
$principalUser = New-ScheduledTaskPrincipal -UserId $UserName -LogonType Interactive -RunLevel Limited
$principalHigh = New-ScheduledTaskPrincipal -UserId $UserName -LogonType Interactive -RunLevel Highest
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $UserName
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 4
$helper = Join-Path $LL 'helper\ll-helper.exe'
Register-ScheduledTask -TaskPath '\LL\' -TaskName 'GlazeWM' -Action (New-ScheduledTaskAction -Execute $gwExe) -Trigger $trigger -Principal $principalUser -Settings $settings -Force | Out-Null
Register-ScheduledTask -TaskPath '\LL\' -TaskName 'Splash' -Action (New-ScheduledTaskAction -Execute $helper -Argument '--splash' -WorkingDirectory (Split-Path $helper)) -Trigger $trigger -Principal $principalUser -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 2) -Priority 4) -Force | Out-Null
if (-not $NoSensors) {
    $temps = Join-Path $LL 'tools\lhm\ll-temps.exe'
    Register-ScheduledTask -TaskPath '\LL\' -TaskName 'Temps' -Action (New-ScheduledTaskAction -Execute $temps -WorkingDirectory (Split-Path $temps)) -Trigger $trigger -Principal $principalHigh -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 7) -Force | Out-Null
}
$eth = Join-Path $LL 'scripts\eth.ps1'
foreach ($pair in @(@('Ethernet-On', 'enable'), @('Ethernet-Off', 'disable'))) {
    $a = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$eth`" $($pair[1])"
    Register-ScheduledTask -TaskPath '\LL\' -TaskName $pair[0] -Action $a -Principal $principalHigh -Settings (New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 1)) -Force | Out-Null
}
# GlazeWM must not also start from an older Run entry
Remove-ItemProperty "$cu\Run" -Name 'GlazeWM' -ErrorAction SilentlyContinue

# ---------------------------------------------------------------- uninstall entry (Apps & features)
Step 'Registering uninstaller'
Copy-Item (Join-Path $Source 'uninstall.ps1') (Join-Path $LL 'uninstall.ps1') -Force
$un = "$cu\Uninstall\LogicalLunge"
New-Item -Path $un -Force | Out-Null
Set-ItemProperty $un 'DisplayName' 'Logical Lunge'
Set-ItemProperty $un 'DisplayVersion' ([string]$backup.version)
Set-ItemProperty $un 'Publisher' 'Logical Lunge'
Set-ItemProperty $un 'DisplayIcon' $helper
Set-ItemProperty $un 'InstallLocation' $LL
Set-ItemProperty $un 'UninstallString' "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$(Join-Path $LL 'uninstall.ps1')`""
Set-ItemProperty $un 'NoModify' 1 -Type DWord
Set-ItemProperty $un 'NoRepair' 1 -Type DWord

# ---------------------------------------------------------------- first run
Step 'Preparing first run'
# the files belong to the user, not to the elevated installer
foreach ($p in $LL, (Join-Path $ZB 'logical-lunge'), $STATE) { & icacls $p /setowner $UserName /T /C /Q | Out-Null }
# taskbar auto-hide / DisabledHotkeys take effect after Explorer restarts (Windows restarts it by itself)
Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
Start-Sleep 2
Log 'Done. Starting the desktop...'
Save-Backup
Start-ScheduledTask -TaskPath '\LL\' -TaskName 'GlazeWM'
if (-not $NoSensors) { Start-ScheduledTask -TaskPath '\LL\' -TaskName 'Temps' -ErrorAction SilentlyContinue }
