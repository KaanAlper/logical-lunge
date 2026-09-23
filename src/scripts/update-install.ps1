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
    ([ordered]@{ state = $state; version = ''; bytes = 0; total = 0; error = [string]$err } | ConvertTo-Json -Compress) |
        Set-Content -Path $status -Encoding UTF8
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
    # Kullanıcı UAC'ı reddederse burada hata verir; örtü henüz açılmadığı için masaüstü olduğu gibi kalır
    $setup = Start-Process powershell.exe -Verb RunAs -PassThru -ArgumentList $args2

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
    if ($splash -and -not $splash.HasExited) { Stop-Process -Id $splash.Id -Force -ErrorAction SilentlyContinue }
    Set-State 'error' $_.Exception.Message
    exit 1
}
