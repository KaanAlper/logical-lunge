# Logical Lunge - uninstaller. Restores every Windows setting that the installer changed and removes
# everything it installed. Configuration files that existed before the install always come back.
# Logical Lunge's own settings and data (~\.config\logical-lunge, clipboard
# history, widget data, shortcut and night-light settings, downloaded wallpapers) are kept or removed:
#   -KeepConfig / -RemoveConfig, or a Yes/No/Cancel question when neither is given.
# Shared WezTerm/fish/starship configs without ownership evidence are kept; originals and later edits are preserved.
param([string]$UserProfile = $env:USERPROFILE, [string]$UserSid = '', [switch]$Elevated, [switch]$KeepConfig, [switch]$RemoveConfig)
$ErrorActionPreference = 'Continue'

$LOCAL = Join-Path $UserProfile 'AppData\Local'
$APP = Join-Path $env:ProgramFiles 'LogicalLunge'
$DATA = Join-Path $LOCAL 'LogicalLunge'
$STATE = Join-Path $DATA 'state'
$CONF = Join-Path $UserProfile '.config\logical-lunge'
. (Join-Path $PSScriptRoot 'scripts\uninstall-restore.ps1')
# Preflight intended deletion roots before even stopping the desktop. Each deletion rechecks its entire tree.
$APP = Assert-LLSafePath $APP $env:ProgramFiles
$DATA = Assert-LLSafePath $DATA $LOCAL
$STATE = Assert-LLSafePath $STATE $DATA
$CONF = Assert-LLSafePath $CONF (Join-Path $UserProfile '.config')
[void](Assert-LLSafePath (Join-Path $PSScriptRoot 'scripts\uninstall-restore.ps1') $PSScriptRoot)

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $UserSid) {
    if ([IO.Path]::GetFullPath($UserProfile).TrimEnd('\') -ne [IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\')) { throw 'Specify UserSid when uninstalling for another profile.' }
    $UserSid = $identity.User.Value
}
if ($UserSid -notmatch '^S-1-5-\d+(-\d+)+$') { throw 'Invalid target user SID.' }
$isAdmin = ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$sameUser = $UserSid -eq $identity.User.Value
$registeredProfile = (Get-ItemProperty -LiteralPath "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$UserSid" -ErrorAction SilentlyContinue).ProfileImagePath
if (-not $registeredProfile -or [IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($registeredProfile)).TrimEnd('\') -ne [IO.Path]::GetFullPath($UserProfile).TrimEnd('\')) { throw 'UserSid and UserProfile do not identify the same Windows profile.' }
$HKU = "Registry::HKEY_USERS\$UserSid"
if (-not (Test-Path -LiteralPath $HKU)) { throw 'The target user registry is not loaded. Run uninstall from that user session.' }
if (-not $sameUser -and -not $isAdmin) { throw 'Uninstall for another account requires administrator rights.' }
$sessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
# Capture before the core consumes its runtime records; fallback still needs the originals after a partial stop.
$recovery = Read-LLRestoreSnapshot $STATE

if (-not $KeepConfig -and -not $RemoveConfig) {
    $tr = (Get-UICulture).Name -like 'tr*'
    # Asked in Logical Lunge's own dialog while its desktop is up; Windows' box only when it is not running
    $askCore = Join-Path $env:ProgramFiles 'LogicalLunge\lunge.exe'
    $picked = 255
    if (Test-Path $askCore) {
        $title = if ($tr) { 'Logical Lunge kaldırılsın mı?' } else { 'Uninstall Logical Lunge?' }
        $body = if ($tr) { "Logical Lunge kaldırılacak ve Windows ayarları eski haline dönecek. Kendi ayarları ve verileri de silinsin mi? (ayar dosyaları, pano geçmişi, yapılacaklar, kısayol ve gece ışığı ayarları, indirilen duvar kağıtları)" }
                else { "Logical Lunge will be removed and your Windows settings restored. Also delete its own settings and data? (config files, clipboard history, to-dos, shortcut and night-light settings, downloaded wallpapers)" }
        $buttons = if ($tr) { 'Hepsini sil|Ayarları sakla|Kaldırma' } else { 'Delete everything|Keep my settings|Don''t uninstall' }
        & $askCore --ask --kind question --title $title --body $body --buttons $buttons --default 1 --cancel 2 | Out-Null
        $picked = $LASTEXITCODE
    }
    if ($picked -eq 2) { return }
    if ($picked -eq 0) { $RemoveConfig = $true }
    elseif ($picked -eq 1) { $KeepConfig = $true }
}
if (-not $KeepConfig -and -not $RemoveConfig) {
    Add-Type -AssemblyName System.Windows.Forms
    $text = if ($tr) { "Logical Lunge kaldırılacak ve Windows ayarları eski haline dönecek.`n`nLogical Lunge'ın kendi ayarları ve verileri de silinsin mi? (ayar dosyaları, pano geçmişi, yapılacaklar, kısayol ve gece ışığı ayarları, indirilen duvar kağıtları)`n`nEvet: hepsini sil`nHayır: ayarları sakla`nİptal: kaldırma" }
            else { "Logical Lunge will be removed and your Windows settings restored.`n`nAlso delete Logical Lunge's own settings and data? (config files, clipboard history, to-dos, shortcut and night-light settings, downloaded wallpapers)`n`nYes: delete everything`nNo: keep my settings`nCancel: don't uninstall" }
    $answer = [System.Windows.Forms.MessageBox]::Show($text, 'Logical Lunge', 'YesNoCancel', 'Question')
    if ($answer -eq 'Cancel') { return }
    if ($answer -eq 'Yes') { $RemoveConfig = $true } else { $KeepConfig = $true }
}

if (-not $isAdmin) {
    # Stop the desktop as the user first: the window manager brings the windows of hidden workspaces back when it
    # exits gracefully (and the core brings back anything left invisible)
    $wasRunning = [bool]@(Get-LLOwnedProcesses $APP)
    Stop-LLDesktop $APP $STATE $sameUser $sessionId $false
    # Carry original runtime records through credential elevation even if the core already deleted them.
    foreach ($name in 'shell-takeover.json', 'toast-banners.json') {
        $record = $recovery[$name]
        if ($record -and $record.Valid) { [IO.File]::WriteAllText($record.Path, $record.Raw, (New-Object Text.UTF8Encoding $false)) }
    }
    # one UAC prompt; the elevated copy needs to know whose settings to restore
    $selfDir = Join-Path $env:TEMP ('logical-lunge-uninstall-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path (Join-Path $selfDir 'scripts') -Force | Out-Null
    $self = Join-Path $selfDir 'uninstall.ps1'
    Copy-Item -LiteralPath $PSCommandPath -Destination $self -Force
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'scripts\uninstall-restore.ps1') -Destination (Join-Path $selfDir 'scripts\uninstall-restore.ps1')
    $choice = if ($RemoveConfig) { '-RemoveConfig' } else { '-KeepConfig' }
    try { Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$self`"", '-UserProfile', "`"$UserProfile`"", '-UserSid', $UserSid, '-Elevated', $choice }
    catch {
        # UAC declined: the desktop comes back
        Remove-Item (Join-Path $STATE 'maintenance') -Force -ErrorAction SilentlyContinue
        if ($wasRunning) { Start-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Start' -ErrorAction SilentlyContinue }
    }
    finally {
        Remove-LLTree $selfDir $env:TEMP
    }
    return
}

$cu = "$HKU\Software\Microsoft\Windows\CurrentVersion"
function Log([string]$m) { Write-Host $m }

$backup = $null
$bf = Join-Path $STATE 'install-backup.json'
if ($recovery['install-backup.json'] -and $recovery['install-backup.json'].Valid) { $backup = $recovery['install-backup.json'].Value }

Log '==> Stopping Logical Lunge'
Stop-LLDesktop $APP $STATE $sameUser $sessionId
# Our video screen saver goes with the app: Windows must not keep pointing at a removed LogicalLunge.scr
$desk = "$HKU\Control Panel\Desktop"
$saver = (Get-ItemProperty $desk -Name 'SCRNSAVE.EXE' -ErrorAction SilentlyContinue).'SCRNSAVE.EXE'
if ($saver) {
    $long = (Get-Item -LiteralPath $saver -ErrorAction SilentlyContinue).FullName
    if ($long -and $long.EndsWith('\LogicalLunge.scr', [StringComparison]::OrdinalIgnoreCase)) {
        Remove-ItemProperty $desk -Name 'SCRNSAVE.EXE' -ErrorAction SilentlyContinue
        Set-ItemProperty $desk -Name 'ScreenSaveActive' -Value '0' -ErrorAction SilentlyContinue
    }
}
$restored = Restore-LLWindowsState $recovery $HKU $sameUser
if (-not $restored) {
    # Keep a runnable recovery copy before deleting the installed uninstaller and its helper.
    $retryDir = Join-Path $STATE 'uninstall-recovery'
    [void](Assert-LLSafePath (Join-Path $retryDir 'scripts') $STATE)
    New-Item -ItemType Directory -Path (Join-Path $retryDir 'scripts') -Force -ErrorAction Stop | Out-Null
    $retryScript = Assert-LLSafePath (Join-Path $retryDir 'uninstall.ps1') $STATE
    $retryHelper = Assert-LLSafePath (Join-Path $retryDir 'scripts\uninstall-restore.ps1') $STATE
    if ([IO.Path]::GetFullPath($PSCommandPath) -ne $retryScript) { Copy-Item -LiteralPath $PSCommandPath -Destination $retryScript -Force -ErrorAction Stop }
    $sourceHelper = Join-Path $PSScriptRoot 'scripts\uninstall-restore.ps1'
    if ([IO.Path]::GetFullPath($sourceHelper) -ne $retryHelper) { Copy-Item -LiteralPath $sourceHelper -Destination $retryHelper -Force -ErrorAction Stop }
}
$startMenu = Join-Path $UserProfile 'AppData\Roaming\Microsoft\Windows\Start Menu\Programs'
if (Test-Path -LiteralPath (Join-Path $startMenu 'Logical Lunge')) { Remove-LLTree (Join-Path $startMenu 'Logical Lunge') $startMenu }

Log '==> Removing startup tasks'
foreach ($folder in 'LogicalLunge', 'LL') {
    Get-ScheduledTask -TaskPath "\$folder\" -ErrorAction SilentlyContinue | Unregister-ScheduledTask -Confirm:$false
    try { $svc = New-Object -ComObject Schedule.Service; $svc.Connect(); $svc.GetFolder('\').DeleteFolder($folder, 0) } catch {}
}

Log '==> Restoring Windows settings'
# Restored from the captured installer/runtime records above, including the live desktop view and taskbar state.
# Registry removal has no filesystem traversal. The target hive was validated before shutdown.
Remove-Item -LiteralPath "$cu\Uninstall\LogicalLunge" -Recurse -Force -ErrorAction SilentlyContinue

$installed = if ($backup) { @($backup.installed) } else { @() }
# remove only the PATH entries the installer added
$added = @($installed | Where-Object { $_ -like 'path:*' } | ForEach-Object { $_.Substring(5) })
if ($added.Count) {
    $cur = (Get-ItemProperty "$HKU\Environment" -Name Path -ErrorAction SilentlyContinue).Path
    if ($cur) { Set-ItemProperty "$HKU\Environment" -Name Path -Value (($cur -split ';' | Where-Object { $_ -and $added -notcontains $_ }) -join ';') -Type ExpandString }
}
if ($installed -contains 'pawnio') {
    Log '==> Removing PawnIO driver'
    $pw = Join-Path $APP 'tools\temps\PawnIO_setup.exe'
    if (Test-Path $pw) { Start-Process $pw -ArgumentList '-uninstall', '-silent' -Wait }
}
if ($installed -contains 'everything') {
    Log '==> Removing Everything (file search)'
    $ev = Join-Path $APP 'tools\everything\Everything.exe'
    # only the copy the installer put next to our tools (an Everything the user installed is left alone); its
    # process and service hold the folder, so both go before the app folder is removed
    Get-CimInstance Win32_Process -Filter "Name = 'Everything.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -eq $ev } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    if (Test-Path $ev) { & $ev -uninstall-service | Out-Null }
}
if ($installed -contains 'fonts') {
    Log '==> Removing JetBrainsMono Nerd Font'
    Get-ChildItem "$env:WINDIR\Fonts" -Filter 'JetBrainsMonoNerdFont-*.ttf' -ErrorAction SilentlyContinue | ForEach-Object {
        Remove-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts' -Name "$($_.BaseName) (TrueType)" -ErrorAction SilentlyContinue
        Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue
    }
}
if ($installed -contains 'msys2' -and (Test-Path -LiteralPath 'C:\msys64')) { Log '==> Removing MSYS2 (fish)'; Remove-LLTree 'C:\msys64' 'C:\' }

Log $(if ($RemoveConfig) { '==> Removing program files and Logical Lunge settings' } else { '==> Removing program files (your settings are kept)' })
Restore-LLUserConfigs $UserProfile ([bool]$RemoveConfig)
if (Test-Path -LiteralPath $APP) {
    # The running script's own install directory is checked just like every other target.
    try { Remove-LLTree $APP $env:ProgramFiles } catch { Write-Warning $_.Exception.Message }
}
if (Test-Path $APP) { Log "    some files are in use (an open terminal?) and stay in $APP; delete it after signing out" }
foreach ($d in 'logs', 'update', 'rollback') {
    $target = Join-Path $DATA $d
    if (Test-Path -LiteralPath $target) { Remove-LLTree $target $DATA }
}
if ($restored) { Remove-Item -LiteralPath $bf -Force -ErrorAction SilentlyContinue }
if ($RemoveConfig) {
    if (Test-Path -LiteralPath $CONF) { Remove-LLTree $CONF (Join-Path $UserProfile '.config') }
    if ($restored -and (Test-Path -LiteralPath $DATA)) { Remove-LLTree $DATA $LOCAL } # keep recovery records on failure
    # downloaded wallpapers: the user's Pictures folder (it may be redirected, e.g. to OneDrive)
    $pics = (Get-ItemProperty "$HKU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders" -Name 'My Pictures' -ErrorAction SilentlyContinue).'My Pictures'
    $pics = if ($pics) { $pics.Replace('%USERPROFILE%', $UserProfile) } else { Join-Path $UserProfile 'Pictures' }
    $wallpapers = Join-Path $pics 'Wallpapers\Logical Lunge'
    if (Test-Path -LiteralPath $wallpapers) { Remove-LLTree $wallpapers $pics }
}

Log ''
if ($restored) { Log 'Logical Lunge has been removed. Shared terminal configs and pre-install backups are preserved.' }
else { Write-Warning "Program files removed, but some Windows settings need recovery. Records are kept in $STATE; run $retryDir\uninstall.ps1 from the target user session before reinstalling." }
