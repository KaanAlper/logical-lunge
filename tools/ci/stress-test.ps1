# Logical Lunge - CI smoke and stress test of a built edition package.
#
# Runs the core (which brings up the window manager and the shell, as the sign-in task does) straight from the package's
# app folder, with no install, then keeps the desktop busy: workspace switches through the core's test pipe
# (\\.\pipe\lunge-test, opened only with LL_TEST=1), synthetic key and mouse input through the input hooks, a few
# minutes idle and a few minutes with every core busy and memory under pressure (a heavy game's footprint), then a churn
# of hundreds of workspace switches, window open/close cycles, popups and fullscreen toggles. It fails when the desktop
# does not hold up: a part restarted or crashed, the bar stopped sending heartbeats, the keyboard hook got slow or was
# dropped, slide frames got too long or slower over time, or handles / GDI / memory / windows kept growing.
#
#   pwsh tools/ci/stress-test.ps1 -Package <folder with app\lunge.exe>
param(
    [Parameter(Mandatory = $true)][string]$Package,
    [string]$Report = $env:GITHUB_STEP_SUMMARY
)
$ErrorActionPreference = 'Stop'

# ------------------------------------------------------------------ thresholds
$STARTUP_TIMEOUT_SEC = 180   # core + window manager + shell + bar windows up
$WARMUP_SEC          = 60    # settle before the baseline (startup screen, first caches)
$IDLE_SEC            = 180   # idle phase: switches and input, nothing else running
$LOAD_SEC            = 180   # load phase: every logical CPU busy at normal priority + memory pressure
$SWITCH_EVERY_SEC    = 6     # one workspace switch (there and back) this often
$MEMORY_PRESSURE     = 0.75  # the load phase takes this share of the free memory it starts with
# GitHub's runners have no GPU: DWM and our surfaces render on WARP (software), so frame budgets are loose; they still
# catch a frame that waits on a stuck call (hundreds of ms).
$MAX_FRAME_IDLE_MS   = 100   # longest slide/animation frame while idle
$MAX_FRAME_LOAD_MS   = 200   # longest frame with every core busy
# the core's own share of a frame (update + gap + waking after DWM composed), apart from DWM's composing; under load the
# 300-500 ms frames were the core re-raising its layer every fourth frame (fixed: only when something rose over it)
$MAX_OURS_IDLE_MS    = 40
$MAX_OURS_LOAD_MS    = 80
# Super+F (a window's state) freezes the screen on the core's UI thread before the command goes to the window manager:
# key to command ("donma+komut"). A user's 508 ms was the freeze's border step stuck in DWM (now on its own thread, waited
# at most 30 ms). Toggled on and off every $STATE_EVERY_SEC s while idle and under load, and over the state churn.
$STATE_EVERY_SEC     = 12
$MAX_STATE_IDLE_MS   = 150
$MAX_STATE_LOAD_MS   = 300
$MAX_HOOK_SLOW_IDLE  = 0     # "klavye kancası yavaş" (>100 ms in the keyboard hook) lines while idle
$MAX_HOOK_SLOW_LOAD  = 3     # ... under load (Windows drops a hook after repeated ~300 ms timeouts)
# growth from the end of warm-up to the end of the run, per part (core, shell, window manager). A one-time
# initialisation after warm-up (e.g. the first WMI query loads its COM plumbing: ~250 handles, ~12 MB once) fits
# under these; a leak is caught by the steady-growth rate below instead.
$MAX_HANDLE_GROWTH   = 400
# steady growth: the median of the 30 s steps between samples, per minute. A single jump moves one step only; a
# leak moves them all (0.7 handles/s is ~42/min).
$SAMPLE_EVERY_SEC    = 30
$MAX_HANDLE_RATE     = 8     # handles per minute
$MAX_PRIVATE_MB_RATE = 2     # MB per minute
$MAX_GDI_GROWTH      = 100
$MAX_USER_GROWTH     = 100
$MAX_PRIVATE_MB_GROWTH = 200
# churn: many workspace switches and window open/close cycles after the load phase; anything a switch or a window
# leaves behind (overlay windows, thumbnails, visuals, timers, tasks, tree nodes) shows up as growth per operation.
# Growth is the median step between consecutive samples, scaled to 100 operations: a one-time jump (a first WMI query
# loads ~250 handles of COM plumbing once) moves a single step, a leak moves them all.
$CHURN_SWITCHES      = 320   # workspace switches (ws-1 .. ws-4 in turn)
$CHURN_WINDOW_CYCLES = 100   # open two windows, close them
$CHURN_SAMPLE_EVERY  = 10    # operations between samples
$MAX_CHURN_HANDLES_PER_100 = 40
$MAX_CHURN_MB_PER_100      = 6
$MAX_CHURN_GDI_PER_100     = 4
$MAX_CHURN_USER_PER_100    = 6
$MAX_CHURN_THREADS_PER_100 = 4
$MAX_CHURN_WINDOWS_PER_100 = 1    # top-level windows a part owns
# Thread pools (Windows' own, the runtimes') add workers under a burst and retire them once idle, so a thread count
# that climbs during a churn is a leak only if the threads are still there after a quiet pause: threads above the
# count before the churn, after at least $CHURN_SETTLE_SEC s of quiet. While a part still holds more, the pause goes
# on (sampled every $CHURN_SETTLE_POLL_SEC s) up to $CHURN_SETTLE_MAX_SEC s: Windows retires idle pool workers in
# batches, not all at once (the window manager's WARP workers on a runner without a GPU dropped by 3-8 at a time,
# 45-165 s after the window churn).
$CHURN_SETTLE_SEC = 45
$CHURN_SETTLE_POLL_SEC = 15
$CHURN_SETTLE_MAX_SEC = 180
$MAX_CHURN_THREADS_SETTLED = 6    # web-ui's core kept 4 after the quiet pause in a passing run
# slides must not get slower as operations pile up: median longest-frame of the last third vs the first third
$MAX_CHURN_FRAME_SLOWDOWN_MS = 40
$MAX_CHURN_THUMB_GROWTH = 8        # registered DWM thumbnails, last third vs first third of the churn

# Diagnostics instead of the checks (LL_TRACE_THREADPOOL=1, the Build workflow's trace_threadpool input): no idle or
# load phase, the window churn recorded with Windows Performance Recorder (threadpool-trace\ll-threadpool.wprp), then
# threadpool-trace\Program.cs names who queued work to the window manager's thread pool and who created its threads.
# Writes threadpool-trace-out\ (the .etl, summary.txt, details.txt) and exits without judging anything.
$TRACE_THREADPOOL = $env:LL_TRACE_THREADPOOL -eq '1'
if ($TRACE_THREADPOOL) { $IDLE_SEC = 0; $LOAD_SEC = 0 }

$failures = New-Object System.Collections.Generic.List[string]
$skipped = New-Object System.Collections.Generic.List[string]
function Fail([string]$m) { $failures.Add($m); Write-Host "FAIL: $m" -ForegroundColor Red }
function Skip([string]$m) { $skipped.Add($m); Write-Host "SKIP: $m" -ForegroundColor Yellow }
function Note([string]$m) { Write-Host $m }

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class LLStress {
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr desk);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    public struct POINT { public int X, Y; }
    // a lone Shift tap: goes through the low-level keyboard hook, triggers no shortcut
    public static void TapShift() { keybd_event(0x10, 0, 0, UIntPtr.Zero); keybd_event(0x10, 0, 2, UIntPtr.Zero); }
    // a real relative move, as a mouse sends it: the low-level mouse hook is called for it (a SetCursorPos warp
    // reaches no hook, so the mouse hook was never exercised and a dead one could not show)
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);
    public static void Nudge(int dx) { mouse_event(0x0001, dx, 0, 0, UIntPtr.Zero); }
    public static bool InteractiveDesktop() { IntPtr d = OpenInputDesktop(0, false, 0x0001); if (d == IntPtr.Zero) return false; CloseDesktop(d); return true; }
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    // top-level windows (visible or not) owned by a process: a window created per operation and never destroyed
    public static int TopLevelWindows(int pid) {
        int n = 0;
        EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == (uint)pid) n++; return true; }, IntPtr.Zero);
        return n;
    }
    // a process's threads grouped by what they are: the thread's name (SetThreadDescription; Rust's named threads), else
    // the module and offset its start routine is in (a pool's workers share one), so the group that grows names the leak
    [DllImport("kernel32.dll")] static extern IntPtr OpenThread(uint access, bool inherit, uint id);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern int GetThreadDescription(IntPtr thread, out IntPtr desc);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
    [DllImport("ntdll.dll")] static extern int NtQueryInformationThread(IntPtr thread, int cls, out IntPtr info, int len, IntPtr ret);
    public static System.Collections.Generic.Dictionary<string, int> ThreadGroups(int pid) {
        var groups = new System.Collections.Generic.Dictionary<string, int>();
        System.Diagnostics.Process p;
        try { p = System.Diagnostics.Process.GetProcessById(pid); } catch { return groups; }
        var mods = new System.Collections.Generic.List<System.Diagnostics.ProcessModule>();
        try { foreach (System.Diagnostics.ProcessModule m in p.Modules) mods.Add(m); } catch { }
        foreach (System.Diagnostics.ProcessThread t in p.Threads) {
            string label = null;
            IntPtr h = OpenThread(0x0040 | 0x0800, false, (uint)t.Id); // THREAD_QUERY_INFORMATION | THREAD_QUERY_LIMITED_INFORMATION
            if (h != IntPtr.Zero) {
                IntPtr d;
                if (GetThreadDescription(h, out d) >= 0 && d != IntPtr.Zero) { string s = Marshal.PtrToStringUni(d); LocalFree(d); if (!string.IsNullOrEmpty(s)) label = "name:" + s; }
                if (label == null) {
                    IntPtr start;
                    if (NtQueryInformationThread(h, 9, out start, IntPtr.Size, IntPtr.Zero) == 0) { // ThreadQuerySetWin32StartAddress
                        long a = start.ToInt64();
                        foreach (var m in mods) {
                            long b = m.BaseAddress.ToInt64();
                            if (a >= b && a < b + m.ModuleMemorySize) { label = m.ModuleName.ToLowerInvariant() + "+0x" + (a - b).ToString("x"); break; }
                        }
                        if (label == null) label = "start:0x" + a.ToString("x");
                    }
                }
                CloseHandle(h);
            }
            if (label == null) label = "unknown";
            int n; groups.TryGetValue(label, out n); groups[label] = n + 1;
        }
        return groups;
    }
    public static int VisibleWindowsTitled(string title) {
        int n = 0; IntPtr h = IntPtr.Zero;
        while ((h = FindWindowEx(IntPtr.Zero, h, null, title)) != IntPtr.Zero) if (IsWindowVisible(h)) n++;
        return n;
    }
}
'@

# ------------------------------------------------------------------ start
$core = Get-ChildItem -Path $Package -Recurse -Filter lunge.exe | Where-Object { $_.Directory.Name -eq 'app' } | Select-Object -First 1
if (-not $core) { throw "No app\lunge.exe under $Package" }
$app = $core.Directory.FullName
$config = Join-Path (Split-Path $app) 'config\config.yaml'
$conf = Join-Path $env:USERPROFILE '.config\logical-lunge'
New-Item -ItemType Directory -Force $conf | Out-Null
if (Test-Path $config) { Copy-Item $config (Join-Path $conf 'config.yaml') -Force }   # what the installer writes
$logs = Join-Path $env:LOCALAPPDATA 'LogicalLunge\logs'

$session = [System.Diagnostics.Process]::GetCurrentProcess().SessionId
$interactive = $session -ne 0 -and [LLStress]::InteractiveDesktop() -and (Get-Process explorer -ErrorAction SilentlyContinue)
Note "session $session, interactive desktop: $([bool]$interactive), logical CPUs: $([Environment]::ProcessorCount)"

$env:LL_TEST = '1'
$coreProc = Start-Process -FilePath $core.FullName -WorkingDirectory $app -PassThru
Note "core started: pid $($coreProc.Id)"

function Part([string]$name) { Get-Process $name -ErrorAction SilentlyContinue | Sort-Object StartTime | Select-Object -First 1 }
$deadline = (Get-Date).AddSeconds($STARTUP_TIMEOUT_SEC)
do {
    Start-Sleep 2
    $tiling = Part 'lunge-tiling'; $shell = Part 'lunge-shell'
    $bars = if ($interactive) { [LLStress]::VisibleWindowsTitled('Logical Lunge · bar') } else { 0 }
} until (($tiling -and $shell -and ($bars -gt 0 -or -not $interactive)) -or (Get-Date) -gt $deadline -or $coreProc.HasExited)
if ($coreProc.HasExited) { Fail "the core exited during startup (code $($coreProc.ExitCode))" }
if (-not $tiling) { Fail 'the window manager did not start' }
if (-not $shell) { Fail 'the shell did not start' }
if ($interactive -and $bars -eq 0) { Fail 'no bar window appeared' }
if (-not $interactive) { Skip 'no interactive desktop on this runner: bar, heartbeat and frame checks are skipped' }

# windows to slide around
$notepads = @(1..3 | ForEach-Object { Start-Process notepad -PassThru })

function Send-Test([string]$line) {
    try {
        $c = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'lunge-test', [System.IO.Pipes.PipeDirection]::Out)
        $c.Connect(3000)
        $w = New-Object System.IO.StreamWriter($c); $w.WriteLine($line); $w.Flush(); $c.Dispose()
    } catch { Note "test pipe: $($_.Exception.Message)" }
}

$pids = @{ core = $coreProc.Id; tiling = $tiling.Id; shell = $shell.Id }
# The core is a .NET process: closed sockets and waits keep their handles until a collection runs, so its count moves
# with the GC's timing. It collects first ("gc" on the test pipe), and what is left is what it really holds.
function Sample {
    Send-Test 'gc'; Start-Sleep -Milliseconds 300
    $r = @{}
    foreach ($k in $pids.Keys) {
        $p = Get-Process -Id $pids[$k] -ErrorAction SilentlyContinue
        if (-not $p) { $r[$k] = $null; continue }
        $r[$k] = [pscustomobject]@{ Handles = $p.HandleCount; PrivateMB = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
            Gdi = [LLStress]::GetGuiResources($p.Handle, 0); User = [LLStress]::GetGuiResources($p.Handle, 1)
            Threads = $p.Threads.Count; Windows = [LLStress]::TopLevelWindows($p.Id) }
    }
    $r
}

# samples every $SAMPLE_EVERY_SEC s from the end of warm-up (steady-growth check)
$script:series = New-Object System.Collections.Generic.List[object]
$script:sampling = $false

# one second of the run: input through the hooks, a workspace switch now and then, the parts still the same processes
$script:tick = 0
function Run-Phase([int]$seconds) {
    $end = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $end) {
        $script:tick++
        if ($interactive) { [LLStress]::TapShift(); [LLStress]::Nudge($(if ($script:tick % 2) { 3 } else { -3 })) }
        if ($script:tick % $SWITCH_EVERY_SEC -eq 0) { Send-Test 'ws-2'; Start-Sleep -Milliseconds 700; Send-Test 'ws-1' }
        if ($script:tick % $STATE_EVERY_SEC -eq 3) { Send-Test 'wm toggle-fullscreen'; Start-Sleep -Milliseconds 800; Send-Test 'wm toggle-fullscreen' }
        foreach ($k in @($pids.Keys)) {
            if (-not (Get-Process -Id $pids[$k] -ErrorAction SilentlyContinue)) {
                Fail "$k (pid $($pids[$k])) ended at $(Get-Date -Format HH:mm:ss)"
                $pids.Remove($k)
            }
        }
        if ($script:sampling -and $script:tick % $SAMPLE_EVERY_SEC -eq 0) { $script:series.Add((Sample)) }
        Start-Sleep -Milliseconds 1000
    }
}

$t0 = Get-Date
Run-Phase $WARMUP_SEC
$base = Sample
$script:series.Add($base); $script:sampling = $true
$tIdle = Get-Date
Run-Phase $IDLE_SEC
$tLoad = Get-Date

# ------------------------------------------------------------------ load
$freeMB = [math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1KB)
$hogMB = [int]($freeMB * $MEMORY_PRESSURE)
Note "load: $([Environment]::ProcessorCount) busy processes, $hogMB MB of $freeMB MB free memory held"
$load = @(1..[Environment]::ProcessorCount | ForEach-Object {
    Start-Process pwsh -ArgumentList '-NoProfile', '-Command', 'while ($true) { }' -WindowStyle Hidden -PassThru })
$hogScript = "`$l = New-Object System.Collections.Generic.List[byte[]]; for (`$i = 0; `$i -lt [int]($hogMB / 64); `$i++) { `$b = New-Object byte[] (64MB); for (`$j = 0; `$j -lt `$b.Length; `$j += 4096) { `$b[`$j] = 1 }; `$l.Add(`$b) }; Start-Sleep 100000"
$load += Start-Process pwsh -ArgumentList '-NoProfile', '-Command', $hogScript -WindowStyle Hidden -PassThru
# LL_STRESS_TRACE=1 (workflow input "trace"): an ETW trace of the whole load phase (CPU samples, context switches and
# ready-thread events with stacks) for tools/ci/trace-analysis, which tells where a long slide frame went. It is stopped
# (and merged, which takes minutes) only after the load processes are gone, so the load phase itself is not cut.
$traceOn = $env:LL_STRESS_TRACE -eq '1'
$traceEtl = Join-Path ([IO.Path]::GetTempPath()) 'stress-load.etl'
if ($traceOn) { & wpr -start CPU -filemode | Out-Null; if ($LASTEXITCODE -ne 0) { Note "trace: wpr -start failed ($LASTEXITCODE)"; $traceOn = $false } }
try { Run-Phase $LOAD_SEC }
finally
{
    $load | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    if ($traceOn)
    {
        & wpr -stop $traceEtl | Out-Null
        Note "trace: $traceEtl ($([math]::Round((Get-Item $traceEtl).Length / 1MB)) MB), the whole load phase"
    }
}
$tEnd = Get-Date
Run-Phase 20   # back to calm: anything that broke under load shows up here
$final = Sample
$script:series.Add($final); $script:sampling = $false

# ------------------------------------------------------------------ churn
# Each kind of operation on its own, sampled every $CHURN_SAMPLE_EVERY operations: what a kind leaves behind shows as a
# slope against the operation count, and the kind that leaks is named.
function Alive-Check { foreach ($k in @($pids.Keys)) { if (-not (Get-Process -Id $pids[$k] -ErrorAction SilentlyContinue)) { Fail "$k (pid $($pids[$k])) ended during churn at $(Get-Date -Format HH:mm:ss)"; $pids.Remove($k) } } }
# $warm operations first, unsampled: the first open of a panel loads what it keeps for good (fonts, images, the quick
# settings' radios and Bluetooth), which is not growth per operation
function Churn([string]$kind, [int]$ops, [scriptblock]$op, [int]$warm = 4) {
    for ($i = 1; $i -le $warm; $i++) { & $op $i }
    Start-Sleep 2
    $s = New-Object System.Collections.Generic.List[object]
    $from = Get-Date
    $s.Add([pscustomobject]@{ Op = 0; Sample = (Sample) })
    for ($i = 1; $i -le $ops; $i++) {
        & $op $i
        if ($i % $CHURN_SAMPLE_EVERY -eq 0 -or $i -eq $ops) { Alive-Check; $s.Add([pscustomobject]@{ Op = $i; Sample = (Sample) }) }
    }
    $to = Get-Date
    Note "churn ${kind}: $ops operations in $([math]::Round(($to - $from).TotalSeconds)) s"
    foreach ($k in @('core', 'shell', 'tiling')) {
        Note ("  $k " + (($s | Where-Object { $_.Sample[$k] } | ForEach-Object { $x = $_.Sample[$k]; "$($_.Op):h$($x.Handles)/t$($x.Threads)/u$($x.User)/w$($x.Windows)/$($x.PrivateMB)MB" }) -join ' '))
    }
    [pscustomobject]@{ Kind = $kind; Ops = $ops; From = $from; To = $to; Series = $s }
}
# which threads a churn leaves: the groups (name or start module) whose count changed between two snapshots
function Thread-Snapshot { $r = @{}; foreach ($k in $pids.Keys) { $r[$k] = [LLStress]::ThreadGroups($pids[$k]) }; $r }
function Thread-Diff([string]$what, $a, $b) {
    foreach ($k in @('core', 'shell', 'tiling')) {
        if (-not $a[$k] -or -not $b[$k]) { continue }
        $keys = @($a[$k].Keys) + @($b[$k].Keys) | Sort-Object -Unique
        $d = foreach ($g in $keys) { $x = if ($a[$k].ContainsKey($g)) { $a[$k][$g] } else { 0 }; $y = if ($b[$k].ContainsKey($g)) { $b[$k][$g] } else { 0 }; if ($x -ne $y) { "$g $x->$y" } }
        if ($d) { Note "threads ${what}: $k $($d -join ', ')" }
    }
}
# open two windows, close them
$windowCycle = {
    $w = @(1..2 | ForEach-Object { Start-Process notepad -PassThru })
    Start-Sleep -Milliseconds 700
    $w | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
}
if ($TRACE_THREADPOOL) {
    $out = Join-Path (Get-Location) 'threadpool-trace-out'
    New-Item -ItemType Directory -Force $out | Out-Null
    $etl = Join-Path $out 'window-churn.etl'
    $wprp = Join-Path $PSScriptRoot 'threadpool-trace\ll-threadpool.wprp'
    Send-Test 'ws-1'; Start-Sleep 2
    $before = Thread-Snapshot
    & wpr -cancel 2>$null | Out-Null
    & wpr -start "$wprp!LLThreadPool" -filemode
    if ($LASTEXITCODE) { throw "wpr -start failed ($LASTEXITCODE)" }
    $traced = Churn 'window' $CHURN_WINDOW_CYCLES $windowCycle
    & wpr -stop $etl 'Logical Lunge window churn'
    if ($LASTEXITCODE) { throw "wpr -stop failed ($LASTEXITCODE)" }
    Thread-Diff 'over the window churn' $before (Thread-Snapshot)
    $peak = ($traced.Series | ForEach-Object { $_.Sample['tiling'].Threads } | Measure-Object -Maximum).Maximum
    Start-Sleep $CHURN_SETTLE_SEC
    Thread-Diff 'kept after the window churn and the quiet pause' $before (Thread-Snapshot)
    $settled = Sample
    Note "window churn: tiling threads $($traced.Series[0].Sample['tiling'].Threads) before, peak $peak, $($settled['tiling'].Threads) after $CHURN_SETTLE_SEC s of quiet"
    Note "trace: $([math]::Round((Get-Item $etl).Length / 1MB, 1)) MB"
    try { & $core.FullName --shutdown } catch { }
    $notepads | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    $symbols = Join-Path (Get-Location) 'tiling-pdb'
    dotnet run --project (Join-Path $PSScriptRoot 'threadpool-trace') -c Release -- $etl lunge-tiling $symbols $out
    if ($LASTEXITCODE) { throw "thread pool trace analysis failed ($LASTEXITCODE)" }
    if ($Report) { @('## Thread pool trace (window churn)', '', '```') + (Get-Content (Join-Path $out 'summary.txt')) + @('```') | Add-Content -Path $Report -Encoding UTF8 }
    # the .etl is kept with the summary unless it is too big for a handy artifact
    if ((Get-Item $etl).Length -gt 400MB) { Remove-Item $etl; Note 'trace: not kept (over 400 MB); summary.txt and details.txt are' }
    exit 0
}
$threadsBefore = Thread-Snapshot
$churn = @()
$churn += Churn 'switch' $CHURN_SWITCHES { param($i) Send-Test "ws-$((($i - 1) % 4) + 1)"; Start-Sleep -Milliseconds 850 }
Send-Test 'ws-1'; Start-Sleep 1
$threadsBeforeWindows = Thread-Snapshot
$churn += Churn 'window' $CHURN_WINDOW_CYCLES $windowCycle
Thread-Diff 'over the window churn' $threadsBeforeWindows (Thread-Snapshot)
# overview and sidebar opened and closed in turn
$churn += Churn 'popup' 60 { param($i)
    $a = if ($i % 2) { 'overview' } else { 'sidebar' }
    Send-Test $a; Start-Sleep -Milliseconds 600; Send-Test $a; Start-Sleep -Milliseconds 500
}
# Super+F on the focused window: the freeze/snapshot animation (an even count leaves it as it was)
$churn += Churn 'state' 60 { Send-Test 'wm toggle-fullscreen'; Start-Sleep -Milliseconds 800 }
Start-Sleep $CHURN_SETTLE_SEC
$churnBefore = $churn[0].Series[0].Sample
$churnSettled = Sample
# the parts still holding more threads than before the churn (beyond what a quiet pause must give back)
function Holding { @('core', 'shell', 'tiling') | Where-Object { $churnBefore[$_] -and $churnSettled[$_] -and $churnSettled[$_].Threads - $churnBefore[$_].Threads -gt $MAX_CHURN_THREADS_SETTLED } }
$quiet = $CHURN_SETTLE_SEC
$quietThreads = @{}
foreach ($k in @('core', 'shell', 'tiling')) { if ($churnSettled[$k]) { $quietThreads[$k] = @("${quiet}s:$($churnSettled[$k].Threads)") } }
while ($quiet -lt $CHURN_SETTLE_MAX_SEC -and @(Holding).Count) {
    Start-Sleep $CHURN_SETTLE_POLL_SEC
    $quiet += $CHURN_SETTLE_POLL_SEC
    $churnSettled = Sample
    foreach ($k in @($quietThreads.Keys)) { if ($churnSettled[$k]) { $quietThreads[$k] += "${quiet}s:$($churnSettled[$k].Threads)" } }
}
if ($quiet -gt $CHURN_SETTLE_SEC) { foreach ($k in @('core', 'shell', 'tiling')) { if ($quietThreads[$k]) { Note "threads over the quiet pause: $k $($quietThreads[$k] -join ' ')" } } }
Thread-Diff 'kept after the churn and the quiet pause' $threadsBefore (Thread-Snapshot)
foreach ($k in @('core', 'shell', 'tiling')) {
    if ($churnBefore[$k] -and $churnSettled[$k]) { Note "after the churn and $quiet s of quiet: $k threads $($churnBefore[$k].Threads) -> $($churnSettled[$k].Threads), handles $($churnBefore[$k].Handles) -> $($churnSettled[$k].Handles), private $($churnBefore[$k].PrivateMB) -> $($churnSettled[$k].PrivateMB) MB" }
}

# ------------------------------------------------------------------ checks
function Read-Log([string]$name) { $p = Join-Path $logs $name; if (Test-Path $p) { Get-Content $p -Encoding UTF8 } else { @() } }
$coreLog = Read-Log 'core.log'; $shellLog = Read-Log 'shell.log'; $tilingLog = Read-Log 'tiling.log'

# core.log: "HH:mm:ss.fff text", a "---- yyyy-MM-dd ----" line when the day changes
$day = $t0.Date
$timed = foreach ($l in $coreLog) {
    if ($l -match '^---- (\d{4}-\d{2}-\d{2}) ----') { $day = [datetime]::ParseExact($Matches[1], 'yyyy-MM-dd', $null); continue }
    if ($l -match '^(\d{2}):(\d{2}):(\d{2})\.(\d{3}) (.*)$') {
        [pscustomobject]@{ At = $day.AddHours([int]$Matches[1]).AddMinutes([int]$Matches[2]).AddSeconds([int]$Matches[3]); Text = $Matches[5] }
    }
}
$run = @($timed | Where-Object { $_.At -ge $t0.AddSeconds(-5) })
function In-Phase($from, $to) { @($run | Where-Object { $_.At -ge $from -and $_.At -lt $to }) }

# restarts and crashes (the shell's first start at bring-up is logged with "açılış")
foreach ($e in $run) {
    if ($e.Text -match 'CRASH:|UI HATA:|bilinmeyen komut') { Fail "core: $($e.Text)" }
    elseif ($e.Text -match 'shell nöbetçisi:' -and $e.Text -notmatch 'açılış') { Fail "watchdog acted on a healthy run: $($e.Text)" }
    elseif ($e.Text -match 'nöbetçisi.*yeniden|yeniden başlatıl|kendini yeniden') { Fail "restart: $($e.Text)" }
}
foreach ($l in $shellLog) { if ($l -match 'panic|Native bar failed|Native bar stopped|died at its last starts') { Fail "shell: $l" } }
foreach ($l in $tilingLog) { if ($l -match 'panicked') { Fail "window manager: $l" } }

# input hooks
$hookIdle = @(In-Phase $tIdle $tLoad | Where-Object { $_.Text -match 'klavye kancası yavaş' }).Count
$hookLoad = @(In-Phase $tLoad $tEnd | Where-Object { $_.Text -match 'klavye kancası yavaş' }).Count
if ($hookIdle -gt $MAX_HOOK_SLOW_IDLE) { Fail "keyboard hook slow $hookIdle times while idle (max $MAX_HOOK_SLOW_IDLE)" }
if ($hookLoad -gt $MAX_HOOK_SLOW_LOAD) { Fail "keyboard hook slow $hookLoad times under load (max $MAX_HOOK_SLOW_LOAD)" }
foreach ($e in $run) { if ($e.Text -match 'kancası girdi görmüyordu|sınama girdisini görmedi') { Fail "input hook dropped by Windows: $($e.Text)" } }
# the mouse is moved every second of the run: the native mouse hook must have counted those moves
if ($interactive) {
    $mouseSeen = 0
    foreach ($e in $run) { if ($e.Text -match 'hooks native key \d+ .*?mouse (\d+)') { $mouseSeen += [int]$Matches[1] } }
    if ($mouseSeen -eq 0) { Fail 'the mouse hook counted no input during the run (it was moved every second)' } else { Note "mouse hook calls counted in the core's reports: $mouseSeen" }
}

# the core's own handle/thread/connection report (LL_TEST=1, every 30 s): printed for diagnosis
foreach ($e in @($run | Where-Object { $_.Text -match '^test: handles ' })) { Note "$($e.At.ToString('HH:mm:ss')) $($e.Text)" }

# bar heartbeats (core logs "test: bars <windows> alive <recent>" every 30 s with LL_TEST=1)
$beats = @(In-Phase $tIdle $tEnd.AddSeconds(20) | Where-Object { $_.Text -match '^test: bars (\d+) alive (\d+)' } | ForEach-Object {
    $null = $_.Text -match '^test: bars (\d+) alive (\d+)'; [pscustomobject]@{ At = $_.At; Windows = [int]$Matches[1]; Alive = [int]$Matches[2] } })
if ($interactive) {
    if ($beats.Count -eq 0) { Fail 'no heartbeat report from the core (LL_TEST=1 reporting missing)' }
    foreach ($b in $beats) {
        if ($b.Windows -eq 0) { Fail "no bar window at $($b.At.ToString('HH:mm:ss'))" }
        elseif ($b.Alive -lt $b.Windows) { Fail "bars silent at $($b.At.ToString('HH:mm:ss')): $($b.Alive)/$($b.Windows) sent a heartbeat in 45 s" }
    }
}

# frames of slides and animations
function Longest($entries) { $m = 0; foreach ($e in $entries) { if ($e.Text -match 'en uzun kare (\d+) ms') { $m = [math]::Max($m, [int]$Matches[1]) } }; $m }
$slidesIdle = @(In-Phase $tIdle $tLoad | Where-Object { $_.Text -match '^(slide|anim)' })
$slidesLoad = @(In-Phase $tLoad $tEnd | Where-Object { $_.Text -match '^(slide|anim)' })
$frameIdle = Longest $slidesIdle; $frameLoad = Longest $slidesLoad
if ($interactive) {
    if ($slidesIdle.Count -eq 0) { Fail 'no slide happened while idle (test pipe not working?)' }
    if ($frameIdle -gt $MAX_FRAME_IDLE_MS) { Fail "longest frame while idle $frameIdle ms (max $MAX_FRAME_IDLE_MS)" }
    if ($frameLoad -gt $MAX_FRAME_LOAD_MS) { Fail "longest frame under load $frameLoad ms (max $MAX_FRAME_LOAD_MS)" }
}
# "bizim payımız en çok 12.5 ms, DWM'in en uzun kompozisyon beklemesi 480.2 ms" (cores that print the split)
function Share($entries, [string]$what) {
    $m = -1
    foreach ($e in $entries) { if ($e.Text -match "$what (\d+)[.,]?(\d*) ms") { $m = [math]::Max($m, [double]("$($Matches[1]).$(if ($Matches[2]) { $Matches[2] } else { '0' })")) } }
    $m
}
$oursIdle = Share $slidesIdle 'bizim payımız en çok'; $oursLoad = Share $slidesLoad 'bizim payımız en çok'
# Super+F: key to command ("durum değişti: toggle-fullscreen, donma+komut 21 ms"), and the longest freeze with its steps
# ("donma adımları (ms, birikimli): boyut 0.0 duvar 0.0 kayıt 3.6 kenar 4.8 göster 7.0 flush 16.8 | ...")
function StateMs($entries) { $m = -1; foreach ($e in $entries) { if ($e.Text -match '^durum değişti: .*donma\+komut (\d+) ms') { $m = [math]::Max($m, [int]$Matches[1]) } }; $m }
function FreezeWorst($entries) {
    $worst = '-'; $wt = -1.0
    foreach ($e in $entries) {
        if ($e.Text -notmatch '^donma adımları') { continue }
        $prev = 0.0; $parts = @()
        foreach ($m in [regex]::Matches(($e.Text -split '\|')[0], '(\p{L}+) (\d+)[.,](\d+)')) {
            $v = [double]"$($m.Groups[2].Value).$($m.Groups[3].Value)"
            $parts += "$($m.Groups[1].Value) $([math]::Round($v - $prev, 1))"; $prev = $v
        }
        if ($prev -gt $wt) { $wt = $prev; $worst = "$([math]::Round($prev)) ($($parts -join ', '))" }
    }
    $worst
}
$stateIdle = StateMs (In-Phase $tIdle $tLoad); $stateLoad = StateMs (In-Phase $tLoad $tEnd)
$freezeIdle = FreezeWorst (In-Phase $tIdle $tLoad); $freezeLoad = FreezeWorst (In-Phase $tLoad $tEnd)
$stc = $churn | Where-Object { $_.Kind -eq 'state' } | Select-Object -First 1
$stateChurn = if ($stc) { "Super+F over the state churn: key to command at most $(StateMs (In-Phase $stc.From $stc.To)) ms, longest freeze $(FreezeWorst (In-Phase $stc.From $stc.To)) ms." } else { '' }
if ($interactive -and $stateIdle -ge 0) {
    if ($stateIdle -gt $MAX_STATE_IDLE_MS) { Fail "Super+F key to command while idle $stateIdle ms (max $MAX_STATE_IDLE_MS)" }
    if ($stateLoad -gt $MAX_STATE_LOAD_MS) { Fail "Super+F key to command under load $stateLoad ms (max $MAX_STATE_LOAD_MS)" }
}
$dwmIdle = Share $slidesIdle "DWM'in en uzun kompozisyon beklemesi"; $dwmLoad = Share $slidesLoad "DWM'in en uzun kompozisyon beklemesi"
if ($interactive -and $oursIdle -ge 0) {
    if ($oursIdle -gt $MAX_OURS_IDLE_MS) { Fail "the core's share of a frame while idle $oursIdle ms (max $MAX_OURS_IDLE_MS)" }
    if ($oursLoad -gt $MAX_OURS_LOAD_MS) { Fail "the core's share of a frame under load $oursLoad ms (max $MAX_OURS_LOAD_MS)" }
}

# growth
function Median([double[]]$v) { if (-not $v -or $v.Count -eq 0) { return 0 }; $o = $v | Sort-Object; $n = $o.Count; if ($n % 2) { $o[[int](($n - 1) / 2)] } else { ($o[$n / 2 - 1] + $o[$n / 2]) / 2 } }
$growth = foreach ($k in @('core', 'shell', 'tiling')) {
    $a = $base[$k]; $b = $final[$k]
    if (-not $a -or -not $b) { continue }
    $g = [pscustomobject]@{ Part = $k; Handles = $b.Handles - $a.Handles; Gdi = $b.Gdi - $a.Gdi; User = $b.User - $a.User; PrivateMB = [math]::Round($b.PrivateMB - $a.PrivateMB, 1) }
    if ($g.Handles -gt $MAX_HANDLE_GROWTH) { Fail "$k handles grew by $($g.Handles) (max $MAX_HANDLE_GROWTH)" }
    if ($g.Gdi -gt $MAX_GDI_GROWTH) { Fail "$k GDI objects grew by $($g.Gdi) (max $MAX_GDI_GROWTH)" }
    if ($g.User -gt $MAX_USER_GROWTH) { Fail "$k USER objects grew by $($g.User) (max $MAX_USER_GROWTH)" }
    if ($g.PrivateMB -gt $MAX_PRIVATE_MB_GROWTH) { Fail "$k private memory grew by $($g.PrivateMB) MB (max $MAX_PRIVATE_MB_GROWTH)" }
    # steady growth: median step between consecutive samples, scaled to a minute
    $steps = @(); $mbSteps = @()
    for ($i = 1; $i -lt $script:series.Count; $i++) {
        $p = $script:series[$i - 1][$k]; $q = $script:series[$i][$k]
        if ($p -and $q) { $steps += ($q.Handles - $p.Handles); $mbSteps += ($q.PrivateMB - $p.PrivateMB) }
    }
    $perMin = 60.0 / $SAMPLE_EVERY_SEC
    $g | Add-Member HandleRate ([math]::Round((Median $steps) * $perMin, 1))
    $g | Add-Member MBRate ([math]::Round((Median $mbSteps) * $perMin, 2))
    if ($steps.Count -ge 4 -and $g.HandleRate -gt $MAX_HANDLE_RATE) { Fail "$k handles grow steadily: $($g.HandleRate)/min (max $MAX_HANDLE_RATE)" }
    if ($mbSteps.Count -ge 4 -and $g.MBRate -gt $MAX_PRIVATE_MB_RATE) { Fail "$k private memory grows steadily: $($g.MBRate) MB/min (max $MAX_PRIVATE_MB_RATE)" }
    $g
}

# churn: growth per 100 operations, by two estimates that must both see it: the median step between consecutive
# samples (a one-time jump moves one step only) and the median slope between all pairs of samples (Theil-Sen; noise
# from what is in flight when a sample is taken cancels out). A leak grows every step and both see the same slope. A
# warm-up spread over two or three samples tips the median step (the window manager's private memory over the window
# churn: +6.8 and +4.5 MB in the first 20 operations, then 66-69 MB for 80 operations, read 9 MB per 100) but not the
# pairwise slope (2.4); a single jump from noise tips the slope but not the step.
function Rate($xs, $ys) {
    $r = @(); for ($i = 1; $i -lt $xs.Count; $i++) { if ($xs[$i] -gt $xs[$i - 1]) { $r += ($ys[$i] - $ys[$i - 1]) / ($xs[$i] - $xs[$i - 1]) } }
    if ($r.Count -lt 3) { return 0 }
    $p = @(); for ($i = 0; $i -lt $xs.Count; $i++) { for ($j = $i + 1; $j -lt $xs.Count; $j++) { if ($xs[$j] -gt $xs[$i]) { $p += ($ys[$j] - $ys[$i]) / ($xs[$j] - $xs[$i]) } } }
    [math]::Min((Median $r), (Median $p))
}
$churnLimits = [ordered]@{ Handles = $MAX_CHURN_HANDLES_PER_100; PrivateMB = $MAX_CHURN_MB_PER_100; Gdi = $MAX_CHURN_GDI_PER_100
    User = $MAX_CHURN_USER_PER_100; Threads = $MAX_CHURN_THREADS_PER_100; Windows = $MAX_CHURN_WINDOWS_PER_100 }
$churnRows = foreach ($c in $churn) {
    foreach ($k in @('core', 'shell', 'tiling')) {
        $pts = @($c.Series | Where-Object { $_.Sample[$k] })
        if ($pts.Count -lt 3) { continue }
        $xs = @($pts | ForEach-Object { [double]$_.Op })
        $row = [ordered]@{ Kind = $c.Kind; Part = $k }
        foreach ($m in $churnLimits.Keys) {
            $per100 = [math]::Round((Rate $xs @($pts | ForEach-Object { [double]$_.Sample[$k].$m })) * 100, 1)
            $row[$m] = $per100
            if ($per100 -le $churnLimits[$m]) { continue }
            if ($m -eq 'Threads' -and $churnBefore[$k] -and $churnSettled[$k]) {
                $kept = $churnSettled[$k].Threads - $churnBefore[$k].Threads
                if ($kept -le $MAX_CHURN_THREADS_SETTLED) { Note "churn $($c.Kind): $k threads rose $per100 per 100 operations but retired once idle ($kept kept)"; continue }
            }
            # Leaked memory is never given back: growing at the limit over this churn leaves at least limit x operations
            # after the quiet pause. Heaps and in-flight surfaces that rise during a churn and fall after it leave less
            # (the window manager over the window churn: 61 -> 68 MB, 51 MB at the start of the next churn; the shell over the popup
            # churn: 49.4 -> 52.6 MB, back to 50.1 by the end of the next churn).
            if ($m -eq 'PrivateMB' -and $churnSettled[$k]) {
                $kept = [math]::Round($churnSettled[$k].PrivateMB - $pts[0].Sample[$k].PrivateMB, 1)
                $atLimit = $churnLimits[$m] * $c.Ops / 100
                if ($kept -lt $atLimit) { Note "churn $($c.Kind): $k private memory rose $per100 MB per 100 operations but $kept MB of it was left after the quiet pause (growing at the limit leaves $atLimit)"; continue }
            }
            Fail "churn $($c.Kind): $k $m grows $per100 per 100 operations (max $($churnLimits[$m]))"
        }
        [pscustomobject]$row
    }
}
# slides must not slow down as switches pile up
$sw = $churn | Where-Object { $_.Kind -eq 'switch' } | Select-Object -First 1
$churnFrames = ''
if ($sw -and $interactive) {
    $third = [timespan]::FromTicks(($sw.To - $sw.From).Ticks / 3)
    $early = @(In-Phase $sw.From ($sw.From + $third) | Where-Object { $_.Text -match '^slide.*en uzun kare (\d+) ms' } | ForEach-Object { $null = $_.Text -match 'en uzun kare (\d+) ms'; [double]$Matches[1] })
    $late = @(In-Phase ($sw.To - $third) $sw.To | Where-Object { $_.Text -match '^slide.*en uzun kare (\d+) ms' } | ForEach-Object { $null = $_.Text -match 'en uzun kare (\d+) ms'; [double]$Matches[1] })
    $me = Median $early; $ml = Median $late
    $churnFrames = "Switch slides, median longest frame: first third $me ms ($($early.Count)), last third $ml ms ($($late.Count))"
    if ($early.Count -ge 5 -and $late.Count -ge 5 -and $ml - $me -gt $MAX_CHURN_FRAME_SLOWDOWN_MS) { Fail "slides got slower over $($sw.Ops) switches: median longest frame $me -> $ml ms (max +$MAX_CHURN_FRAME_SLOWDOWN_MS)" }
}
# DWM thumbnails the core holds while an animation plays ("önizleme=N" on slide/anim lines): a frozen layer that is
# never released stays registered and DWM keeps composing it, so the count must not climb across the switches
$thumbs = @(@(if ($sw) { In-Phase $sw.From $sw.To }) | Where-Object { $_.Text -match '^(slide|anim).*önizleme=(\d+)' } | ForEach-Object { $null = $_.Text -match 'önizleme=(\d+)'; [double]$Matches[1] })
if ($thumbs.Count -ge 10) {
    $n3 = [int]($thumbs.Count / 3)
    $te = Median $thumbs[0..($n3 - 1)]; $tl = Median $thumbs[($thumbs.Count - $n3)..($thumbs.Count - 1)]
    $churnFrames += "; DWM thumbnails during animations, median: first third $te, last third $tl"
    if ($tl - $te -gt $MAX_CHURN_THUMB_GROWTH) { Fail "DWM thumbnails held during animations grew over the churn: median $te -> $tl (max +$MAX_CHURN_THUMB_GROWTH)" }
}

# ------------------------------------------------------------------ report
$lines = @(
    "## Stress test",
    "",
    "| Check | Idle | Load | Limit |",
    "|---|---|---|---|",
    "| Longest frame (ms) | $frameIdle ($($slidesIdle.Count) slides) | $frameLoad ($($slidesLoad.Count) slides) | $MAX_FRAME_IDLE_MS / $MAX_FRAME_LOAD_MS |",
    "| Core's share of a frame (ms) | $oursIdle | $oursLoad | $MAX_OURS_IDLE_MS / $MAX_OURS_LOAD_MS |",
    "| DWM composing, longest wait (ms) | $dwmIdle | $dwmLoad |  |",
    "| Keyboard hook slow | $hookIdle | $hookLoad | $MAX_HOOK_SLOW_IDLE / $MAX_HOOK_SLOW_LOAD |",
    "| Super+F key to command (ms) | $stateIdle | $stateLoad | $MAX_STATE_IDLE_MS / $MAX_STATE_LOAD_MS |",
    "| Longest freeze (ms, steps) | $freezeIdle | $freezeLoad |  |",
    "| Bar heartbeat reports | $($beats.Count) |  |  |",
    "",
    "| Part | Handles | GDI | USER | Private MB | Handles/min (steady) | MB/min (steady) |",
    "|---|---|---|---|---|---|---|"
) + @($growth | ForEach-Object { "| $($_.Part) | $($_.Handles) | $($_.Gdi) | $($_.User) | $($_.PrivateMB) | $($_.HandleRate) | $($_.MBRate) |" }) + @(
    "",
    "Limits: growth $MAX_HANDLE_GROWTH handles / $MAX_PRIVATE_MB_GROWTH MB; steady $MAX_HANDLE_RATE handles/min, $MAX_PRIVATE_MB_RATE MB/min.",
    "",
    "### Churn: growth per 100 operations",
    "",
    "| Operation | Part | Handles | Private MB | GDI | USER | Threads | Top-level windows |",
    "|---|---|---|---|---|---|---|---|"
) + @($churnRows | ForEach-Object { "| $($_.Kind) | $($_.Part) | $($_.Handles) | $($_.PrivateMB) | $($_.Gdi) | $($_.User) | $($_.Threads) | $($_.Windows) |" }) + @(
    "",
    "Limits per 100: $MAX_CHURN_HANDLES_PER_100 handles, $MAX_CHURN_MB_PER_100 MB, $MAX_CHURN_GDI_PER_100 GDI, $MAX_CHURN_USER_PER_100 USER, $MAX_CHURN_THREADS_PER_100 threads, $MAX_CHURN_WINDOWS_PER_100 windows. $churnFrames",
    "",
    $stateChurn,
    "",
    "Interactive desktop: $([bool]$interactive)"
) + @($skipped | ForEach-Object { "- skipped: $_" }) + @($failures | ForEach-Object { "- **failed:** $_" })
$lines | ForEach-Object { Write-Host $_ }
if ($Report) { $lines | Add-Content -Path $Report -Encoding UTF8 }

# leave the desktop
try { & $core.FullName --shutdown } catch { }
$notepads | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }

if ($failures.Count) { Write-Host "$($failures.Count) check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host 'stress test passed' -ForegroundColor Green
