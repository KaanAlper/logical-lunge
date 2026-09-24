# Logical Lunge - uninstaller. Restores every Windows setting that the installer changed and removes
# everything it installed. Configuration files that existed before the install always come back.
# Logical Lunge's own settings and data (its GlazeWM/Zebar/WezTerm/fish/starship/tacky-borders configs,
# clipboard history, shortcut and night-light settings, downloaded wallpapers) are kept or removed:
#   -KeepConfig / -RemoveConfig, or a Yes/No/Cancel question when neither is given.
param([string]$UserProfile = $env:USERPROFILE, [string]$UserSid = '', [switch]$Elevated, [switch]$KeepConfig, [switch]$RemoveConfig)
$ErrorActionPreference = 'Continue'

if (-not $KeepConfig -and -not $RemoveConfig) {
    Add-Type -AssemblyName System.Windows.Forms
    $tr = (Get-UICulture).Name -like 'tr*'
    $text = if ($tr) { "Logical Lunge kaldırılacak ve Windows ayarları eski haline dönecek.`n`nLogical Lunge'ın kendi ayarları ve verileri de silinsin mi? (config dosyaları, pano geçmişi, kısayol ve gece ışığı ayarları, indirilen duvar kağıtları)`n`nEvet: hepsini sil`nHayır: ayarları sakla`nİptal: kaldırma" }
            else { "Logical Lunge will be removed and your Windows settings restored.`n`nAlso delete Logical Lunge's own settings and data? (config files, clipboard history, shortcut and night-light settings, downloaded wallpapers)`n`nYes: delete everything`nNo: keep my settings`nCancel: don't uninstall" }
    $answer = [System.Windows.Forms.MessageBox]::Show($text, 'Logical Lunge', 'YesNoCancel', 'Question')
    if ($answer -eq 'Cancel') { return }
    if ($answer -eq 'Yes') { $RemoveConfig = $true } else { $KeepConfig = $true }
}

if (-not $UserSid) { $UserSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    # one UAC prompt; the elevated copy needs to know whose settings to restore
    $self = Join-Path $env:TEMP 'logical-lunge-uninstall.ps1'
    Copy-Item $PSCommandPath $self -Force
    $choice = if ($RemoveConfig) { '-RemoveConfig' } else { '-KeepConfig' }
    Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$self`"", '-UserProfile', "`"$UserProfile`"", '-UserSid', $UserSid, '-Elevated', $choice
    return
}

$LL = Join-Path $UserProfile '.glzr\logical-lunge'
$ZB = Join-Path $UserProfile '.glzr\zebar'
$STATE = Join-Path $UserProfile 'AppData\Local\logical-lunge'
$HKU = "Registry::HKEY_USERS\$UserSid"
$cu = "$HKU\Software\Microsoft\Windows\CurrentVersion"
function Log([string]$m) { Write-Host $m }

$backup = $null
$bf = Join-Path $STATE 'install-backup.json'
if (Test-Path $bf) { $backup = Get-Content $bf -Raw | ConvertFrom-Json }

Log '==> Stopping Logical Lunge'
foreach ($n in 'glazewm', 'zebar', 'll-helper', 'tacky-borders', 'll-temps', 'll-songrec') { Get-Process $n -ErrorAction SilentlyContinue | Stop-Process -Force }

Log '==> Removing startup tasks'
Get-ScheduledTask -TaskPath '\LL\' -ErrorAction SilentlyContinue | Unregister-ScheduledTask -Confirm:$false

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
foreach ($c in 'Shell_TrayWnd', 'Shell_SecondaryTrayWnd') { $h = [LLTB]::FindWindow($c, $null); if ($h -ne [IntPtr]::Zero) { [LLTB]::ShowWindow($h, 5) | Out-Null } }

$installed = if ($backup) { @($backup.installed) } else { @() }
# remove only the PATH entries the installer added
$added = @($installed | Where-Object { $_ -like 'path:*' } | ForEach-Object { $_.Substring(5) })
if ($added.Count) {
    $cur = (Get-ItemProperty "$HKU\Environment" -Name Path -ErrorAction SilentlyContinue).Path
    if ($cur) { Set-ItemProperty "$HKU\Environment" -Name Path -Value (($cur -split ';' | Where-Object { $_ -and $added -notcontains $_ }) -join ';') -Type ExpandString }
}
function Uninstall-Msi([string]$display) {
    foreach ($root in 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall') {
        Get-ChildItem $root -ErrorAction SilentlyContinue | ForEach-Object {
            $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
            if ($p.DisplayName -like "$display*" -and $_.PSChildName -match '^\{') { Start-Process msiexec.exe -ArgumentList '/x', $_.PSChildName, '/qn', '/norestart' -Wait }
        }
    }
}
if ($installed -contains 'glazewm') { Log '==> Removing GlazeWM'; Uninstall-Msi 'GlazeWM' }
if ($installed -contains 'zebar') { Log '==> Removing Zebar'; Uninstall-Msi 'Zebar' }
if ($installed -contains 'pawnio') {
    Log '==> Removing PawnIO driver'
    $pw = Join-Path $LL 'tools\lhm\PawnIO_setup.exe'
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
foreach ($f in "$ZB\settings.json", "$UserProfile\.glzr\glazewm\config.yaml", "$UserProfile\.wezterm.lua", "$UserProfile\.config\fish\config.fish", "$UserProfile\.config\starship.toml") {
    if (Test-Path "$f.before-ll") { Move-Item "$f.before-ll" $f -Force }   # your pre-install version always comes back
    elseif ($RemoveConfig) { Remove-Item $f -Force -ErrorAction SilentlyContinue }   # ours: there was none before the install
}
Remove-Item (Join-Path $ZB 'logical-lunge') -Recurse -Force -ErrorAction SilentlyContinue
foreach ($d in 'bin', 'helper', 'tools', 'scripts') { Remove-Item (Join-Path $LL $d) -Recurse -Force -ErrorAction SilentlyContinue }
Remove-Item $bf -Force -ErrorAction SilentlyContinue
if ($RemoveConfig) {
    Remove-Item (Join-Path $UserProfile '.config\tacky-borders') -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $STATE -Recurse -Force -ErrorAction SilentlyContinue   # clipboard history, shortcuts, night light
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
Remove-Item (Join-Path $LL 'uninstall.ps1') -Force -ErrorAction SilentlyContinue
if ($RemoveConfig) { Remove-Item $LL -Recurse -Force -ErrorAction SilentlyContinue }
