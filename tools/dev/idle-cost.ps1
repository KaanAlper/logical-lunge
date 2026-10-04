# What the desktop costs while nobody uses it: CPU, wakeups (context switches) and memory of every Logical Lunge
# process, Explorer and DWM over a window. Run it after every install of a changed core, with the machine idle: an idle
# desktop must stay near zero (a region loop once took a whole core here, and only this showed it).
#   powershell -NoProfile -File tools\dev\idle-cost.ps1 [-Seconds 60]
param([int]$Seconds = 60)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class IdleCost {
  [StructLayout(LayoutKind.Sequential)] struct LASTINPUT { public uint size; public uint time; }
  [DllImport("user32.dll")] static extern bool GetLastInputInfo(ref LASTINPUT l);
  public static uint IdleSeconds() { var l = new LASTINPUT { size = 8 }; GetLastInputInfo(ref l); return ((uint)Environment.TickCount - l.time) / 1000; }
  [DllImport("ntdll.dll")] static extern int NtQueryTimerResolution(out uint min, out uint max, out uint cur);
  public static double TimerMs() { uint a, b, c; NtQueryTimerResolution(out a, out b, out c); return c / 10000.0; }
}
'@
$names = 'lunge', 'lunge-tiling', 'lunge-shell', 'lunge-wallpaper', 'lunge-temps', 'lunge-webview2', 'explorer', 'dwm'

# process id -> row name. WebView2 counts as Logical Lunge's only under a lunge process (the web edition's panels),
# not under other apps.
function Owners {
    $procs = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name)
    $own = @{}
    foreach ($p in $procs) { $n = $p.Name -replace '\.exe$', ''; if ($names -contains $n) { $own[[int]$p.ProcessId] = $n } }
    $lunge = @($own.Keys | Where-Object { $own[$_] -like 'lunge*' })
    do {
        $added = 0
        foreach ($p in $procs) {
            if ($p.Name -eq 'msedgewebview2.exe' -and -not $own.ContainsKey([int]$p.ProcessId) -and
                ($lunge -contains [int]$p.ParentProcessId -or $own[[int]$p.ParentProcessId] -eq 'lunge-webview2')) {
                $own[[int]$p.ProcessId] = 'lunge-webview2'; $added++
            }
        }
    } while ($added)
    $own
}

function Sample($own) {
    $s = @{}
    foreach ($n in $names) { $s[$n] = [pscustomobject]@{ Switches = [uint64]0; Cpu = 0.0; Private = 0; Working = 0; Threads = 0; Count = 0 } }
    # CPU time from the processor counters: DWM's own process times are not readable by a user
    foreach ($p in Get-CimInstance Win32_PerfRawData_PerfProc_Process) {
        $n = $own[[int]$p.IDProcess]
        if ($n) {
            $e = $s[$n]; $e.Cpu += [double]$p.PercentProcessorTime / 1e4   # 100 ns units -> ms
            $e.Private += $p.PrivateBytes; $e.Working += $p.WorkingSet; $e.Threads += $p.ThreadCount; $e.Count++
        }
    }
    foreach ($t in Get-CimInstance Win32_PerfRawData_PerfProc_Thread) {
        $n = $own[[int]$t.IDProcess]
        if ($n) { $s[$n].Switches += [uint64]$t.ContextSwitchesPersec }
    }
    $s
}

$idle = [IdleCost]::IdleSeconds()
$own = Owners
$a = Sample $own; $clock = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds $Seconds
$b = Sample $own; $el = $clock.Elapsed.TotalSeconds
'{0:HH:mm:ss}  {1:0} s window, no input for {2} s before, timer resolution {3:0.###} ms' -f (Get-Date), $el, $idle, [IdleCost]::TimerMs()
if ([IdleCost]::IdleSeconds() -lt $Seconds) { 'warning: someone used the machine during the window' }
'{0,-16} {1,8} {2,9} {3,9} {4,9} {5,8}' -f 'process', 'cpu %', 'wakes/s', 'priv MB', 'ws MB', 'threads'
$total = 0.0
foreach ($n in $names) {
    $x = $b[$n]; if ($x.Count -eq 0) { continue }
    $cpu = ($x.Cpu - $a[$n].Cpu) / ($el * 1000) * 100
    if ($n -like 'lunge*') { $total += $cpu }
    '{0,-16} {1,8:0.00} {2,9:0} {3,9:0.0} {4,9:0.0} {5,8}' -f $n, $cpu, (($x.Switches - $a[$n].Switches) / $el), ($x.Private / 1MB), ($x.Working / 1MB), $x.Threads
}
'Logical Lunge total: {0:0.00}% of one core' -f $total
