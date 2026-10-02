# Logical Lunge - kurulu sürümü indirilmiş paketle günceller (sağ panel > güncelle).
# lunge.exe --update-install bunu kendi kopyasından, gizli olarak başlatır. Adımlar:
#   paketi aç -> UAC'ı bir kez sor (masaüstü daha açıkken: pencere görünsün) -> masaüstünü kapat (kullanıcı olarak)
#   -> bekleyen yönetici süreç kurulum betiğini çalıştırır -> perde, masaüstü açılır
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
    # from its sign-in task: the core starts with its rights (elevated)
    Start-ScheduledTask -TaskPath '\LogicalLunge\' -TaskName 'Start' -ErrorAction SilentlyContinue
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

    # UAC önce: masaüstü kapandıktan sonra sorulunca pencere perdenin arkasında kalıyordu (ekran boş, yalnızca
    # Alt+Tab'da). Onaylanan yönetici süreç "go" dosyasını bekler; masaüstü kullanıcı olarak kapanınca kurulumu yapar.
    # "cancel" dosyası ya da 3 dakika: vazgeçilir.
    $go = Join-Path $dir 'go'; $cancel = Join-Path $dir 'cancel'
    Remove-Item $go, $cancel -Force -ErrorAction SilentlyContinue
    $me = [Security.Principal.WindowsIdentity]::GetCurrent()
    function Q($v) { "'" + ([string]$v -replace "'", "''") + "'" }
    $inner = "`$sw = [Diagnostics.Stopwatch]::StartNew(); " +
        "while (-not (Test-Path $(Q $go))) { if ((Test-Path $(Q $cancel)) -or `$sw.Elapsed.TotalMinutes -gt 3) { exit 2 }; Start-Sleep -Milliseconds 100 }; " +
        "& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $(Q (Join-Path $src 'installer\setup.ps1')) -Source $(Q $src) " +
        "-UserProfile $(Q $env:USERPROFILE) -UserSid $(Q $me.User.Value) -UserName $(Q $me.Name); exit `$LASTEXITCODE"
    $enc = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($inner))
    # Onay penceresi öndeki pencereye bağlanır: sahibi olmayan bir istek görev çubuğunda yanıp sönen bir düğme olarak
    # kalıyordu (bizim masaüstünde görev çubuğu yok). Reddedilirse hata verir: masaüstü hiç kapanmamış olur.
    Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class LLElevate {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct SEI { public int cbSize; public uint fMask; public IntPtr hwnd; public string lpVerb, lpFile, lpParameters, lpDirectory;
        public int nShow; public IntPtr hInstApp, lpIDList; public string lpClass; public IntPtr hkeyClass; public uint dwHotKey;
        public IntPtr hIcon, hProcess; }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool ShellExecuteExW(ref SEI e);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr h, uint ms);
    [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr h, out int code);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    public static IntPtr Start(string file, string args) {
        var e = new SEI { cbSize = Marshal.SizeOf(typeof(SEI)), fMask = 0x40 /* NOCLOSEPROCESS */, hwnd = GetForegroundWindow(),
            lpVerb = "runas", lpFile = file, lpParameters = args, nShow = 0 };
        if (!ShellExecuteExW(ref e)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        return e.hProcess;
    }
    public static int Wait(IntPtr h) { int c; WaitForSingleObject(h, 0xFFFFFFFF); GetExitCodeProcess(h, out c); CloseHandle(h); return c; }
}
'@
    $setup = [LLElevate]::Start('powershell.exe', "-NoProfile -ExecutionPolicy Bypass -EncodedCommand $enc")

    try {
        # Perde (kurulan dosya kilitlenmesin diye kopyadan): mevcut masaüstünün kapanmasını, sonra yenisini bekler
        $copy = Join-Path $env:TEMP 'lunge-update-splash.exe'
        Copy-Item $pkgCore $copy -Force
        $env:LL_SPLASH_WAIT_RESTART = '1'
        $splash = Start-Process $copy -ArgumentList '--splash' -PassThru
        $env:LL_SPLASH_WAIT_RESTART = $null

        # Masaüstünü kullanıcı olarak kapat: pencere yöneticisi gizli workspace'lerin pencerelerini geri getirir
        & $pkgCore --stop-desktop | Out-Null
        $stopped = $true
        New-Item $go -ItemType File -Force | Out-Null
    }
    catch { New-Item $cancel -ItemType File -Force | Out-Null; throw }
    $code = [LLElevate]::Wait($setup)
    Remove-Item $go -Force -ErrorAction SilentlyContinue
    if ($code -ne 0) { throw "Kurulum başarısız (kod $code). Günlük: $env:TEMP\logical-lunge-install.log" }
    Set-State 'done' ''
}
catch {
    if ($splash -and -not $splash.HasExited) { Stop-Process -Id $splash.Id -Force -ErrorAction SilentlyContinue }
    if ($stopped) { Start-Desktop }
    Set-State 'error' $_.Exception.Message
    exit 1
}
