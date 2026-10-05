# How Logical Lunge holds up over days: reads the core's hourly memory lines ("bellek (saatlik)" in core.log and
# core.log.old) and reports, per process instance, private memory, handles and GDI/USER objects at its first and last
# hour, its peak, and the growth per day. A forked window manager was reported to slow everything down after about two
# days (glazewm #1225); a GDI or USER leak (zebar #264) ends at 10000 objects per process, when windows stop drawing.
#   powershell -NoProfile -File tools\dev\soak-report.ps1 [-Logs <folder>]
param([string]$Logs = (Join-Path $env:LOCALAPPDATA 'LogicalLunge\logs'))
$ErrorActionPreference = 'Stop'

# Warn: a process instance whose private memory grows faster than this per day after its first hour, or whose GDI/USER
# objects or handles pass these counts (Windows' per-process GDI and USER limits are 10000)
$growthMBPerDay = 50
$objectsLimit = 5000
$handlesLimit = 10000

$lines = foreach ($name in 'core.log.old', 'core.log') {
    $file = Join-Path $Logs $name
    if (Test-Path -LiteralPath $file) { Get-Content -LiteralPath $file -Encoding UTF8 | Where-Object { $_ -match 'bellek \(saatlik\)' } }
}
if (-not $lines) { throw "No hourly memory lines in $Logs" }

# A log line has only the time of day: days are counted when the time goes backwards
$samples = New-Object Collections.Generic.List[object]
$day = 0; $last = $null
foreach ($line in $lines) {
    if ($line -notmatch '^(\d\d):(\d\d):(\d\d)') { continue }
    $time = [TimeSpan]::new([int]$Matches[1], [int]$Matches[2], [int]$Matches[3])
    if ($last -and $time -lt $last) { $day++ }
    $last = $time
    $at = $day * 24 + $time.TotalHours
    foreach ($m in [regex]::Matches($line, '(\S+)#(\d+) \S+ (\d+) MB, handle (\d+)(?:, GDI (\d+), USER (\d+))?')) {
        $samples.Add([pscustomobject]@{
            Process = $m.Groups[1].Value; Id = [int]$m.Groups[2].Value; Hours = $at
            MB = [int]$m.Groups[3].Value; Handles = [int]$m.Groups[4].Value
            Gdi = if ($m.Groups[5].Success) { [int]$m.Groups[5].Value } else { $null }
            User = if ($m.Groups[6].Success) { [int]$m.Groups[6].Value } else { $null }
        })
    }
}

$warnings = 0
'{0,-22} {1,6} {2,9} {3,13} {4,9} {5,9} {6,15} {7,11}' -f 'process#id', 'hours', 'MB first', 'last (peak)', 'MB/day', 'handles', 'GDI / USER', 'status'
foreach ($group in $samples | Group-Object Process, Id | Sort-Object { $_.Group[0].Hours }) {
    $s = @($group.Group | Sort-Object Hours)
    $first = $s[0]; $lastS = $s[-1]
    $span = $lastS.Hours - $first.Hours
    $perDay = if ($span -ge 6) { ($lastS.MB - $first.MB) / $span * 24 } else { $null }
    $peak = ($s | Measure-Object MB -Maximum).Maximum
    $objects = if ($null -ne $lastS.Gdi) { '{0} / {1}' -f $lastS.Gdi, $lastS.User } else { '-' }
    $issues = @()
    if ($null -ne $perDay -and $perDay -gt $growthMBPerDay) { $issues += 'memory grows' }
    if ($lastS.Handles -gt $handlesLimit) { $issues += 'handles' }
    if (($lastS.Gdi -gt $objectsLimit) -or ($lastS.User -gt $objectsLimit)) { $issues += 'GDI/USER' }
    if ($issues) { $warnings++ }
    '{0,-22} {1,6:N1} {2,9} {3,13} {4,9} {5,9} {6,15} {7,11}' -f ($first.Process + '#' + $first.Id), $span, $first.MB,
        ('{0} ({1})' -f $lastS.MB, $peak), $(if ($null -ne $perDay) { '{0:N0}' -f $perDay } else { '-' }), $lastS.Handles, $objects,
        $(if ($issues) { $issues -join ', ' } else { 'ok' })
}
''
if ($warnings) { "$warnings process instance(s) need a look." } else { 'Nothing grows past the limits.' }
