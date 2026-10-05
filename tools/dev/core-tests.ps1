param([switch]$RestartOnly)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$out = Join-Path $root 'build\tests'
New-Item -ItemType Directory -Force $out | Out-Null
$exe = Join-Path $out 'core-regression.exe'
# every core source file, as build.ps1 (a hand-written list missed Uninstaller.cs and failed the 2026-10-05 release)
$sources = @(Get-ChildItem (Join-Path $root 'core') -Filter *.cs | Sort-Object Name | ForEach-Object FullName) + (Join-Path $PSScriptRoot 'CoreRegression.cs')
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /target:exe /main:CoreRegression /optimize+ "/out:$exe" /r:System.Web.Extensions.dll /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Management.dll /r:Accessibility.dll /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll $sources
if ($LASTEXITCODE) { throw 'Core test compilation failed' }
if ($RestartOnly) { & $exe --restart-only } else { & $exe }
if ($LASTEXITCODE) { throw 'Core regression test failed' }

