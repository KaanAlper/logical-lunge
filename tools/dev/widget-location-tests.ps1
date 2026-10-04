param([switch]$Live)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$outputDir = Join-Path $root 'build/tests'
New-Item -ItemType Directory -Force $outputDir | Out-Null
$testExe = Join-Path $outputDir 'widget-location-tests.exe'
$compileArgs = @('/nologo','/target:exe','/main:WidgetLocationTests',('/out:' + $testExe))
foreach ($reference in @('System.Web.Extensions.dll','System.Windows.Forms.dll','System.Drawing.dll','System.Management.dll','Accessibility.dll','System.IO.Compression.dll','System.IO.Compression.FileSystem.dll')) { $compileArgs += '/r:' + $reference }
$compileArgs += Get-ChildItem -LiteralPath (Join-Path $root 'core') -Filter '*.cs' | ForEach-Object { $_.FullName }
$compileArgs += Join-Path $PSScriptRoot 'WidgetLocationTests.cs'
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" @compileArgs
if ($LASTEXITCODE) { throw 'Widget location test compilation failed' }
if ($Live) { & $testExe --live } else { & $testExe }
if ($LASTEXITCODE) { throw 'Widget location tests failed' }
