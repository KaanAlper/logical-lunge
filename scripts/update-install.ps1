# Logical Lunge - kurulu sürümü indirilmiş paketle günceller (sağ panel > güncelle).
# ll-helper --update-install bunu kendi kopyasından, gizli olarak başlatır. Adımlar:
#   paketi aç -> UAC'ı bir kez sor -> kurulum betiğini çalıştır -> masaüstü yeniden açılırken duvar kağıdı örtüsü
# Durum, update\status.json'a yazılır; widget onu okuyup ilerlemeyi ya da hatayı gösterir.
param([Parameter(Mandatory = $true)][string]$Zip)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$dir = Split-Path $Zip
$status = Join-Path $dir 'status.json'
function Set-State($state, $err) {
    $j = [ordered]@{ state = $state; version = ''; bytes = 0; total = 0; error = [string]$err } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($status, $j, (New-Object Text.UTF8Encoding $false)) # BOM'suz
}
# Stop the window manager as the user before the elevated setup: GlazeWM hides the windows of other workspaces
# (cloak) and a killed / older GlazeWM left them invisible. A graceful exit brings them back (Logical Lunge's GlazeWM
# build); anything still hidden is brought back by the helper. Returns whether GlazeWM was running.
function Stop-LLDesktop([string]$helperExe) {
    # Maintenance marker: ll-helper's watchdogs restart a crashed GlazeWM / Zebar / helper, but not while this is
    # present (setup removes it when it starts the desktop again; it counts for 10 minutes at most)
    $state = Join-Path $env:LOCALAPPDATA 'logical-lunge'
    New-Item -ItemType Directory -Force $state | Out-Null
    Set-Content (Join-Path $state 'maintenance') (Get-Date -Format o)
    $gw = Get-Process glazewm -ErrorAction SilentlyContinue | Sort-Object StartTime | Select-Object -First 1
    if (-not $gw) { return $false }
    try { & $gw.Path command wm-exit 2>$null | Out-Null } catch {}
    if (-not $gw.WaitForExit(5000)) {
        # stuck: the helper first, so that it can't restart GlazeWM
        Get-Process ll-helper -ErrorAction SilentlyContinue | Stop-Process -Force
        Stop-Process -Id $gw.Id -Force -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 300
    }
    if ($helperExe -and (Test-Path $helperExe)) { try { & $helperExe --uncloak-orphans | Out-Null } catch {} }
    return $true
}

$splash = $null
try {
    Set-State 'installing' ''
    $work = Join-Path $dir 'pkg'
    if (Test-Path $work) { Remove-Item $work -Recurse -Force }
    Expand-Archive -Path $Zip -DestinationPath $work -Force
    $src = $null
    if (Test-Path (Join-Path $work 'installer\setup.ps1')) { $src = $work }
    else { $src = (Get-ChildItem $work -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'installer\setup.ps1') } | Select-Object -First 1).FullName }
    if (-not $src) { throw 'Paket geçersiz: installer\setup.ps1 bulunamadı.' }

    $me = [Security.Principal.WindowsIdentity]::GetCurrent()
    $args2 = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $src 'installer\setup.ps1')`"",
        '-Source', "`"$src`"", '-UserProfile', "`"$env:USERPROFILE`"", '-UserSid', $me.User.Value, '-UserName', "`"$($me.Name)`"")
    $wasRunning = Stop-LLDesktop (Join-Path $src 'logical-lunge\helper\ll-helper.exe')
    # Kullanıcı UAC'ı reddederse burada hata verir: masaüstünü (GlazeWM) geri başlat
    try { $setup = Start-Process powershell.exe -Verb RunAs -PassThru -ArgumentList $args2 }
    catch { Remove-Item (Join-Path $env:LOCALAPPDATA 'logical-lunge\maintenance') -Force -ErrorAction SilentlyContinue; if ($wasRunning) { Start-ScheduledTask -TaskPath '\LL\' -TaskName 'GlazeWM' -ErrorAction SilentlyContinue }; throw }

    # Onaylandı: kurulum masaüstünü kapatacak. Örtüyü hemen aç (kurulan dosya kilitlenmesin diye kopyadan)
    $copy = Join-Path $env:TEMP 'll-update-splash.exe'
    Copy-Item (Join-Path $env:USERPROFILE '.glzr\logical-lunge\helper\ll-helper.exe') $copy -Force
    $env:LL_SPLASH_WAIT_RESTART = '1'
    $splash = Start-Process $copy -ArgumentList '--splash' -PassThru

    $setup.WaitForExit()
    if ($setup.ExitCode -ne 0) { throw "Kurulum başarısız (kod $($setup.ExitCode)). Günlük: $env:TEMP\logical-lunge-install.log" }
    Set-State 'done' ''
}
catch {
    Remove-Item (Join-Path $env:LOCALAPPDATA 'logical-lunge\maintenance') -Force -ErrorAction SilentlyContinue
    if ($splash -and -not $splash.HasExited) { Stop-Process -Id $splash.Id -Force -ErrorAction SilentlyContinue }
    Set-State 'error' $_.Exception.Message
    exit 1
}
