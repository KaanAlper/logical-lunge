# Logical Lunge - kurulu sürümü indirilmiş paketle günceller (sağ panel > güncelle).
# lunge.exe --update-install bunu kendi kopyasından, gizli olarak başlatır. Adımlar:
#   paketi aç -> masaüstünü kapat (kullanıcı olarak) -> UAC'ı bir kez sor -> kurulum betiği -> perde, masaüstü açılır
# Durum update\status.json'a yazılır; widget onu okuyup ilerlemeyi ya da hatayı gösterir.
param([Parameter(Mandatory = $true)][string]$Zip)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$dir = Split-Path $Zip
$status = Join-Path $dir 'status.json'
function Set-State($state, $err) {
    $j = [ordered]@{ state = $state; version = ''; bytes = 0; total = 0; error = [string]$err } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($status, $j, (New-Object Text.UTF8Encoding $false)) # BOM'suz
}
# Kurulum yapılmadan vazgeçilirse (UAC reddi, bozuk paket) masaüstünü geri aç
function Start-Desktop {
    foreach ($m in 'LogicalLunge\state\maintenance', 'logical-lunge\maintenance') { Remove-Item (Join-Path $env:LOCALAPPDATA $m) -Force -ErrorAction SilentlyContinue }
    $core = Join-Path $env:LOCALAPPDATA 'Programs\LogicalLunge\lunge.exe'
    if (Test-Path $core) { Start-Process $core -WorkingDirectory $env:USERPROFILE }
}

$splash = $null; $stopped = $false
try {
    Set-State 'installing' ''
    $work = Join-Path $dir 'pkg'
    if (Test-Path $work) { Remove-Item $work -Recurse -Force }
    Expand-Archive -Path $Zip -DestinationPath $work -Force
    $src = $null
    if (Test-Path (Join-Path $work 'installer\setup.ps1')) { $src = $work }
    else { $src = (Get-ChildItem $work -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'installer\setup.ps1') } | Select-Object -First 1).FullName }
    if (-not $src) { throw 'Paket geçersiz: installer\setup.ps1 bulunamadı.' }
    $pkgCore = Join-Path $src 'app\lunge.exe'
    if (-not (Test-Path $pkgCore)) { throw 'Paket geçersiz: app\lunge.exe bulunamadı.' }

    # Perde (kurulan dosya kilitlenmesin diye kopyadan): mevcut masaüstünün kapanmasını, sonra yenisini bekler
    $copy = Join-Path $env:TEMP 'lunge-update-splash.exe'
    Copy-Item $pkgCore $copy -Force
    $env:LL_SPLASH_WAIT_RESTART = '1'
    $splash = Start-Process $copy -ArgumentList '--splash' -PassThru
    $env:LL_SPLASH_WAIT_RESTART = $null

    # Masaüstünü kullanıcı olarak kapat: pencere yöneticisi gizli workspace'lerin pencerelerini geri getirir
    & $pkgCore --stop-desktop | Out-Null
    $stopped = $true

    $me = [Security.Principal.WindowsIdentity]::GetCurrent()
    $args2 = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $src 'installer\setup.ps1')`"",
        '-Source', "`"$src`"", '-UserProfile', "`"$env:USERPROFILE`"", '-UserSid', $me.User.Value, '-UserName', "`"$($me.Name)`"")
    # Kullanıcı UAC'ı reddederse hata verir: catch masaüstünü geri açar
    $setup = Start-Process powershell.exe -Verb RunAs -PassThru -ArgumentList $args2
    $setup.WaitForExit()
    if ($setup.ExitCode -ne 0) { throw "Kurulum başarısız (kod $($setup.ExitCode)). Günlük: $env:TEMP\logical-lunge-install.log" }
    Set-State 'done' ''
}
catch {
    if ($splash -and -not $splash.HasExited) { Stop-Process -Id $splash.Id -Force -ErrorAction SilentlyContinue }
    if ($stopped) { Start-Desktop }
    Set-State 'error' $_.Exception.Message
    exit 1
}
