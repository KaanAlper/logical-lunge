# Frame pacing as the screen shows it: DwmFlush per frame (the core) against a vblank-phased timer. Draws a strip across
# the primary monitor for about 15 s per 10 runs. It takes no focus, but run it only while nobody is using the screen.
#   powershell -NoProfile -File tools\dev\frame-bench\frame-bench.ps1 [-Runs 10] [-Load 0]
# -Load N keeps N normal-priority processes busy meanwhile (the slides must stay smooth under load).
param([int]$Runs = 10, [int]$Load = 0)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$out = Join-Path $root 'build\frame-bench'
New-Item -ItemType Directory -Force $out | Out-Null

$env:CARGO_TARGET_DIR = Join-Path $out 'target'
& cargo build --release --offline --quiet --manifest-path (Join-Path $PSScriptRoot 'capture\Cargo.toml')
if ($LASTEXITCODE) { throw 'frame-capture did not build' }
$capture = Join-Path $out 'target\release\frame-capture.exe'
$bench = Join-Path $out 'frame-bench.exe'
$sources = @(Get-ChildItem (Join-Path $root 'core') -Filter *.cs | ForEach-Object FullName) + (Join-Path $PSScriptRoot 'FrameBench.cs')
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /nowarn:108,169 /target:exe /main:FrameBench /optimize+ "/out:$bench" /r:System.Web.Extensions.dll /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Management.dll /r:Accessibility.dll /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll $sources
if ($LASTEXITCODE) { throw 'FrameBench did not build' }

$busy = @()
for ($i = 0; $i -lt $Load; $i++) {
    $busy += Start-Process powershell -ArgumentList '-NoProfile', '-Command', '$t=[DateTime]::Now.AddMinutes(2); while([DateTime]::Now -lt $t){}' -WindowStyle Hidden -PassThru
}
try {
    $seconds = [Math]::Ceiling($Runs * 2 * 1.3) + 3
    $rec = Start-Process $capture -ArgumentList '632', $seconds, (Join-Path $out 'capture.csv') -WindowStyle Hidden -PassThru
    $rec.PriorityClass = 'High'   # a starved recorder would report the bench's frames as missing
    Start-Sleep -Milliseconds 800
    & $bench (Join-Path $out 'bench.csv') $Runs
    $rec.WaitForExit()
} finally {
    foreach ($p in $busy) { try { $p.Kill() } catch {} }
}
$py = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python310\python.exe'
if (-not (Test-Path $py)) { $py = 'python' }
& $py (Join-Path $PSScriptRoot 'analyze.py') (Join-Path $out 'bench.csv') (Join-Path $out 'capture.csv')
