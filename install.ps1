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
        $expected = ((Invoke-WebRequest -UseBasicParsing $sha.browser_download_url).Content -replace '[^0-9a-fA-F].*$', '').Trim()
        if ($expected -and $expected -ne (Get-FileHash $zip -Algorithm SHA256).Hash) { Write-Host 'Checksum mismatch, aborting.' -ForegroundColor Red; return }
    }
    Expand-Archive $zip $work -Force
    $src = (Get-ChildItem $work -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'installer\setup.ps1') } | Select-Object -First 1).FullName
}
if (-not $src -or -not (Test-Path (Join-Path $src 'installer\setup.ps1'))) { Write-Host 'Installer files not found.' -ForegroundColor Red; return }

$me = [Security.Principal.WindowsIdentity]::GetCurrent()
$args2 = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $src 'installer\setup.ps1')`"",
    '-Source', "`"$src`"", '-UserProfile', "`"$env:USERPROFILE`"", '-UserSid', $me.User.Value, '-UserName', "`"$($me.Name)`"")
if ($env:LL_NO_TERMINAL) { $args2 += '-NoTerminal' }
if ($env:LL_NO_SENSORS) { $args2 += '-NoSensors' }
Write-Host 'Installing (Windows will ask for permission once)...'
$p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList $args2
if ($p.ExitCode -eq 0) {
    Write-Host ''
    Write-Host 'Logical Lunge is installed. Press Super for search, Super+Enter for a terminal.' -ForegroundColor Green
    Write-Host 'Uninstall any time from Settings > Apps, or run: ~\.glzr\logical-lunge\uninstall.ps1'
}
else { Write-Host "Setup failed (exit $($p.ExitCode)). Log: $env:TEMP\logical-lunge-install.log" -ForegroundColor Red }
