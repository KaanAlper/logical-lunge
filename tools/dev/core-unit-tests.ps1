# Core tests that never touch the running desktop (CoreUnitTests.cs), built with every core source file.
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$out = Join-Path $root 'build\tests'
New-Item -ItemType Directory -Force $out | Out-Null
$exe = Join-Path $out 'core-unit-tests.exe'
$sources = @(Get-ChildItem (Join-Path $root 'core') -Filter *.cs | ForEach-Object FullName) + (Join-Path $PSScriptRoot 'CoreUnitTests.cs')
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /target:exe /main:CoreUnitTests /optimize+ "/out:$exe" /r:System.Web.Extensions.dll /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Management.dll /r:Accessibility.dll /r:System.IO.Compression.dll /r:System.IO.Compression.FileSystem.dll $sources
if ($LASTEXITCODE) { throw 'Core unit test compilation failed' }
& $exe $root
if ($LASTEXITCODE) { throw 'Core unit tests failed' }
