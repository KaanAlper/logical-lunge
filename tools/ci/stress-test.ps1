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
$MAX_FRAME_IDLE_MS   = 150   # longest slide/animation frame while idle
$MAX_FRAME_LOAD_MS   = 500   # longest frame with every core busy
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
$MAX_CHURN_GDI_PER_100     = 10
$MAX_CHURN_USER_PER_100    = 10
$MAX_CHURN_THREADS_PER_100 = 4
$MAX_CHURN_WINDOWS_PER_100 = 2    # top-level windows a part owns
# slides must not get slower as operations pile up: median longest-frame of the last third vs the first third
$MAX_CHURN_FRAME_SLOWDOWN_MS = 40
$MAX_CHURN_THUMB_GROWTH = 8        # registered DWM thumbnails, last third vs first third of the churn

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
    public static void Nudge(int dx) { POINT p; if (GetCursorPos(out p)) SetCursorPos(p.X + dx, p.Y); }
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
function Sample {
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
try { Run-Phase $LOAD_SEC }
finally { $load | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue } }
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
$churn = @()
$churn += Churn 'switch' $CHURN_SWITCHES { param($i) Send-Test "ws-$((($i - 1) % 4) + 1)"; Start-Sleep -Milliseconds 850 }
Send-Test 'ws-1'; Start-Sleep 1
$churn += Churn 'window' $CHURN_WINDOW_CYCLES {
    $w = @(1..2 | ForEach-Object { Start-Process notepad -PassThru })
    Start-Sleep -Milliseconds 700
    $w | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
}
# overview and sidebar opened and closed in turn
$churn += Churn 'popup' 60 { param($i)
    $a = if ($i % 2) { 'overview' } else { 'sidebar' }
    Send-Test $a; Start-Sleep -Milliseconds 600; Send-Test $a; Start-Sleep -Milliseconds 500
}
# Super+F on the focused window: the freeze/snapshot animation (an even count leaves it as it was)
$churn += Churn 'state' 60 { Send-Test 'wm toggle-fullscreen'; Start-Sleep -Milliseconds 800 }
Start-Sleep 2

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

# churn: growth per 100 operations (median step between consecutive samples of each kind, per operation)
function Rate($xs, $ys) {
    $r = @(); for ($i = 1; $i -lt $xs.Count; $i++) { if ($xs[$i] -gt $xs[$i - 1]) { $r += ($ys[$i] - $ys[$i - 1]) / ($xs[$i] - $xs[$i - 1]) } }
    if ($r.Count -lt 3) { return 0 }
    Median $r
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
            if ($per100 -gt $churnLimits[$m]) { Fail "churn $($c.Kind): $k $m grows $per100 per 100 operations (max $($churnLimits[$m]))" }
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
    "| Keyboard hook slow | $hookIdle | $hookLoad | $MAX_HOOK_SLOW_IDLE / $MAX_HOOK_SLOW_LOAD |",
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
    "Interactive desktop: $([bool]$interactive)"
) + @($skipped | ForEach-Object { "- skipped: $_" }) + @($failures | ForEach-Object { "- **failed:** $_" })
$lines | ForEach-Object { Write-Host $_ }
if ($Report) { $lines | Add-Content -Path $Report -Encoding UTF8 }

# leave the desktop
try { & $core.FullName --shutdown } catch { }
$notepads | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }

if ($failures.Count) { Write-Host "$($failures.Count) check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host 'stress test passed' -ForegroundColor Green
