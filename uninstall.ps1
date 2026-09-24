# Logical Lunge - uninstaller. Restores every Windows setting that the installer changed and removes
# everything it installed. Configuration files that existed before the install always come back.
# Logical Lunge's own settings and data (~\.config\logical-lunge, its WezTerm/fish/starship configs, clipboard
# history, widget data, shortcut and night-light settings, downloaded wallpapers) are kept or removed:
#   -KeepConfig / -RemoveConfig, or a Yes/No/Cancel question when neither is given.
param([string]$UserProfile = $env:USERPROFILE, [string]$UserSid = '', [switch]$Elevated, [switch]$KeepConfig, [switch]$RemoveConfig)
$ErrorActionPreference = 'Continue'

$LOCAL = Join-Path $UserProfile 'AppData\Local'
$APP = Join-Path $LOCAL 'Programs\LogicalLunge'
$DATA = Join-Path $LOCAL 'LogicalLunge'
$STATE = Join-Path $DATA 'state'
$CONF = Join-Path $UserProfile '.config\logical-lunge'

if (-not $KeepConfig -and -not $RemoveConfig) {
    Add-Type -AssemblyName System.Windows.Forms
    $tr = (Get-UICulture).Name -like 'tr*'
    $text = if ($tr) { "Logical Lunge kaldırılacak ve Windows ayarları eski haline dönecek.`n`nLogical Lunge'ın kendi ayarları ve verileri de silinsin mi? (ayar dosyaları, pano geçmişi, yapılacaklar, kısayol ve gece ışığı ayarları, indirilen duvar kağıtları)`n`nEvet: hepsini sil`nHayır: ayarları sakla`nİptal: kaldırma" }
            else { "Logical Lunge will be removed and your Windows settings restored.`n`nAlso delete Logical Lunge's own settings and data? (config files, clipboard history, to-dos, shortcut and night-light settings, downloaded wallpapers)`n`nYes: delete everything`nNo: keep my settings`nCancel: don't uninstall" }
    $answer = [System.Windows.Forms.MessageBox]::Show($text, 'Logical Lunge', 'YesNoCancel', 'Question')
    if ($answer -eq 'Cancel') { return }
    if ($answer -eq 'Yes') { $RemoveConfig = $true } else { $KeepConfig = $true }
}

if (-not $UserSid) { $UserSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $isAdmin) {
    # Stop the desktop as the user first: the window manager brings the windows of hidden workspaces back when it
    # exits gracefully (and the core brings back anything left invisible)
    $core = Join-Path $APP 'lunge.exe'
    $wasRunning = [bool](Get-Process lunge-tiling -ErrorAction SilentlyContinue)
    if (Test-Path $core) { & $core --stop-desktop | Out-Null }
    # one UAC prompt; the elevated copy needs to know whose settings to restore
    $self = Join-Path $env:TEMP 'logical-lunge-uninstall.ps1'
    Copy-Item $PSCommandPath $self -Force
    $choice = if ($RemoveConfig) { '-RemoveConfig' } else { '-KeepConfig' }
    try { Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$self`"", '-UserProfile', "`"$UserProfile`"", '-UserSid', $UserSid, '-Elevated', $choice }
    catch {
        # UAC declined: the desktop comes back
        Remove-Item (Join-Path $STATE 'maintenance') -Force -ErrorAction SilentlyContinue
        if ($wasRunning -and (Test-Path $core)) { Start-Process $core -WorkingDirectory $UserProfile }
    }
    return
}

$HKU = "Registry::HKEY_USERS\$UserSid"
$cu = "$HKU\Software\Microsoft\Windows\CurrentVersion"
function Log([string]$m) { Write-Host $m }

$backup = $null
$bf = Join-Path $STATE 'install-backup.json'
if (Test-Path $bf) { $backup = Get-Content $bf -Raw | ConvertFrom-Json }

Log '==> Stopping Logical Lunge'
# the core first: its watchdogs would restart the other parts
foreach ($n in 'lunge', 'lunge-tiling', 'lunge-tiling-watcher', 'lunge-shell', 'lunge-temps', 'lunge-songrec', 'lunge-termcolors') { Get-Process $n -ErrorAction SilentlyContinue | Stop-Process -Force }
Remove-Item (Join-Path $UserProfile 'AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Logical Lunge') -Recurse -Force -ErrorAction SilentlyContinue

Log '==> Removing startup tasks'
foreach ($folder in 'LogicalLunge', 'LL') {
    Get-ScheduledTask -TaskPath "\$folder\" -ErrorAction SilentlyContinue | Unregister-ScheduledTask -Confirm:$false
    try { $svc = New-Object -ComObject Schedule.Service; $svc.Connect(); $svc.GetFolder('\').DeleteFolder($folder, 0) } catch {}
}

Log '==> Restoring Windows settings'
if ($backup) {
    foreach ($r in @($backup.registry)) {
        try {
            if ($r.existed) {
                $v = if ($r.binary) { [Convert]::FromBase64String($r.old) } else { $r.old }
                Set-ItemProperty -Path $r.path -Name $r.name -Value $v -Type $r.type
            }
            else { Remove-ItemProperty -Path $r.path -Name $r.name -ErrorAction SilentlyContinue }
        }
        catch { Log "    could not restore $($r.path)\$($r.name): $($_.Exception.Message)" }
    }
}
Remove-Item "$cu\Uninstall\LogicalLunge" -Recurse -Force -ErrorAction SilentlyContinue

Log '==> Showing the Windows taskbar again'
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class LLTB { [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string t); [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n); }
'@
foreach ($c in 'Shell_TrayWnd', 'Shell_SecondaryTrayWnd') { $h = [LLTB]::FindWindow($c, [NullString]::Value); if ($h -ne [IntPtr]::Zero) { [LLTB]::ShowWindow($h, 5) | Out-Null } }

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
if ($installed -contains 'fonts') {
    Log '==> Removing JetBrainsMono Nerd Font'
    Get-ChildItem "$env:WINDIR\Fonts" -Filter 'JetBrainsMonoNerdFont-*.ttf' -ErrorAction SilentlyContinue | ForEach-Object {
        Remove-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts' -Name "$($_.BaseName) (TrueType)" -ErrorAction SilentlyContinue
        Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue
    }
}
if ($installed -contains 'msys2') { Log '==> Removing MSYS2 (fish)'; Remove-Item 'C:\msys64' -Recurse -Force -ErrorAction SilentlyContinue }

Log $(if ($RemoveConfig) { '==> Removing program files and Logical Lunge settings' } else { '==> Removing program files (your settings are kept)' })
foreach ($f in "$UserProfile\.wezterm.lua", "$UserProfile\.config\fish\config.fish", "$UserProfile\.config\starship.toml") {
    if (Test-Path "$f.before-ll") { Move-Item "$f.before-ll" $f -Force }   # your pre-install version always comes back
    elseif ($RemoveConfig) { Remove-Item $f -Force -ErrorAction SilentlyContinue }   # ours: there was none before the install
}
Remove-Item $APP -Recurse -Force -ErrorAction SilentlyContinue
if (Test-Path $APP) { Log "    some files are in use (an open terminal?) and stay in $APP; delete it after signing out" }
foreach ($d in 'logs', 'update', 'rollback') { Remove-Item (Join-Path $DATA $d) -Recurse -Force -ErrorAction SilentlyContinue }
Remove-Item $bf -Force -ErrorAction SilentlyContinue
if ($RemoveConfig) {
    Remove-Item $CONF -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $DATA -Recurse -Force -ErrorAction SilentlyContinue   # clipboard history, widget data, night light
    # downloaded wallpapers: the user's Pictures folder (it may be redirected, e.g. to OneDrive)
    $pics = (Get-ItemProperty "$HKU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders" -Name 'My Pictures' -ErrorAction SilentlyContinue).'My Pictures'
    $pics = if ($pics) { $pics.Replace('%USERPROFILE%', $UserProfile) } else { Join-Path $UserProfile 'Pictures' }
    Remove-Item (Join-Path $pics 'Wallpapers\Logical Lunge') -Recurse -Force -ErrorAction SilentlyContinue
}

Log '==> Restarting Explorer'
Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
Log ''
Log 'Logical Lunge has been removed. Sign out and back in to finish.'
Start-Sleep 3
