$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$out = Join-Path $root 'build\tests'
New-Item -ItemType Directory -Force $out | Out-Null
$exe = Join-Path $out 'core-regression.exe'
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /target:exe /main:CoreRegression /optimize+ "/out:$exe" /r:System.Web.Extensions.dll /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Management.dll /r:Accessibility.dll /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll "$root\core\lunge.cs" "$root\core\WorkspaceConfigText.cs" "$root\core\MonitorFriendlyNames.cs" "$root\core\SettingsFile.cs" "$root\core\Brightness.cs" "$root\core\ToastPayload.cs" "$root\core\WinNotifications.cs" "$PSScriptRoot\CoreRegression.cs"
if ($LASTEXITCODE) { throw 'Core test compilation failed' }
& $exe
if ($LASTEXITCODE) { throw 'Core regression test failed' }

