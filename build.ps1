# Logical Lunge - build a release package: dist\LogicalLunge-<version>.zip (+ .sha256)
# Build machine needs: Windows 10/11 x64 (.NET Framework 4.8 csc is built in), Windows 10 SDK (lunge-media.exe),
# Rust (rustup; tiling and shell), Python 3.12 (packaged tools) and Node.js (translations).
#
# Package layout (app\ is copied as is to %LOCALAPPDATA%\Programs\LogicalLunge):
#   app\lunge.exe, lunge-tiling.exe, lunge-tiling-cli.exe, lunge-tiling-watcher.exe, lunge-shell.exe, VERSION,
#       uninstall.ps1, ui\logical-lunge\*, scripts\*.ps1, tools\{lunge-media.exe, temps\, termcolors\, songrec\}
#   config\   templates for ~\.config\logical-lunge and the terminal
#   installer\setup.ps1
param([switch]$SkipPython, [switch]$SkipRust, [switch]$NoZip)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$root = $PSScriptRoot
$ver = (Get-Content (Join-Path $root 'VERSION')).Trim()
$name = "LogicalLunge-$ver"
$out = Join-Path $root "dist\$name"
$cache = Join-Path $root 'build'
New-Item -ItemType Directory -Force $cache | Out-Null
Remove-Item $out -Recurse -Force -ErrorAction SilentlyContinue
$app = Join-Path $out 'app'
$pack = Join-Path $app 'ui\logical-lunge'
New-Item -ItemType Directory -Force $pack, "$app\scripts", "$app\tools\temps", "$app\tools\songrec", "$app\tools\termcolors", "$out\installer" | Out-Null
function Step([string]$m) { Write-Host "==> $m" -ForegroundColor Cyan }
# Native tools (cargo, pip, PyInstaller, node) write progress to stderr: that is not an error in PowerShell 5.1 terms
function Native([scriptblock]$sb, [string]$what) { $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'; & $sb 2>&1 | ForEach-Object { if ($_ -is [System.Management.Automation.ErrorRecord]) { $_.ToString() } else { $_ } } | Where-Object { $_ -notmatch '^\d+ (INFO|WARNING|DEBUG):' } | Out-Host; $code = $LASTEXITCODE; $ErrorActionPreference = $eap; if ($code) { throw "$what failed ($code)" } }

$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$fx = "${env:ProgramFiles(x86)}\Reference Assemblies\Microsoft\Framework\.NETFramework\v4.8"
$icon = Join-Path $root 'resources\lunge.ico'

Step 'lunge.exe (core)'
# File description = the name Task Manager shows for the app (the other parts are grouped under it)
$nver = ($ver -replace '[^0-9.]', '') + '.0'
$info = Join-Path $cache 'AssemblyInfo.cs'
[IO.File]::WriteAllText($info, @"
using System.Reflection;
[assembly: AssemblyTitle("Logical Lunge")]
[assembly: AssemblyProduct("Logical Lunge")]
[assembly: AssemblyCompany("Logical Lunge")]
[assembly: AssemblyCopyright("GPL-3.0")]
[assembly: AssemblyVersion("$nver")]
[assembly: AssemblyFileVersion("$nver")]
[assembly: AssemblyInformationalVersion("$ver")]
"@)
& $csc /nologo /target:winexe /optimize+ "/out:$app\lunge.exe" "/win32icon:$icon" /r:System.Web.Extensions.dll /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:Accessibility.dll "$root\core\lunge.cs" $info
if ($LASTEXITCODE) { throw 'lunge.exe build failed' }

Step 'lunge-media.exe (album art + seek, WinRT)'
$winmd = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\UnionMetadata" -Recurse -Filter Windows.winmd -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\Facade\\' } | Sort-Object { [version]$_.Directory.Name } -Descending | Select-Object -First 1
if (-not $winmd) { throw 'Windows 10 SDK (UnionMetadata\Windows.winmd) not found' }
& $csc /nologo /target:winexe /optimize+ "/out:$app\tools\lunge-media.exe" "/r:$($winmd.FullName)" /r:System.Runtime.WindowsRuntime.dll "/r:$fx\Facades\System.Runtime.dll" /nowarn:1701 "$root\tools\lunge-media.cs"
if ($LASTEXITCODE) { throw 'lunge-media build failed' }

Step 'lunge-temps.exe (CPU/GPU temperature, LibreHardwareMonitorLib)'
$lz = Join-Path $cache 'LibreHardwareMonitor.zip'
if (-not (Test-Path $lz)) { Invoke-WebRequest -UseBasicParsing 'https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases/download/v0.9.6/LibreHardwareMonitor.zip' -OutFile $lz }
$lx = Join-Path $cache 'lhm'; if (-not (Test-Path "$lx\LibreHardwareMonitorLib.dll")) { Expand-Archive $lz $lx -Force }
& $csc /nologo /target:winexe /optimize+ "/out:$app\tools\temps\lunge-temps.exe" "/r:$lx\LibreHardwareMonitorLib.dll" "/r:$fx\Facades\netstandard.dll" /r:System.Core.dll "$root\tools\temps\lunge-temps.cs"
if ($LASTEXITCODE) { throw 'lunge-temps build failed' }

if (-not $SkipPython) {
    Step 'lunge-songrec.exe + lunge-termcolors.exe (PyInstaller, no Python needed on the target)'
    $venv = Join-Path $cache 'venv'
    if (-not (Test-Path "$venv\Scripts\python.exe")) { Native { py -3.12 -m venv $venv } 'venv' }
    Native { & "$venv\Scripts\python.exe" -m pip install --quiet --disable-pip-version-check -r "$root\tools\requirements.txt" } 'pip'
    $tc = Join-Path $root 'tools\termcolors'
    Native { & "$venv\Scripts\python.exe" -m PyInstaller --noconfirm --clean --onefile --noconsole --name lunge-termcolors --distpath "$cache\py-out" --workpath "$cache\pyi-tc" --specpath $cache `
        --add-data "$tc\generate_colors_material.py;." --add-data "$tc\scheme-base.json;." --collect-all materialyoucolor --hidden-import PIL.Image "$tc\wezterm-colors.py" } 'lunge-termcolors'
    Native { & "$venv\Scripts\python.exe" -m PyInstaller --noconfirm --clean --onefile --console --name lunge-songrec --distpath "$cache\py-out" --workpath "$cache\pyi-sr" --specpath $cache `
        --collect-all shazamio --collect-all shazamio_core --collect-all pyaudiowpatch "$root\tools\songrec\recognize.py" } 'lunge-songrec'
}
# (-SkipPython reuses the previous PyInstaller output)
foreach ($pair in @(@('lunge-termcolors.exe', 'termcolors'), @('lunge-songrec.exe', 'songrec'))) {
    $built = Join-Path "$cache\py-out" $pair[0]
    if (Test-Path $built) { Copy-Item $built (Join-Path "$app\tools" $pair[1]) }
    elseif (Test-Path (Join-Path "$app\tools\$($pair[1])" $pair[0])) { }
    else { Write-Host "    $($pair[0]) is missing (build without -SkipPython)" -ForegroundColor Yellow }
}

if (-not $SkipRust) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $env:Path = "$env:USERPROFILE\scoop\apps\rustup\current\.cargo\bin;$env:USERPROFILE\.cargo\bin;$env:Path" }
    $env:VERSION_NUMBER = ($ver -replace '[^0-9.]', '')
    Step 'lunge-tiling (window manager, watcher, CLI)'
    Push-Location "$root\tiling"
    try { Native { cargo build --release -p wm -p wm-cli -p wm-watcher } 'tiling' }
    finally { Pop-Location }
    Copy-Item "$root\tiling\target\release\lunge-tiling.exe", "$root\tiling\target\release\lunge-tiling-cli.exe", "$root\tiling\target\release\lunge-tiling-watcher.exe" $app
    Step 'lunge-shell (widget host)'
    # tauri-build merges this into tauri.conf.json: the exe carries the release version
    $env:TAURI_CONFIG = '{"version":"' + ($ver -replace '[^0-9.]', '') + '"}'
    Push-Location "$root\shell"
    try { Native { cargo build --release -p lunge-shell } 'shell' }
    finally { Pop-Location }
    Copy-Item "$root\shell\target\release\lunge-shell.exe" $app
}
else {
    Step 'tiling + shell: reusing the previous builds (-SkipRust)'
    foreach ($exe in 'tiling\target\release\lunge-tiling.exe', 'tiling\target\release\lunge-tiling-cli.exe', 'tiling\target\release\lunge-tiling-watcher.exe', 'shell\target\release\lunge-shell.exe') {
        if (Test-Path "$root\$exe") { Copy-Item "$root\$exe" $app }
    }
}
foreach ($exe in 'lunge-tiling.exe', 'lunge-tiling-cli.exe', 'lunge-tiling-watcher.exe', 'lunge-shell.exe') {
    if (-not (Test-Path (Join-Path $app $exe))) { throw "$exe is missing (build without -SkipRust)" }
}

Step 'UI (widgets bundled with their libraries and fonts; translations)'
# React, the Tauri API and the shell client are bundled in (ui\build.mjs): nothing is loaded from the network
Push-Location "$root\ui"
try {
    Native { npm ci --no-audit --no-fund } 'npm ci (ui)'
    Native { node build.mjs $pack } 'ui build'
}
finally { Pop-Location }

Step 'Scripts, configs, installer'
Copy-Item "$root\scripts\*.ps1" "$app\scripts\"
Copy-Item "$root\uninstall.ps1", "$root\VERSION" $app
Copy-Item "$root\config" "$out\config" -Recurse
Copy-Item "$root\installer\setup.ps1" "$out\installer\"
Copy-Item "$root\VERSION" $out

if ($NoZip) { Write-Host "Built $out (no zip)" -ForegroundColor Green; return }
Step 'Package'
$zip = Join-Path $root "dist\$name.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $out -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash
Set-Content -Encoding ASCII "$zip.sha256" "$hash  $name.zip"
Write-Host ''
Write-Host "Built $zip ($([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB)" -ForegroundColor Green
Write-Host "SHA256 $hash"
