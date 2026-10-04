# Isolated uninstall regressions. Never dot-source or run uninstall.ps1 itself.
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$sandbox = Join-Path ([IO.Path]::GetTempPath()) ('ll-uninstall-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $sandbox | Out-Null
$script:passed = 0
function Assert($condition, [string]$message) { if (-not $condition) { throw $message }; $script:passed++ }
try {
    $helper = Join-Path $root 'scripts\uninstall-restore.ps1'
    if (Test-Path -LiteralPath $helper) { . $helper }
    else {
        # Exercise the old config cleanup in isolation to demonstrate the original data loss.
        $tokens = $null; $errors = $null
        $ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $root 'uninstall.ps1'), [ref]$tokens, [ref]$errors)
        $cleanup = $ast.Find({ param($n) $n -is [Management.Automation.Language.ForEachStatementAst] -and $n.Extent.Text.Contains('"$UserProfile\.wezterm.lua"') }, $true)
        function Restore-LLUserConfigs($Profile, $Delete) {
            $UserProfile = $Profile; $RemoveConfig = $Delete
            & ([scriptblock]::Create($cleanup.Extent.Text))
        }
    }
    $profile = Join-Path $sandbox 'user'
    New-Item -ItemType Directory -Path $profile | Out-Null
    $wez = Join-Path $profile '.wezterm.lua'
    [IO.File]::WriteAllText($wez, 'return { font_size = 17 }')
    Restore-LLUserConfigs $profile $true
    Assert (Test-Path -LiteralPath $wez) 'RemoveConfig deleted an unowned/live WezTerm config with no backup'
    Assert ([IO.File]::ReadAllText($wez) -eq 'return { font_size = 17 }') 'Unowned WezTerm config was changed'

    # A previous config must return without destroying the edits or consuming the original backup.
    [IO.File]::WriteAllText("$wez.before-ll", 'return { font_size = 12 }')
    Restore-LLUserConfigs $profile $false
    Assert ([IO.File]::ReadAllText($wez) -eq 'return { font_size = 12 }') 'Pre-install config was not restored'
    Assert ([IO.File]::ReadAllText("$wez.after-ll") -eq 'return { font_size = 17 }') 'Post-install user edits were lost'
    Assert (Test-Path -LiteralPath "$wez.before-ll") 'Original backup was consumed'
    Restore-LLUserConfigs $profile $true
    Assert (@(Get-ChildItem -LiteralPath $profile -Filter '*.after-ll*').Count -eq 1) 'Repeat uninstall created redundant backups'
    [IO.File]::WriteAllText($wez, 'return { font_size = 20 }')
    Restore-LLUserConfigs $profile $true
    Assert ([IO.File]::ReadAllText("$wez.after-ll.1") -eq 'return { font_size = 20 }') 'Existing saved edits were overwritten'
    foreach ($relative in '.config\fish\config.fish', '.config\starship.toml') {
        $file = Join-Path $profile $relative
        New-Item -ItemType Directory -Path (Split-Path $file) -Force | Out-Null
        [IO.File]::WriteAllText($file, 'user config')
        Restore-LLUserConfigs $profile $true
        Assert ([IO.File]::ReadAllText($file) -eq 'user config') "Unowned $relative was deleted"
        [IO.File]::WriteAllText("$file.before-ll", 'original config')
        Restore-LLUserConfigs $profile $true
        Assert ([IO.File]::ReadAllText($file) -eq 'original config') "Original $relative was not restored"
    }

    # Parse all deliverables, and compile the native helper only. Never call its shell-changing methods.
    foreach ($path in (Join-Path $root 'uninstall.ps1'), $helper, $PSCommandPath) {
        $tokens = $null; $errors = $null
        [void][Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
        Assert ($errors.Count -eq 0) "PowerShell syntax errors: $path"
    }
    Initialize-LLRestoreNative
    Assert ($null -ne ('LLUninstallShell' -as [type])) 'Native restoration interop did not compile'

    $boundary = Join-Path $sandbox 'deletion-root'
    $foreign = Join-Path $sandbox 'deletion-root-other'
    New-Item -ItemType Directory -Path $boundary, $foreign | Out-Null
    $safe = Join-Path $boundary 'LogicalLunge'
    New-Item -ItemType Directory -Path $safe | Out-Null
    [IO.File]::WriteAllText((Join-Path $safe 'owned'), 'owned')
    [IO.File]::WriteAllText((Join-Path $foreign 'unrelated'), 'unrelated')
    foreach ($invalid in $foreign, (Join-Path $boundary '..\deletion-root-other'), $boundary, 'relative\LogicalLunge') {
        $refused = $false
        try { Remove-LLTree $invalid $boundary } catch { $refused = $true }
        Assert $refused "Unsafe recursive deletion was allowed: $invalid"
        Assert (Test-Path -LiteralPath (Join-Path $foreign 'unrelated')) 'A failed boundary check still deleted unrelated files'
    }
    $junction = Join-Path $safe 'linked'
    New-Item -ItemType Junction -Path $junction -Target $foreign | Out-Null
    try {
        $refused = $false
        try { Remove-LLTree $safe $boundary } catch { $refused = $true }
        Assert $refused 'Tree containing a junction was recursively deleted'
        Assert (Test-Path -LiteralPath (Join-Path $safe 'owned')) 'Junction rejection happened after owned files were removed'
        $refused = $false
        try { Assert-LLSafePath (Join-Path $junction 'unrelated') $boundary } catch { $refused = $true }
        Assert $refused 'A junction in a target ancestor was accepted'
        Assert (Test-Path -LiteralPath (Join-Path $foreign 'unrelated')) 'Junction traversal deleted unrelated contents'
    }
    finally { [IO.Directory]::Delete($junction) } # Nonrecursive junction removal; never traverse the target.
    Remove-LLTree $safe $boundary
    Assert (-not (Test-Path -LiteralPath $safe)) 'Verified product tree was not removed'
    Assert (Test-Path -LiteralPath $foreign) 'Safe product deletion touched a sibling directory'

    $nativeRefresh = ${function:Invoke-LLShellRefresh}

    # Every system boundary below is a fake. File operations continue to use this test's temp directory.
    $script:registry = @{}
    $script:failRegistry = ''
    function Test-Path {
        [CmdletBinding()] param([string]$LiteralPath, [string]$Path)
        $p = if ($LiteralPath) { $LiteralPath } else { $Path }
        if ($p.StartsWith('Registry::')) { return $script:registry.ContainsKey($p) }
        return Microsoft.PowerShell.Management\Test-Path -LiteralPath $p
    }
    function New-Item {
        [CmdletBinding()] param([string]$Path, [string]$ItemType, [switch]$Force)
        if ($Path.StartsWith('Registry::')) { if (-not $script:registry.ContainsKey($Path)) { $script:registry[$Path] = @{} }; return }
        Microsoft.PowerShell.Management\New-Item -Path $Path -ItemType $ItemType -Force:$Force
    }
    function Get-ItemProperty {
        [CmdletBinding()] param([string]$LiteralPath)
        if (-not $LiteralPath.StartsWith('Registry::')) { throw 'Unexpected non-mock registry read' }
        if (-not $script:registry.ContainsKey($LiteralPath)) { throw 'Missing mock key' }
        return [pscustomobject]$script:registry[$LiteralPath]
    }
    function Set-ItemProperty {
        [CmdletBinding()] param([string]$LiteralPath, [string]$Name, $Value, [string]$Type)
        if (-not $LiteralPath.StartsWith('Registry::')) { throw 'Unexpected non-mock registry write' }
        if ($Name -eq $script:failRegistry) { throw 'Simulated registry access denied' }
        $script:registry[$LiteralPath][$Name] = $Value
    }
    function Remove-ItemProperty {
        [CmdletBinding()] param([string]$LiteralPath, [string]$Name)
        if (-not $LiteralPath.StartsWith('Registry::')) { throw 'Unexpected non-mock registry deletion' }
        $script:registry[$LiteralPath].Remove($Name)
    }
    $script:live = $null; $script:liveFail = $false
    function Invoke-LLShellRefresh($AutoHide, $HideIcons, $Arranging) {
        $script:live = @($AutoHide, $HideIcons, $Arranging)
        return -not $script:liveFail
    }
    $hku = 'Registry::HKEY_USERS\S-1-5-21-111-222-333-1001'
    $advanced = "$hku\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced"
    $taskbar = "$hku\Software\Microsoft\Windows\CurrentVersion\Explorer\StuckRects3"
    $state = Join-Path $sandbox 'state'
    New-Item -ItemType Directory -Path $state | Out-Null
    $takePath = Join-Path $state 'shell-takeover.json'
    $installPath = Join-Path $state 'install-backup.json'
    $bannerPath = Join-Path $state 'toast-banners.json'
    [IO.File]::WriteAllText($takePath, '{"v":1,"reg":[{"k":"Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced","n":"SnapAssist","had":true,"old":1},{"k":"Control Panel\\Desktop","n":"WindowArrangementActive","had":false,"old":null}],"autoHide":2}')
    [IO.File]::WriteAllText($installPath, ('{"registry":[{"path":"' + $advanced.Replace('\', '\\') + '","name":"HideIcons","existed":false,"old":null,"type":"DWord","binary":false}],"installed":[],"version":"0.2.1"}'))
    [IO.File]::WriteAllText($bannerPath, '{"app.original-off":0,"app.original-default":null,"app.user-enabled":null}')
    $script:registry[$advanced] = @{ HideIcons = 1; SnapAssist = 0 }
    $bytes = New-Object byte[] 40; $bytes[8] = 131; $bytes[22] = 77
    $script:registry[$taskbar] = @{ Settings = $bytes }
    $bannerRoot = "$hku\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings"
    $script:registry["$bannerRoot\app.original-off"] = @{ ShowBanner = 0 }
    $script:registry["$bannerRoot\app.original-default"] = @{ ShowBanner = 0 }
    $script:registry["$bannerRoot\app.user-enabled"] = @{ ShowBanner = 1 }
    $records = Read-LLRestoreSnapshot $state
    # Simulate the core consuming the record before the fallback runs.
    Remove-Item -LiteralPath $takePath
    Assert (Restore-LLWindowsState $records $hku $true) 'Valid snapshots failed to restore'
    Assert ($script:registry[$advanced].SnapAssist -eq 1) 'Saved SnapAssist was not restored'
    Assert (-not $script:registry[$advanced].ContainsKey('HideIcons')) 'Originally absent HideIcons was not removed'
    Assert ($script:live[0] -eq 2 -and $script:live[1] -eq $false -and $script:live[2] -eq $true) 'Live taskbar/icons/Snap restoration used incorrect originals'
    Assert ($bytes[8] -eq 130 -and $bytes[22] -eq 77) 'Persisted auto-hide flags or unrelated taskbar bytes were corrupted'
    Assert ($script:registry["$bannerRoot\app.original-off"].ShowBanner -eq 0) 'Originally disabled banners were enabled'
    Assert (-not $script:registry["$bannerRoot\app.original-default"].ContainsKey('ShowBanner')) 'Default-enabled banners were not restored'
    Assert ($script:registry["$bannerRoot\app.user-enabled"].ShowBanner -eq 1) 'User-enabled banners were overwritten'
    Assert (-not (Test-Path -LiteralPath $takePath)) 'Successful restoration retained a runtime record'
    Assert (Restore-LLWindowsState $records $hku $true) 'Repeat restoration failed'
    Assert ($bytes[8] -eq 130) 'Repeat restoration changed taskbar flags'

    # Live failure or registry failure must retain the snapshot even after core cleanup deleted it.
    $script:liveFail = $true
    Assert (-not (Restore-LLWindowsState $records $hku $true)) 'Live refresh failure was reported as success'
    Assert (Test-Path -LiteralPath $takePath) 'Live failure lost the consumed takeover record'
    Assert (Test-Path -LiteralPath $bannerPath) 'Live failure lost banner recovery'
    $script:liveFail = $false; $script:failRegistry = 'SnapAssist'
    Assert (-not (Restore-LLWindowsState $records $hku $true)) 'Registry failure was reported as success'
    Assert ((Read-LLRestoreSnapshot $state)['shell-takeover.json'].Value.autoHide -eq 2) 'Failed restore overwrote the original taskbar state'
    $script:failRegistry = ''; $script:live = $null
    Assert (-not (Restore-LLWindowsState $records $hku $false)) 'Cross-account restore incorrectly claimed a live refresh'
    Assert ($null -eq $script:live) 'Cross-account restoration touched the administrator shell'
    Assert (Test-Path -LiteralPath $takePath) 'Cross-account restoration discarded recovery originals'
    Assert ($bytes[8] -eq 130) 'Cross-account taskbar state was not persisted'
    $rejected = $false
    try { Resolve-LLRegistryPath 'Registry::HKEY_USERS\S-1-5-21-999\Software\Test' $hku } catch { $rejected = $true }
    Assert $rejected 'A backup for another account was accepted'
    Assert ((Resolve-LLRegistryPath 'HKCU:\Software\Test' $hku) -eq "$hku\Software\Test") 'HKCU legacy backup was not mapped to the target account'

    # Legacy installer StuckRects originals override a newer runtime snapshot.
    $originalBytes = New-Object byte[] 40; $originalBytes[8] = 3
    [IO.File]::WriteAllText($installPath, ('{"registry":[{"path":"' + $taskbar.Replace('\', '\\') + '","name":"Settings","existed":true,"old":"' + [Convert]::ToBase64String($originalBytes) + '","type":"Binary","binary":true}]}'))
    Assert (Restore-LLWindowsState (Read-LLRestoreSnapshot $state) $hku $true) 'Legacy taskbar original restore failed'
    Assert ($script:live[0] -eq 3) 'Installer-original auto-hide preference was overridden'
    Remove-Item -LiteralPath $installPath
    [IO.File]::WriteAllText($takePath, '{broken json')
    Assert (-not (Restore-LLWindowsState (Read-LLRestoreSnapshot $state) $hku $true)) 'Corrupt recovery JSON was reported as restored'
    Assert ([IO.File]::ReadAllText($takePath) -eq '{broken json') 'Corrupt recovery evidence was deleted or rewritten'
    Remove-Item -LiteralPath $takePath
    Assert (Restore-LLWindowsState (Read-LLRestoreSnapshot $state) $hku $true) 'Missing records made a repeat uninstall fail'
    Assert ($script:live[0] -eq -1 -and $null -eq $script:live[1]) 'Missing originals caused guessed Windows preferences'

    # Command timeout uses a fake Process; it can only terminate its own child.
    $app = Join-Path $sandbox 'app'; New-Item -ItemType Directory -Path $app | Out-Null
    $core = Join-Path $app 'lunge.exe'; [IO.File]::WriteAllText($core, 'not executable')
    $script:events = New-Object Collections.Generic.List[string]
    $script:waitTimes = New-Object Collections.Generic.List[int]
    $script:childExit = $false; $script:exitCode = 0
    function Start-Process {
        [CmdletBinding()] param($FilePath, $ArgumentList, $WindowStyle, [switch]$PassThru)
        if ($WindowStyle -ne 'Hidden') { throw 'Command could steal focus' }
        $child = [pscustomobject]@{ ExitCode = $script:exitCode }
        $child | Add-Member ScriptMethod WaitForExit { param($ms) $script:waitTimes.Add($ms); return $script:childExit }
        $child | Add-Member ScriptMethod Kill { $script:events.Add('child-kill') }
        $child | Add-Member ScriptMethod Dispose { $script:events.Add('child-dispose') }
        return $child
    }
    Assert (-not (Invoke-LLBoundedCommand $core @('--stop-desktop') 25)) 'Hung command did not time out'
    Assert ($script:waitTimes[0] -eq 25 -and $script:waitTimes[1] -eq 2000) 'Command wait was not bounded'
    Assert (($script:events -join ',') -eq 'child-kill,child-dispose') 'Timed-out child was not killed/disposed'
    $script:childExit = $true; $script:events.Clear(); $script:exitCode = 5
    Assert (-not (Invoke-LLBoundedCommand $core @('--takeover-restore') 25)) 'Failed command exit was ignored'
    Assert (($script:events -join ',') -eq 'child-dispose') 'An exited command was killed'
    $script:exitCode = 0
    Assert (Invoke-LLBoundedCommand $core @('--takeover-restore') 25) 'Successful restore command was rejected'

    # Process inventory is fake, but path/session filtering and shutdown sequencing are the actual production logic.
    $script:cim = @()
    function Get-CimInstance { [CmdletBinding()] param($ClassName); if ($ClassName -ne 'Win32_Process') { throw 'Unexpected CIM query' }; $script:cim }
    function Stop-Process {
        [CmdletBinding()] param([int]$Id, [switch]$Force)
        $script:events.Add("kill:$Id")
        $script:cim = @($script:cim | Where-Object { $_.ProcessId -ne $Id })
    }
    $script:gracefulExit = $false
    function Invoke-LLBoundedCommand($File, $Arguments, $TimeoutMs = 45000) {
        $script:events.Add($Arguments[0])
        if ($Arguments[0] -eq '--stop-desktop' -and $script:gracefulExit) { $script:cim = @($script:cim | Where-Object { $_.ProcessId -notin 10, 11 }) }
        return $script:gracefulExit
    }
    function Inventory {
        $script:cim = @(
            [pscustomobject]@{ Name = 'lunge-tiling.exe'; ExecutablePath = (Join-Path $app 'lunge-tiling.exe'); ProcessId = 11; SessionId = 7 },
            [pscustomobject]@{ Name = 'lunge.exe'; ExecutablePath = $core; ProcessId = 10; SessionId = 7 },
            [pscustomobject]@{ Name = 'lunge.exe'; ExecutablePath = (Join-Path $sandbox 'app-other\lunge.exe'); ProcessId = 99; SessionId = 7 },
            [pscustomobject]@{ Name = 'wezterm-gui.exe'; ExecutablePath = (Join-Path $app 'tools\wezterm\wezterm-gui.exe'); ProcessId = 98; SessionId = 7 })
    }
    Inventory; $script:events.Clear()
    Stop-LLDesktop $app $state $true 7
    Assert (($script:events -join ',') -eq '--stop-desktop,--restore-banners,--takeover-restore,kill:10,kill:11,--uncloak-orphans') 'Restore/graceful stop did not precede force kill, or orphan recovery was omitted'
    Assert (@($script:cim).Count -eq 2) 'Unrelated app or live WezTerm was terminated'
    Assert (Test-Path -LiteralPath (Join-Path $state 'maintenance')) 'Watchdogs were not suppressed before shutdown'
    Inventory; $script:gracefulExit = $true; $script:events.Clear()
    Stop-LLDesktop $app $state $true 7
    Assert (($script:events -join ',') -eq '--stop-desktop,--restore-banners,--takeover-restore,--uncloak-orphans') 'Successful graceful stop force-killed applications'
    Inventory; $script:gracefulExit = $false; $script:events.Clear()
    Stop-LLDesktop $app $state $true 7 $false
    Assert (-not ($script:events -match '^kill:')) 'Nonadmin caller attempted to kill privileged survivors'
    Assert (@(Get-LLOwnedProcesses $app).Count -eq 2) 'Nonadmin caller did not leave survivors for elevation'
    Inventory; $script:cim[0].SessionId = 8; $script:events.Clear(); $refused = $false
    try { Stop-LLDesktop $app $state $true 7 } catch { $refused = $true }
    Assert $refused 'Uninstall was allowed while another session was using this install'
    Assert ($script:events.Count -eq 0) 'Other-session detection happened after desktop changes'

    # Cross-account shutdown must not run any HKCU restore command as the credential administrator.
    function Get-NetTCPConnection { [CmdletBinding()] param($LocalPort, $State); return $null }
    Inventory; $script:events.Clear()
    Stop-LLDesktop $app $state $false 7
    Assert (($script:events -join ',') -eq 'kill:10,kill:11') 'Cross-account shutdown ran commands against the administrator profile'

    # Exercise both real caller statements without running the uninstaller's other system operations.
    $tokens = $null; $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $root 'uninstall.ps1'), [ref]$tokens, [ref]$errors)
    $calls = $ast.FindAll({ param($n) $n -is [Management.Automation.Language.CommandAst] -and $n.GetCommandName() -eq 'Stop-LLDesktop' }, $true)
    Assert ($calls.Count -eq 2) 'Both uninstall privilege paths must invoke the restoration shutdown'
    $APP = $app; $STATE = $state; $sameUser = $true; $sessionId = 7
    Inventory; $script:events.Clear()
    & ([scriptblock]::Create($calls[0].Extent.Text))
    Assert (-not ($script:events -match '^kill:')) 'Nonadmin uninstall caller did not defer forced shutdown to elevation'
    Inventory; $script:events.Clear()
    & ([scriptblock]::Create($calls[1].Extent.Text))
    Assert (($script:events -join ',') -eq '--stop-desktop,--restore-banners,--takeover-restore,kill:10,kill:11,--uncloak-orphans') 'Admin uninstall caller bypassed graceful restoration'

    # Inspect the emitted worker command via a fake launcher; never execute it against Explorer.
    function Invoke-LLBoundedCommand($File, $Arguments, $TimeoutMs) {
        $script:workerArguments = $Arguments; $script:workerTimeout = $TimeoutMs; return $true
    }
    Assert (& $nativeRefresh 2 $false $true) 'Shell refresh worker was not launched'
    $encoded = $script:workerArguments[-1]
    $worker = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($encoded))
    $workerAst = [Management.Automation.Language.Parser]::ParseInput($worker, [ref]$tokens, [ref]$errors)
    $load = $workerAst.Find({ param($n) $n -is [Management.Automation.Language.CommandAst] -and $n.InvocationOperator -eq 'Dot' }, $true)
    Assert ($load.CommandElements[0].Value -eq $helper) 'Worker did not load its defining restoration helper'
    Assert ($script:workerTimeout -eq 10000 -and $script:workerArguments -contains '-STA') 'Explorer worker did not have a timeout and STA COM context'

    Write-Host "PASS: $script:passed uninstall assertions"
}
finally {
    $resolved = [IO.Path]::GetFullPath($sandbox)
    if (-not $resolved.StartsWith([IO.Path]::GetTempPath(), [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe test cleanup path' }
    if (Get-Command Remove-LLTree -ErrorAction SilentlyContinue) { Remove-LLTree $resolved ([IO.Path]::GetTempPath()) }
    else { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
