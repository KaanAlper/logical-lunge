# Logical Lunge - build a release package: dist\LogicalLunge-<version>.zip (+ .sha256)
# Build machine needs: Windows 10/11 x64 (.NET Framework 4.8 csc is built in), Windows 10 SDK
# (for media-art.exe), Python 3.12 (for the packaged helpers) and Node.js (translations).
param([switch]$SkipPython, [string]$Forks = (Join-Path $PSScriptRoot '..\logical-lunge-forks'))
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
$ll = Join-Path $out 'logical-lunge'
New-Item -ItemType Directory -Force "$ll\helper", "$ll\scripts", "$ll\tools\lhm", "$ll\tools\songrec", "$ll\tools\termcolors", "$out\shell", "$out\installer" | Out-Null
function Step([string]$m) { Write-Host "==> $m" -ForegroundColor Cyan }
# Native tools (pip, PyInstaller, node) write progress to stderr: that is not an error in PowerShell 5.1 terms
function Native([scriptblock]$sb, [string]$what) { $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'; & $sb 2>&1 | ForEach-Object { if ($_ -is [System.Management.Automation.ErrorRecord]) { $_.ToString() } else { $_ } } | Where-Object { $_ -notmatch '^\d+ (INFO|WARNING|DEBUG):' } | Out-Host; $code = $LASTEXITCODE; $ErrorActionPreference = $eap; if ($code) { throw "$what failed ($code)" } }

$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$fx = "${env:ProgramFiles(x86)}\Reference Assemblies\Microsoft\Framework\.NETFramework\v4.8"

Step 'll-helper.exe'
& $csc /nologo /target:winexe /optimize+ "/out:$ll\helper\ll-helper.exe" /r:System.Web.Extensions.dll /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:Accessibility.dll "$root\src\helper\ll-helper.cs"
if ($LASTEXITCODE) { throw 'll-helper build failed' }

Step 'media-art.exe (album art + seek, WinRT)'
$winmd = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\UnionMetadata" -Recurse -Filter Windows.winmd -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\Facade\\' } | Sort-Object { [version]$_.Directory.Name } -Descending | Select-Object -First 1
if (-not $winmd) { throw 'Windows 10 SDK (UnionMetadata\Windows.winmd) not found' }
& $csc /nologo /target:winexe /optimize+ "/out:$ll\helper\media-art.exe" "/r:$($winmd.FullName)" /r:System.Runtime.WindowsRuntime.dll "/r:$fx\Facades\System.Runtime.dll" /nowarn:1701 "$root\src\helper\media-art.cs"
if ($LASTEXITCODE) { throw 'media-art build failed' }

Step 'll-temps.exe (CPU/GPU temperature, LibreHardwareMonitorLib)'
$lz = Join-Path $cache 'LibreHardwareMonitor.zip'
if (-not (Test-Path $lz)) { Invoke-WebRequest -UseBasicParsing 'https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases/download/v0.9.6/LibreHardwareMonitor.zip' -OutFile $lz }
$lx = Join-Path $cache 'lhm'; if (-not (Test-Path "$lx\LibreHardwareMonitorLib.dll")) { Expand-Archive $lz $lx -Force }
& $csc /nologo /target:winexe /optimize+ "/out:$ll\tools\lhm\ll-temps.exe" "/r:$lx\LibreHardwareMonitorLib.dll" "/r:$fx\Facades\netstandard.dll" /r:System.Core.dll "$root\src\tools\temps\ll-temps.cs"
if ($LASTEXITCODE) { throw 'll-temps build failed' }

if (-not $SkipPython) {
    Step 'll-songrec.exe + ll-termcolors.exe (PyInstaller, no Python needed on the target)'
    $venv = Join-Path $cache 'venv'
    if (-not (Test-Path "$venv\Scripts\python.exe")) { Native { py -3.12 -m venv $venv } 'venv' }
    Native { & "$venv\Scripts\python.exe" -m pip install --quiet --disable-pip-version-check -r "$root\src\tools\requirements.txt" } 'pip'
    $tc = Join-Path $root 'src\tools\termcolors'
    Native { & "$venv\Scripts\python.exe" -m PyInstaller --noconfirm --clean --onefile --noconsole --name ll-termcolors --distpath "$ll\tools\termcolors" --workpath "$cache\pyi-tc" --specpath $cache `
        --add-data "$tc\generate_colors_material.py;." --add-data "$tc\scheme-base.json;." --collect-all materialyoucolor --hidden-import PIL.Image "$tc\wezterm-colors.py" } 'll-termcolors'
    Native { & "$venv\Scripts\python.exe" -m PyInstaller --noconfirm --clean --onefile --console --name ll-songrec --distpath "$ll\tools\songrec" --workpath "$cache\pyi-sr" --specpath $cache `
        --collect-all shazamio --collect-all shazamio_core --collect-all pyaudiowpatch "$root\src\tools\songrec\recognize.py" } 'll-songrec'
}

# Our GlazeWM / Zebar forks (tray icons, unused providers removed). When they are not checked out next to
# the repo the package falls back to the pinned upstream releases, which setup.ps1 downloads.
if (Test-Path "$Forks\glazewm\Cargo.toml") {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $env:Path = "$env:USERPROFILE\scoop\apps\rustup\current\.cargo\bin;$env:USERPROFILE\.cargo\bin;$env:Path" }
    Step 'GlazeWM fork (glazewm, glazewm-watcher, glazewm-cli)'
    New-Item -ItemType Directory -Force "$ll\bin" | Out-Null
    Push-Location "$Forks\glazewm"
    $env:VERSION_NUMBER = ((Get-Content "$root\VERSION").Trim() -replace '[^0-9.]', '')
    Native { cargo build --release -p wm -p wm-cli -p wm-watcher } 'glazewm'
    Copy-Item target\release\glazewm.exe, target\release\glazewm-watcher.exe, target\release\glazewm-cli.exe "$ll\bin\"
    Pop-Location
}
if (Test-Path "$Forks\zebar\Cargo.toml") {
    Step 'Zebar fork'
    Push-Location "$Forks\zebar"
    # Our Zebar has no settings UI / client package to build: only the Rust app (widgets are the shell's pack)
    Native { cargo build --release -p zebar } 'zebar'
    Copy-Item target\release\zebar.exe "$ll\bin\"
    Pop-Location
}

Step 'Shell (Zebar widget pack) + translations'
Push-Location "$root\src\shell"; Native { node i18n.src.js } 'i18n'; Pop-Location
$snippet = [IO.File]::ReadAllText("$root\src\shell\i18n.snippet.js").Replace("`r`n", "`n")
Get-ChildItem "$root\src\shell" -File | Where-Object { $_.Name -notin 'i18n.src.js', 'i18n.snippet.js' } | ForEach-Object {
    $dst = Join-Path "$out\shell" $_.Name
    if ($_.Extension -eq '.html') {
        # every widget carries the same inline translation layer
        $h = [IO.File]::ReadAllText($_.FullName).Replace("`r`n", "`n")
        $h = [regex]::Replace($h, '(?s)    <script>\n// Logical Lunge dil katmanı.*?    </script>', ("    <script>`n" + $snippet + '    </script>').Replace('$', '$$'))
        [IO.File]::WriteAllText($dst, $h, (New-Object Text.UTF8Encoding $false))
    }
    else { Copy-Item $_.FullName $dst }
}

Step 'Scripts, configs, installer'
Copy-Item "$root\src\scripts\*.ps1" "$ll\scripts\"
Copy-Item "$root\src\config" "$out\config" -Recurse
Copy-Item "$root\installer\setup.ps1" "$out\installer\"
Copy-Item "$root\uninstall.ps1", "$root\VERSION" $out

Step 'Package'
$zip = Join-Path $root "dist\$name.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $out -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash
Set-Content -Encoding ASCII "$zip.sha256" "$hash  $name.zip"
Write-Host ''
Write-Host "Built $zip ($([math]::Round((Get-Item $zip).Length / 1MB, 1)) MB)" -ForegroundColor Green
Write-Host "SHA256 $hash"
