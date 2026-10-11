# Logical Lunge - CI test of the uninstall while the desktop runs.
#
# Installs a built edition package the way the setup app does (installer\setup.ps1, without the terminal, the sensors and
# Everything), waits for its desktop (core, window manager, shell), then makes the window manager hang: it is suspended,
# so it answers nothing and the core's watchdog sees a hung window manager and restarts it, as on a user's PC on
# 2026-10-12, where the uninstall then failed at "Closing the core". The uninstall runs right then (uninstall.ps1 with
# -KeepConfig, as the uninstall window runs it, its steps in a work folder). It must report "done" (or "partial": only Explorer's live refresh unconfirmed), leave no process
# running from the install folder (nothing may come back afterwards either) and remove the folder.
#
#   powershell tools/ci/uninstall-test.ps1 -Package <folder with app\lunge.exe and installer\setup.ps1>
param(
    [Parameter(Mandatory = $true)][string]$Package,
    [string]$Report = $env:GITHUB_STEP_SUMMARY
)
$ErrorActionPreference = 'Stop'

$STARTUP_TIMEOUT_SEC = 180   # core + window manager + shell up after the install
$UNINSTALL_TIMEOUT_SEC = 300 # the whole uninstall (the stop waits at most ~2 min for a part that does not close)
$AFTER_SEC = 20              # nothing of ours may come back after the uninstall

$failures = New-Object Collections.Generic.List[string]
function Fail([string]$m) { $failures.Add($m); Write-Host "FAIL: $m" -ForegroundColor Red }
function Note([string]$m) { Write-Host $m }

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class LLHang {
    [DllImport("ntdll.dll")] static extern int NtSuspendProcess(IntPtr h);
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(int access, bool inherit, int pid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    // PROCESS_SUSPEND_RESUME
    public static int Suspend(int pid) {
        IntPtr h = OpenProcess(0x0800, false, pid);
        if (h == IntPtr.Zero) return Marshal.GetLastWin32Error();
        try { return NtSuspendProcess(h); } finally { CloseHandle(h); }
    }
}
'@

$app = Join-Path $env:ProgramFiles 'LogicalLunge'
$appPrefix = $app.TrimEnd('\') + '\'
function Ours {
    @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($appPrefix, [StringComparison]::OrdinalIgnoreCase) })
}
function Names($ps) { (@($ps) | ForEach-Object { "$($_.Name) ($($_.ProcessId))" }) -join ', ' }
# Why a process is still there: its command line, parent and what its threads wait on (a process ended while one of
# its threads is stuck in a call to Windows stays listed until that call returns)
function Explain($ps) {
    foreach ($p in @($ps)) {
        Note "  $($p.Name) ($($p.ProcessId)) parent $($p.ParentProcessId): $($p.CommandLine)"
        try {
            $proc = Get-Process -Id $p.ProcessId -ErrorAction Stop
            $states = $proc.Threads | Group-Object { "$($_.ThreadState)/$(if ($_.ThreadState -eq 'Wait') { $_.WaitReason } else { '-' })" } | ForEach-Object { "$($_.Name) x$($_.Count)" }
            Note "    threads: $($states -join ', ')"
            try { Stop-Process -Id $p.ProcessId -Force -ErrorAction Stop; Note '    ended now (TerminateProcess accepted)' } catch { Note "    end refused: $($_.Exception.Message)" }
        }
        catch { Note "    gone: $($_.Exception.Message)" }
    }
}

# ------------------------------------------------------------------ install
$coreFile = Get-ChildItem -Path $Package -Recurse -Filter lunge.exe | Where-Object { $_.Directory.Name -eq 'app' } | Select-Object -First 1
if (-not $coreFile) { throw "No app\lunge.exe under $Package" }
$source = Split-Path $coreFile.Directory.FullName
$setup = Join-Path $source 'installer\setup.ps1'
if (-not (Test-Path $setup)) { throw "No installer\setup.ps1 in $source" }
$me = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not ([Security.Principal.WindowsPrincipal]$me).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'The runner is not elevated: setup.ps1 cannot install' }
Note "installing $source"
$installWatch = [Diagnostics.Stopwatch]::StartNew()
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $setup -Source $source -UserProfile $env:USERPROFILE -UserSid $me.User.Value -UserName $me.Name -NoTerminal -NoSensors -NoEverything
$installCode = $LASTEXITCODE
Note "install exit $installCode after $([int]$installWatch.Elapsed.TotalSeconds) s"
if ($installCode) {
    $log = Join-Path $env:TEMP 'logical-lunge-install.log'
    if (Test-Path $log) { Get-Content $log -Tail 60 | ForEach-Object { Note "  | $_" } }
    throw "setup.ps1 failed ($installCode)"
}
if (-not (Test-Path (Join-Path $app 'uninstall.ps1'))) { throw "No uninstall.ps1 in $app after the install" }

# ------------------------------------------------------------------ desktop up
function Part([string]$name) {
    Get-CimInstance Win32_Process -Filter "Name = '$name'" | Where-Object { $_.ExecutablePath -and $_.ExecutablePath.StartsWith($appPrefix, [StringComparison]::OrdinalIgnoreCase) } |
        Sort-Object CreationDate | Select-Object -First 1
}
$deadline = (Get-Date).AddSeconds($STARTUP_TIMEOUT_SEC)
$started = $false
do {
    Start-Sleep 2
    $core = Part 'lunge.exe'; $tiling = Part 'lunge-tiling.exe'; $shell = Part 'lunge-shell.exe'
    # the sign-in task may not run on a runner (no interactive sign-in): started the way setup's fallback does
    if (-not $core -and -not $started -and (Get-Date) -gt $deadline.AddSeconds(-$STARTUP_TIMEOUT_SEC + 20)) {
        Note 'the sign-in task did not start the core; starting it directly'
        Start-Process -FilePath (Join-Path $app 'lunge.exe') -WorkingDirectory $env:USERPROFILE
        $started = $true
    }
} until (($core -and $tiling -and $shell) -or (Get-Date) -gt $deadline)
if (-not ($core -and $tiling -and $shell)) { throw "The desktop did not come up after the install: $(Names (Ours))" }
Note "desktop up: $(Names (Ours))"
Start-Sleep 15   # past the core's start-up (its watchdogs begin after 10-15 s)

# ------------------------------------------------------------------ hang the window manager, uninstall right then
$code = [LLHang]::Suspend([int]$tiling.ProcessId)
if ($code) { Fail "could not suspend the window manager ($code)" } else { Note "window manager suspended (pid $($tiling.ProcessId))" }
Start-Sleep 20   # the watchdog finds it unanswering (5 s polls, 8 s each)

$work = Join-Path $env:TEMP ('ll-uninstall-ci-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$steps = Join-Path $work 'steps.txt'
[IO.File]::WriteAllText($steps, '')
Note 'uninstalling while the desktop runs'
$uninstallWatch = [Diagnostics.Stopwatch]::StartNew()
$proc = Start-Process powershell.exe -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $app 'uninstall.ps1')`"",
    '-KeepConfig', '-Driver', "`"$work`"" -PassThru -WindowStyle Hidden
if (-not $proc.WaitForExit($UNINSTALL_TIMEOUT_SEC * 1000)) {
    Fail "the uninstall did not finish in $UNINSTALL_TIMEOUT_SEC s"
    try { $proc.Kill() } catch {}
}
$seconds = [int]$uninstallWatch.Elapsed.TotalSeconds
$text = [IO.File]::ReadAllText($steps)
Note "uninstall exit $($proc.ExitCode) after $seconds s; steps:"
$text -split "`r?`n" | Where-Object { $_ } | ForEach-Object { Note "  | $_" }
$result = ($text -split "`r?`n" | Where-Object { $_ -like 'result *' } | Select-Object -Last 1)
# "partial": everything was removed but Explorer's live refresh (desktop icons, taskbar) did not confirm; a runner's
# desktop has no folder view for it (seen 2026-10-12). This test is about stopping the desktop, so that is only noted.
if ($result -eq 'result partial') { Note "NOTE: the uninstall reported 'partial' (Explorer's live refresh did not confirm on this runner)" }
elseif ($result -ne 'result done') { Fail "the uninstall reported '$result', not 'result done'" }

# ------------------------------------------------------------------ nothing left, nothing comes back
$left = Ours
if ($left.Count) { Fail "processes still running from the install folder: $(Names $left)"; Explain $left }
Start-Sleep $AFTER_SEC
$back = Ours
if ($back.Count) { Fail "processes came back after the uninstall: $(Names $back)" }
if (Test-Path $app) { Fail "the install folder is still there: $(@(Get-ChildItem $app -Recurse -File | Select-Object -First 5 | ForEach-Object FullName) -join ', ')" }
if (Get-ScheduledTask -TaskPath '\LogicalLunge\' -ErrorAction SilentlyContinue) { Fail 'startup tasks are still registered' }
$ulog = Join-Path $env:LOCALAPPDATA 'LogicalLunge\logs\uninstall.log'
if (Test-Path $ulog) { Note "uninstall.log:"; Get-Content $ulog -Tail 40 | ForEach-Object { Note "  | $_" } }
else { Fail 'no logs\uninstall.log was written' }

if ($Report) {
    $lines = @('### Uninstall while the desktop runs (window manager hung)', '',
        "install: $([int]$installWatch.Elapsed.TotalSeconds) s; uninstall: $seconds s, $result", '')
    if ($failures.Count) { $lines += $failures | ForEach-Object { "- FAIL: $_" } } else { $lines += '- all checks passed' }
    $lines -join "`n" | Out-File -FilePath $Report -Append -Encoding utf8
}
if ($failures.Count) { Write-Host "$($failures.Count) check(s) failed" -ForegroundColor Red; exit 1 }
Write-Host 'PASS: uninstalled while the desktop ran' -ForegroundColor Green
