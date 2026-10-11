# Logical Lunge - CI test: the window manager against windows that refuse their place.
#
# A v1.5.2 report had the window manager's thread stuck at full processor use for minutes (no workspace switches,
# no layout) until the core killed it. This runs the desktop from the package and opens test windows that never accept
# what the window manager gives them, all at once:
#   - a window whose minimum size (WM_GETMINMAXINFO) is bigger than any tile,
#   - a window that resizes itself back to its own size each time it is resized,
#   - a window that moves itself by a pixel thousands of times a second (a location event storm),
#   - a window that renames itself thousands of times a second (a title event storm).
# The window manager must stay responsive (every IPC query answered quickly), keep its processor use low and not be
# restarted. Its stall watch lines (tiling.log) are printed.
#
#   pwsh tools/ci/refusing-windows-test.ps1 -Package <folder with app\lunge.exe>
param(
    [Parameter(Mandatory = $true)][string]$Package,
    [string]$Report = $env:GITHUB_STEP_SUMMARY
)
$ErrorActionPreference = 'Stop'

$STARTUP_TIMEOUT_SEC = 180
$SETTLE_SEC          = 30    # after startup, before the baseline
$STORM_SEC           = 40    # the refusing windows run this long
$MAX_TILING_CPU      = 60    # % of one logical CPU, window manager process, over the storm
$MAX_PING_MS         = 2000  # slowest answer to an IPC query during the storm

$failures = New-Object System.Collections.Generic.List[string]
function Fail([string]$m) { $failures.Add($m); Write-Host "FAIL: $m" -ForegroundColor Red }
function Note([string]$m) { Write-Host $m }

Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

public static class LLRefuse
{
    delegate IntPtr WndProcFn(IntPtr h, uint m, IntPtr w, IntPtr l);
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WNDCLASSEX { public uint cbSize, style; public WndProcFn lpfnWndProc; public int cbClsExtra, cbWndExtra; public IntPtr hInstance, hIcon, hCursor, hbrBackground; public string lpszMenuName, lpszClassName; public IntPtr hIconSm; }
    [StructLayout(LayoutKind.Sequential)] struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct MINMAXINFO { public POINT Reserved, MaxSize, MaxPosition, MinTrackSize, MaxTrackSize; }
    [StructLayout(LayoutKind.Sequential)] struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam, lParam; public uint time; public POINT pt; }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern ushort RegisterClassExW(ref WNDCLASSEX c);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint ex, string cls, string title, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
    [DllImport("user32.dll")] static extern IntPtr DefWindowProcW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool PeekMessageW(out MSG m, IntPtr h, uint min, uint max, uint remove);
    [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG m);
    [DllImport("user32.dll")] static extern IntPtr DispatchMessageW(ref MSG m);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int hgt, uint flags);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool SetWindowTextW(IntPtr h, string t);
    [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr h);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandleW(string name);

    const uint WS_VISIBLE_OVERLAPPED = 0x10000000 | 0x00CF0000;
    const uint SWP_NOMOVE = 0x2, SWP_NOSIZE = 0x1, SWP_NOZORDER = 0x4, SWP_NOACTIVATE = 0x10;
    const int OWN_W = 1400, OWN_H = 900;

    static readonly WndProcFn proc = Proc;   // kept alive while the windows live
    static IntPtr minWin, fighter, mover, titler;
    static volatile bool fightPending, stop;
    static Thread thread;
    public static long Fights, Moves, Renames;

    static IntPtr Proc(IntPtr h, uint m, IntPtr w, IntPtr l)
    {
        if (m == 0x0024 && h == minWin)   // WM_GETMINMAXINFO: bigger than any tile
        {
            var info = (MINMAXINFO)Marshal.PtrToStructure(l, typeof(MINMAXINFO));
            info.MinTrackSize.X = 2600; info.MinTrackSize.Y = 1500;
            Marshal.StructureToPtr(info, l, false);
            return IntPtr.Zero;
        }
        if (m == 0x0047 && h == fighter)   // WM_WINDOWPOSCHANGED: resized by someone else
        {
            RECT r;
            if (GetWindowRect(h, out r) && (r.R - r.L != OWN_W || r.B - r.T != OWN_H)) fightPending = true;
        }
        return DefWindowProcW(h, m, w, l);
    }

    public static void Start(int seconds)
    {
        stop = false;
        thread = new Thread(() => Run(seconds)) { IsBackground = true };
        thread.Start();
    }

    public static bool Running { get { return thread != null && thread.IsAlive; } }

    public static void Stop() { stop = true; if (thread != null) thread.Join(5000); }

    static void Run(int seconds)
    {
        IntPtr inst = GetModuleHandleW(null);
        var wc = new WNDCLASSEX { cbSize = (uint)Marshal.SizeOf(typeof(WNDCLASSEX)), lpfnWndProc = proc, hInstance = inst, lpszClassName = "LLRefuseTest" };
        RegisterClassExW(ref wc);
        minWin = CreateWindowExW(0, "LLRefuseTest", "LL refuse: minimum size", WS_VISIBLE_OVERLAPPED, 100, 100, OWN_W, OWN_H, IntPtr.Zero, IntPtr.Zero, inst, IntPtr.Zero);
        fighter = CreateWindowExW(0, "LLRefuseTest", "LL refuse: own size", WS_VISIBLE_OVERLAPPED, 140, 140, OWN_W, OWN_H, IntPtr.Zero, IntPtr.Zero, inst, IntPtr.Zero);
        mover = CreateWindowExW(0, "LLRefuseTest", "LL refuse: mover", WS_VISIBLE_OVERLAPPED, 180, 180, 800, 600, IntPtr.Zero, IntPtr.Zero, inst, IntPtr.Zero);
        titler = CreateWindowExW(0, "LLRefuseTest", "LL refuse: title 0", WS_VISIBLE_OVERLAPPED, 220, 220, 800, 600, IntPtr.Zero, IntPtr.Zero, inst, IntPtr.Zero);

        var sw = Stopwatch.StartNew();
        long nextOp = 0, step = 0;
        MSG msg;
        while (!stop && sw.Elapsed.TotalSeconds < seconds)
        {
            while (PeekMessageW(out msg, IntPtr.Zero, 0, 0, 1)) { TranslateMessage(ref msg); DispatchMessageW(ref msg); }
            if (fightPending)
            {
                fightPending = false;
                SetWindowPos(fighter, IntPtr.Zero, 0, 0, OWN_W, OWN_H, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                Fights++;
            }
            // about 2000 moves and 2000 renames a second
            long now = sw.ElapsedTicks * 1000000 / Stopwatch.Frequency;
            if (now >= nextOp)
            {
                nextOp = now + 500;
                step++;
                RECT r;
                if (GetWindowRect(mover, out r))
                {
                    SetWindowPos(mover, IntPtr.Zero, r.L + (step % 2 == 0 ? 1 : -1), r.T, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                    Moves++;
                }
                SetWindowTextW(titler, "LL refuse: title " + step);
                Renames++;
            }
            else Thread.Sleep(0);
        }
        foreach (var h in new[] { minWin, fighter, mover, titler }) DestroyWindow(h);
        while (PeekMessageW(out msg, IntPtr.Zero, 0, 0, 1)) { TranslateMessage(ref msg); DispatchMessageW(ref msg); }
    }
}
'@

# the window manager's IPC: one query on its own connection, its answer time in ms (-1: no answer)
function Ping-Wm {
    $ws = [System.Net.WebSockets.ClientWebSocket]::new()
    $none = [System.Threading.CancellationToken]::None
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        if (-not $ws.ConnectAsync([uri]'ws://127.0.0.1:6123', $none).Wait(5000)) { return -1 }
        $bytes = [Text.Encoding]::UTF8.GetBytes('query paused')
        if (-not $ws.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $none).Wait(5000)) { return -1 }
        $buf = [byte[]]::new(65536)
        if (-not $ws.ReceiveAsync([ArraySegment[byte]]::new($buf), $none).Wait(10000)) { return -1 }
        return [int]$sw.ElapsedMilliseconds
    } catch { return -1 } finally { $ws.Dispose() }
}

function Cpu-Percent($proc, [double]$seconds, [TimeSpan]$before) {
    $proc.Refresh()
    [math]::Round(($proc.TotalProcessorTime - $before).TotalMilliseconds / ($seconds * 10), 1)
}

# ------------------------------------------------------------------ start
$core = Get-ChildItem -Path $Package -Recurse -Filter lunge.exe | Where-Object { $_.Directory.Name -eq 'app' } | Select-Object -First 1
if (-not $core) { throw "No app\lunge.exe under $Package" }
$app = $core.Directory.FullName
$config = Join-Path (Split-Path $app) 'config\config.yaml'
$conf = Join-Path $env:USERPROFILE '.config\logical-lunge'
New-Item -ItemType Directory -Force $conf | Out-Null
if (Test-Path $config) { Copy-Item $config (Join-Path $conf 'config.yaml') -Force }
$logs = Join-Path $env:LOCALAPPDATA 'LogicalLunge\logs'
$tilingLogPath = Join-Path $logs 'tiling.log'
$tilingLogStart = if (Test-Path $tilingLogPath) { (Get-Item $tilingLogPath).Length } else { 0 }
$coreLogPath = Join-Path $logs 'core.log'
$coreLogStart = if (Test-Path $coreLogPath) { (Get-Item $coreLogPath).Length } else { 0 }

$env:LL_TEST = '1'
$coreProc = Start-Process -FilePath $core.FullName -WorkingDirectory $app -PassThru
Note "core started: pid $($coreProc.Id)"
function Part([string]$name) { Get-Process $name -ErrorAction SilentlyContinue | Sort-Object StartTime | Select-Object -First 1 }
$deadline = (Get-Date).AddSeconds($STARTUP_TIMEOUT_SEC)
do { Start-Sleep 2; $tiling = Part 'lunge-tiling' } until ($tiling -or (Get-Date) -gt $deadline -or $coreProc.HasExited)
if (-not $tiling) { Fail 'the window manager did not start' }

$rows = @()
if ($tiling) {
    Start-Sleep $SETTLE_SEC
    $idlePing = Ping-Wm
    $before = $tiling.TotalProcessorTime; Start-Sleep 5
    $idleCpu = Cpu-Percent $tiling 5 $before
    Note "baseline: window manager $idleCpu% of a CPU, IPC answer $idlePing ms"

    [LLRefuse]::Start($STORM_SEC)
    Start-Sleep 3   # the windows are managed (floated, tiled) first
    $tiling.Refresh(); $before = $tiling.TotalProcessorTime; $t = [System.Diagnostics.Stopwatch]::StartNew()
    $pings = New-Object System.Collections.Generic.List[int]
    while ([LLRefuse]::Running -and $t.Elapsed.TotalSeconds -lt ($STORM_SEC - 5)) {
        $pings.Add((Ping-Wm))
        Start-Sleep -Milliseconds 1000
    }
    $stormCpu = Cpu-Percent $tiling $t.Elapsed.TotalSeconds $before
    [LLRefuse]::Stop()
    $missed = @($pings | Where-Object { $_ -lt 0 }).Count
    $worst = if ($pings.Count) { ($pings | Measure-Object -Maximum).Maximum } else { -1 }
    $median = if ($pings.Count) { ($pings | Where-Object { $_ -ge 0 } | Sort-Object)[[int][math]::Floor(($pings.Count - $missed) / 2)] } else { -1 }
    Note "storm: window manager $stormCpu% of a CPU; IPC answers $($pings.Count - $missed)/$($pings.Count), median $median ms, slowest $worst ms; the window resized itself back $([LLRefuse]::Fights) times, moved $([LLRefuse]::Moves), renamed $([LLRefuse]::Renames)"

    if ($stormCpu -gt $MAX_TILING_CPU) { Fail "window manager used $stormCpu% of a CPU against refusing windows (max $MAX_TILING_CPU)" }
    if ($missed -gt 0) { Fail "$missed of $($pings.Count) IPC queries got no answer during the storm" }
    if ($worst -gt $MAX_PING_MS) { Fail "slowest IPC answer during the storm $worst ms (max $MAX_PING_MS)" }
    $now = Part 'lunge-tiling'
    if (-not $now -or $now.Id -ne $tiling.Id) { Fail 'the window manager was restarted or ended during the storm' }
    $rows = @(
        "| Window manager CPU (% of one CPU) | $idleCpu | $stormCpu | $MAX_TILING_CPU |",
        "| IPC answer (ms) | $idlePing | median $median, slowest $worst, missed $missed | $MAX_PING_MS |",
        "| Self-resizes / moves / renames |  | $([LLRefuse]::Fights) / $([LLRefuse]::Moves) / $([LLRefuse]::Renames) |  |"
    )
}

# the text a log got since this test started
function Log-Since([string]$path, [long]$start) {
    if (-not (Test-Path $path)) { return @() }
    $fs = [IO.File]::Open($path, 'Open', 'Read', 'ReadWrite, Delete')
    try { $null = $fs.Seek([math]::Min($start, $fs.Length), 'Begin'); $text = (New-Object IO.StreamReader($fs, [Text.Encoding]::UTF8)).ReadToEnd() } finally { $fs.Dispose() }
    $text -split "`r?`n"
}

# the window manager's own notes on the storm (stall watch, correction budget)
foreach ($l in (Log-Since $tilingLogPath $tilingLogStart)) {
    if ($l -match 'panicked') { Fail "window manager: $l" }
    elseif ($l -match 'busy for|Slow step|event storm|keeps moving itself|can''t take its tile|bigger than its tile') { Note "tiling.log: $l" }
}
foreach ($l in (Log-Since $coreLogPath $coreLogStart)) {
    if ($l -match 'tiling nöbetçisi: .*(yanıt vermi|yanıt vermedi|beklenmedik)') { Fail "core watchdog: $l" }
}

# ------------------------------------------------------------------ report
$lines = @(
    "## Refusing windows",
    "",
    "| Check | Idle | Storm | Limit |",
    "|---|---|---|---|"
) + $rows + @("") + @($failures | ForEach-Object { "- **failed:** $_" })
$lines | ForEach-Object { Write-Host $_ }
if ($Report) { $lines | Add-Content -Path $Report -Encoding UTF8 }

try { & $core.FullName --shutdown } catch { }
if ($failures.Count) { Write-Host "$($failures.Count) check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host 'refusing windows test passed' -ForegroundColor Green
