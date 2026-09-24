# Logical Lunge - one-line installer
#   irm https://raw.githubusercontent.com/KaanAlper/logical-lunge/main/install.ps1 | iex
# Downloads the latest release and runs installer\setup.ps1 with a single UAC prompt.
# Options (set before running):  $env:LL_NO_TERMINAL = 1   skip WezTerm + fish
#                                 $env:LL_NO_SENSORS = 1    skip the PawnIO driver (CPU temperature)
#                                 $env:LL_SOURCE = <folder> install from a local build (dist\LogicalLunge-x.y.z)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$repo = 'KaanAlper/logical-lunge'

Write-Host ''
Write-Host '  Logical Lunge' -ForegroundColor Magenta
Write-Host '  illogical-impulse for Windows (GlazeWM + Zebar + ll-helper)' -ForegroundColor DarkGray
Write-Host ''

$build = [int](Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').CurrentBuildNumber
if ($build -lt 19041) { Write-Host "Windows 10 2004 (19041) or newer is required (this is $build)." -ForegroundColor Red; return }

$src = $env:LL_SOURCE
if (-not $src) {
    $work = Join-Path $env:TEMP 'll-install'
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $work | Out-Null
    Write-Host 'Downloading the latest release...'
    $rel = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$repo/releases/latest"
    $asset = $rel.assets | Where-Object { $_.name -like 'LogicalLunge-*.zip' } | Select-Object -First 1
    if (-not $asset) { Write-Host 'No release package found.' -ForegroundColor Red; return }
    $zip = Join-Path $work $asset.name
    Invoke-WebRequest -UseBasicParsing -Uri $asset.browser_download_url -OutFile $zip
    $sha = $rel.assets | Where-Object { $_.name -eq "$($asset.name).sha256" } | Select-Object -First 1
    if ($sha) {
        $raw = (Invoke-WebRequest -UseBasicParsing $sha.browser_download_url).Content
        if ($raw -is [byte[]]) { $raw = [Text.Encoding]::ASCII.GetString($raw) }
        $expected = ($raw -split '\s+')[0].Trim().ToUpper()
        if ($expected -and $expected -ne (Get-FileHash $zip -Algorithm SHA256).Hash) { Write-Host 'Checksum mismatch, aborting.' -ForegroundColor Red; return }
    }
    Expand-Archive $zip $work -Force
    $src = (Get-ChildItem $work -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'installer\setup.ps1') } | Select-Object -First 1).FullName
}
if (-not $src -or -not (Test-Path (Join-Path $src 'installer\setup.ps1'))) { Write-Host 'Installer files not found.' -ForegroundColor Red; return }

# Stop the window manager as the user before the elevated setup: GlazeWM hides the windows of other workspaces
# (cloak) and a killed / older GlazeWM left them invisible. A graceful exit brings them back (Logical Lunge's GlazeWM
# build); anything still hidden is brought back by the helper. Returns whether GlazeWM was running.
function Stop-LLDesktop([string]$helperExe) {
    $gw = Get-Process glazewm -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $gw) { return $false }
    try { & $gw.Path command wm-exit 2>$null | Out-Null } catch {}
    if (-not $gw.WaitForExit(5000)) { Stop-Process -Id $gw.Id -Force -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 300 }
    if ($helperExe -and (Test-Path $helperExe)) { try { & $helperExe --uncloak-orphans | Out-Null } catch {} }
    return $true
}

$me = [Security.Principal.WindowsIdentity]::GetCurrent()
$args2 = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $src 'installer\setup.ps1')`"",
    '-Source', "`"$src`"", '-UserProfile', "`"$env:USERPROFILE`"", '-UserSid', $me.User.Value, '-UserName', "`"$($me.Name)`"")
if ($env:LL_NO_TERMINAL) { $args2 += '-NoTerminal' }
if ($env:LL_NO_SENSORS) { $args2 += '-NoSensors' }
Write-Host 'Installing (Windows will ask for permission once)...'
$wasRunning = Stop-LLDesktop (Join-Path $src 'logical-lunge\helper\ll-helper.exe')
try { $p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList $args2 }
catch {
    if ($wasRunning) { Start-ScheduledTask -TaskPath '\LL\' -TaskName 'GlazeWM' -ErrorAction SilentlyContinue }
    Write-Host 'Installation cancelled.' -ForegroundColor Yellow; return
}
if ($p.ExitCode -eq 0) {
    Write-Host ''
    Write-Host 'Logical Lunge is installed. Press Super for search, Super+Enter for a terminal.' -ForegroundColor Green
    Write-Host 'Uninstall any time from Settings > Apps, or run: ~\.glzr\logical-lunge\uninstall.ps1'
}
else { Write-Host "Setup failed (exit $($p.ExitCode)). Log: $env:TEMP\logical-lunge-install.log" -ForegroundColor Red }
