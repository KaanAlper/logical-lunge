# Shared uninstall helpers. Loading this file defines functions only; no desktop or registry changes.
function Assert-LLSafePath([string]$Target, [string]$Root, [switch]$Recursive) {
    if (-not [IO.Path]::IsPathRooted($Target) -or -not [IO.Path]::IsPathRooted($Root)) { throw 'Uninstall paths must be absolute.' }
    $absolute = [IO.Path]::GetFullPath($Target).TrimEnd('\')
    $boundary = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    if (-not $absolute.StartsWith($boundary, [StringComparison]::OrdinalIgnoreCase)) { throw "Uninstall target escapes its intended directory: $Target" }
    # GetFullPath normalizes dot segments, but does not resolve junctions. Reject links in every ancestor.
    $ancestor = $absolute
    while ($ancestor) {
        $item = Get-Item -LiteralPath $ancestor -Force -ErrorAction SilentlyContinue
        if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Uninstall refuses a linked path: $ancestor" }
        $parent = [IO.Path]::GetDirectoryName($ancestor)
        if ($parent -eq $ancestor) { break }
        $ancestor = $parent
    }
    if ($Recursive -and (Test-Path -LiteralPath $absolute)) {
        # Do not recurse through a junction while looking for junctions. Scan one physical level at a time.
        $pending = New-Object Collections.Generic.Stack[string]
        $pending.Push($absolute)
        while ($pending.Count) {
            foreach ($entry in Get-ChildItem -LiteralPath $pending.Pop() -Force -ErrorAction Stop) {
                if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Uninstall refuses a tree containing a link: $($entry.FullName)" }
                if ($entry.PSIsContainer) { $pending.Push($entry.FullName) }
            }
        }
    }
    return $absolute
}

function Remove-LLTree([string]$Target, [string]$Root) {
    $absolute = Assert-LLSafePath $Target $Root -Recursive
    Remove-Item -LiteralPath $absolute -Recurse -Force -ErrorAction Stop
}

function Invoke-LLBoundedCommand([string]$File, [string[]]$Arguments, [int]$TimeoutMs = 45000) {
    if (-not (Test-Path -LiteralPath $File)) { return $false }
    $process = $null
    try {
        $process = Start-Process -FilePath $File -ArgumentList $Arguments -WindowStyle Hidden -PassThru -ErrorAction Stop
        if (-not $process.WaitForExit($TimeoutMs)) {
            # Only this command's child, never an arbitrary process with the same name.
            $process.Kill()
            [void]$process.WaitForExit(2000)
            Write-Warning "Timed out: $File $Arguments"
            return $false
        }
        return $process.ExitCode -eq 0
    }
    catch { Write-Warning "Could not run $File $Arguments : $($_.Exception.Message)"; return $false }
    finally { if ($process) { $process.Dispose() } }
}

function Get-LLOwnedProcesses([string]$App) {
    $prefix = [IO.Path]::GetFullPath($App).TrimEnd('\') + '\'
    $names = @('lunge.exe', 'lunge-tiling.exe', 'lunge-tiling-watcher.exe', 'lunge-shell.exe', 'lunge-wallpaper.exe',
        'LogicalLunge.exe', 'LogicalLunge.scr', 'lunge-temps.exe', 'lunge-songrec.exe', 'lunge-termcolors.exe')
    Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
        $_.Name -in $names -and $_.ExecutablePath -and
        ([IO.Path]::GetFullPath($_.ExecutablePath)).StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
    }
}

function Stop-LLDesktop([string]$App, [string]$State, [bool]$SameUser, [int]$SessionId, [bool]$AllowForce = $true) {
    $owned = @(Get-LLOwnedProcesses $App)
    if (@($owned | Where-Object { $_.SessionId -ne $SessionId }).Count) {
        throw 'Logical Lunge is in use in another session. Sign that session out before uninstalling.'
    }
    New-Item -ItemType Directory -Path $State -Force -ErrorAction Stop | Out-Null
    [IO.File]::WriteAllText((Join-Path $State 'maintenance'), [DateTime]::UtcNow.ToString('o'))
    $core = Join-Path $App 'lunge.exe'
    if ($SameUser) { [void](Invoke-LLBoundedCommand $core @('--stop-desktop')) }
    elseif (@($owned | Where-Object { $_.Name -eq 'lunge.exe' }).Count) {
        # A credential elevation must ask the original core to do its own HKCU cleanup.
        # Verify the loopback listener belongs to this install/session before sending anything.
        $ids = @($owned | Where-Object { $_.Name -eq 'lunge.exe' } | ForEach-Object { $_.ProcessId })
        $listener = Get-NetTCPConnection -LocalPort 6131 -State Listen -ErrorAction SilentlyContinue |
            Where-Object { $_.OwningProcess -in $ids }
        if ($listener) {
            try {
                $request = [Net.HttpWebRequest]::Create('http://127.0.0.1:6131/cmd?a=stop-desktop')
                $request.Method = 'POST'; $request.ContentLength = 0; $request.Proxy = $null
                $request.Timeout = 2000; $request.ReadWriteTimeout = 2000
                $response = $request.GetResponse(); $response.Dispose()
                $watch = [Diagnostics.Stopwatch]::StartNew()
                while (@(Get-LLOwnedProcesses $App).Count -and $watch.ElapsedMilliseconds -lt 45000) { Start-Sleep -Milliseconds 100 }
            }
            catch { Write-Warning "Graceful stop request failed: $($_.Exception.Message)" }
        }
    }
    if ($SameUser) {
        [void](Invoke-LLBoundedCommand $core @('--restore-banners') 10000)
        [void](Invoke-LLBoundedCommand $core @('--takeover-restore') 10000)
    }
    # Graceful stop and restoration have had their chance. Kill only surviving installed parts, core first.
    $survivors = @(Get-LLOwnedProcesses $App | Sort-Object @{ Expression = { if ($_.Name -eq 'lunge.exe') { 0 } else { 1 } } })
    # Medium integrity cannot terminate the elevated WM/core if graceful shutdown failed. The elevated copy retries.
    if (-not $AllowForce -and $survivors.Count) { return }
    foreach ($part in $survivors) {
        if ($part.SessionId -ne $SessionId) { throw 'Another session started Logical Lunge during uninstall.' }
        Stop-Process -Id $part.ProcessId -Force -ErrorAction Stop
    }
    if (@(Get-LLOwnedProcesses $App).Count) { throw 'Some Logical Lunge processes could not be stopped; files and recovery records are kept.' }
    # Also repair cloaked workspaces if the graceful command timed out or the WM had already crashed.
    if ($SameUser) { [void](Invoke-LLBoundedCommand $core @('--uncloak-orphans') 15000) }
}

function Read-LLRestoreSnapshot([string]$State) {
    $snapshot = @{}
    foreach ($name in 'install-backup.json', 'shell-takeover.json', 'toast-banners.json') {
        $path = Join-Path $State $name
        if (-not (Test-Path -LiteralPath $path)) { continue }
        try {
            [void](Assert-LLSafePath $path $State)
            $raw = [IO.File]::ReadAllText($path)
            $value = $raw | ConvertFrom-Json -ErrorAction Stop
            if ($null -eq $value) { throw 'Empty recovery record' }
            $snapshot[$name] = @{ Path = $path; Raw = $raw; Value = $value; Valid = $true }
        }
        catch { $snapshot[$name] = @{ Path = $path; Valid = $false }; Write-Warning "Unreadable recovery record: $path" }
    }
    return $snapshot
}

function Resolve-LLRegistryPath([string]$Path, [string]$Hku) {
    if ($Path.StartsWith($Hku + '\', [StringComparison]::OrdinalIgnoreCase)) { return $Path }
    if ($Path -match '^(HKCU:|HKEY_CURRENT_USER|Registry::HKEY_CURRENT_USER)\\(.+)$') { return "$Hku\$($Matches[2])" }
    throw "Recovery record is not for the target user: $Path"
}

function Set-LLRestoredValue([string]$Path, [string]$Name, $Value, [string]$Type, [bool]$Existed) {
    if ($Existed) {
        if (-not (Test-Path -LiteralPath $Path)) { New-Item -Path $Path -Force -ErrorAction Stop | Out-Null }
        Set-ItemProperty -LiteralPath $Path -Name $Name -Value $Value -Type $Type -ErrorAction Stop
    }
    elseif (Test-Path -LiteralPath $Path) {
        $current = Get-ItemProperty -LiteralPath $Path -ErrorAction Stop
        if ($current.PSObject.Properties[$Name]) { Remove-ItemProperty -LiteralPath $Path -Name $Name -ErrorAction Stop }
    }
}

function Restore-LLWindowsState($Snapshot, [string]$Hku, [bool]$LiveUser) {
    $ok = $true; $autoHide = -1; $hideIcons = $null; $arranging = $null
    $take = $Snapshot['shell-takeover.json']; $install = $Snapshot['install-backup.json']; $banners = $Snapshot['toast-banners.json']
    foreach ($record in @($take, $install, $banners)) { if ($record -and -not $record.Valid) { $ok = $false } }
    if ($take -and $take.Valid) {
        try {
            if ($null -eq $take.Value.reg) { throw 'Missing takeover registry entries' }
            if ($null -ne $take.Value.autoHide) {
                if ($take.Value.autoHide -notin -1, 0, 1, 2, 3) { throw 'Invalid taskbar state' }
                $autoHide = [int]$take.Value.autoHide
            }
            foreach ($e in @($take.Value.reg)) {
                if ($e.had -isnot [bool] -or -not $e.k -or -not $e.n -or $e.k -match '(^\\|\.\.)') { throw 'Invalid takeover registry entry' }
                $path = Resolve-LLRegistryPath "$Hku\$($e.k)" $Hku
                if ($e.had -and $null -eq $e.old) { throw 'Missing original registry value' }
                $type = if ($e.old -is [string]) { 'String' } else { 'DWord' }
                Set-LLRestoredValue $path $e.n $e.old $type $e.had
                if ($e.k -eq 'Control Panel\Desktop' -and $e.n -eq 'WindowArrangementActive') { $arranging = (-not $e.had -or [string]$e.old -ne '0') }
            }
        }
        catch { $ok = $false; Write-Warning "Shell restoration failed: $($_.Exception.Message)" }
    }
    # Installer originals take precedence over a snapshot captured by a later runtime.
    if ($install -and $install.Valid) {
        foreach ($r in @($install.Value.registry)) {
            try {
                if (-not $r.path -or -not $r.name -or $r.existed -isnot [bool]) { throw 'Invalid installer registry entry' }
                $path = Resolve-LLRegistryPath $r.path $Hku
                $v = $r.old
                if ($r.binary -and $r.existed) { $v = [Convert]::FromBase64String([string]$r.old) }
                Set-LLRestoredValue $path $r.name $v $r.type $r.existed
                if ($path -eq "$Hku\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced" -and $r.name -eq 'HideIcons') { $hideIcons = ($r.existed -and [int]$v -ne 0) }
                if ($path -eq "$Hku\Control Panel\Desktop" -and $r.name -eq 'WindowArrangementActive') { $arranging = (-not $r.existed -or [string]$v -ne '0') }
                if ($path -eq "$Hku\Software\Microsoft\Windows\CurrentVersion\Explorer\StuckRects3" -and $r.name -eq 'Settings' -and $r.existed -and $v -is [byte[]] -and $v.Length -ge 9) { $autoHide = $v[8] -band 3 }
            }
            catch { $ok = $false; Write-Warning "Registry restoration failed: $($_.Exception.Message)" }
        }
    }
    if ($banners -and $banners.Valid) {
        foreach ($entry in $banners.Value.PSObject.Properties) {
            try {
                if (-not $entry.Name -or $entry.Name.Length -gt 255 -or $entry.Name -match '[\\\x00-\x1f]') { throw 'Invalid notification identity' }
                $path = "$Hku\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\$($entry.Name)"
                if (Test-Path -LiteralPath $path) {
                    $now = (Get-ItemProperty -LiteralPath $path -ErrorAction Stop).ShowBanner
                    if ($null -ne $now -and $now -eq 0) { Set-LLRestoredValue $path 'ShowBanner' $entry.Value 'DWord' ($null -ne $entry.Value) }
                }
            }
            catch { $ok = $false; Write-Warning "Banner restoration failed: $($_.Exception.Message)" }
        }
    }
    if ($autoHide -ge 0) {
        # Preserve the full saved appbar flags, including ALWAYSONTOP. Also persist for credential elevation.
        $path = "$Hku\Software\Microsoft\Windows\CurrentVersion\Explorer\StuckRects3"
        try {
            $settings = (Get-ItemProperty -LiteralPath $path -ErrorAction Stop).Settings
            if ($settings -is [byte[]] -and $settings.Length -ge 9) {
                $settings[8] = ($settings[8] -band 252) -bor $autoHide
                Set-LLRestoredValue $path 'Settings' $settings 'Binary' $true
            }
            elseif (-not $LiveUser) { throw 'No taskbar settings to persist for the target account' }
        }
        catch { if (-not $LiveUser) { $ok = $false }; Write-Warning "Taskbar persistence: $($_.Exception.Message)" }
    }
    if ($LiveUser) {
        if (-not (Invoke-LLShellRefresh $autoHide $hideIcons $arranging)) { $ok = $false }
    }
    else {
        if ($Snapshot.Count) { $ok = $false }
        Write-Warning 'Registry restored for the target account; live shell refresh requires that user session. Recovery records are retained.'
    }
    # The core may have consumed these files during graceful stop. Retain snapshots until ALL restoration succeeds.
    foreach ($record in @($take, $banners)) {
        if (-not $record -or -not $record.Valid) { continue }
        [void](Assert-LLSafePath $record.Path (Split-Path $record.Path))
        if ($ok) { Remove-Item -LiteralPath $record.Path -Force -ErrorAction SilentlyContinue }
        else { [IO.File]::WriteAllText($record.Path, $record.Raw, (New-Object Text.UTF8Encoding $false)) }
    }
    return $ok
}

function Restore-LLUserConfigs([string]$Profile, [bool]$Delete) {
    # No ownership manifest was written by older installers. Lack of a backup is NOT proof we own a config.
    # Keep live/shared terminal configs even for RemoveConfig (WezTerm watches the file while it runs).
    foreach ($relative in '.wezterm.lua', '.config\fish\config.fish', '.config\starship.toml') {
        $file = Join-Path $Profile $relative; $original = "$file.before-ll"
        [void](Assert-LLSafePath $file $Profile)
        [void](Assert-LLSafePath $original $Profile)
        if (-not (Test-Path -LiteralPath $original)) { continue }
        if (Test-Path -LiteralPath $file) {
            if ((Get-FileHash -LiteralPath $file).Hash -eq (Get-FileHash -LiteralPath $original).Hash) { continue }
            # Preserve any edits made since install, and leave the original backup reusable after an interrupted uninstall.
            $saved = "$file.after-ll"; $suffix = 0
            while (Test-Path -LiteralPath $saved) {
                [void](Assert-LLSafePath $saved $Profile)
                if ((Get-FileHash -LiteralPath $saved).Hash -eq (Get-FileHash -LiteralPath $file).Hash) { break }
                $suffix++; $saved = "$file.after-ll.$suffix"
            }
            [void](Assert-LLSafePath $saved $Profile)
            if (-not (Test-Path -LiteralPath $saved)) { Copy-Item -LiteralPath $file -Destination $saved -ErrorAction Stop }
        }
        Copy-Item -LiteralPath $original -Destination $file -Force -ErrorAction Stop
    }
}

function Invoke-LLShellRefresh([int]$AutoHide, $HideIcons, $Arranging) {
    # Isolate COM/Explorer work behind a timeout too. Encoded arguments avoid path/quote interpolation problems.
    $source = $PSCommandPath.Replace("'", "''")
    $hide = if ($null -eq $HideIcons) { -1 } elseif ($HideIcons) { 1 } else { 0 }
    $snap = if ($null -eq $Arranging) { -1 } elseif ($Arranging) { 1 } else { 0 }
    $code = ". '$source'; Initialize-LLRestoreNative; if (-not [LLUninstallShell]::Restore($AutoHide, $hide, $snap)) { exit 1 }"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($code))
    return Invoke-LLBoundedCommand (Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe') @('-NoProfile', '-STA', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $encoded) 10000
}

function Initialize-LLRestoreNative {
    if ('LLUninstallShell' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class LLUninstallShell {
    [StructLayout(LayoutKind.Sequential)] struct AppBar { public int size; public IntPtr hwnd; public uint callback, edge; public int left, top, right, bottom; public IntPtr param; }
    delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, System.Text.StringBuilder name, int size);
    [DllImport("user32.dll")] static extern bool ShowWindowAsync(IntPtr h, int cmd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
    [DllImport("shell32.dll")] static extern IntPtr SHAppBarMessage(uint msg, ref AppBar data);
    [DllImport("user32.dll", SetLastError=true)] static extern bool SystemParametersInfo(uint action, uint param, IntPtr value, uint flags);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr h, uint msg, IntPtr w, string l, uint flags, uint timeout, out IntPtr result);
    // Desktop IFolderView2 via ShellWindows -> IServiceProvider -> IShellBrowser -> active view.
    // https://devblogs.microsoft.com/oldnewthing/20130318-00/?p=4933
    [ComImport, Guid("6d5140c1-7436-11ce-8034-00aa006009fa"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface Provider { [PreserveSig] int QueryService(ref Guid service, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out object result); }
    [ComImport, Guid("000214e2-0000-0000-c000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface Browser {
        void GetWindow(); void ContextSensitiveHelp(); void InsertMenusSB(); void SetMenuSB(); void RemoveMenusSB();
        void SetStatusTextSB(); void EnableModelessSB(); void TranslateAcceleratorSB(); void BrowseObject();
        void GetViewStateStream(); void GetControlWindow(); void SendControlMsg();
        void QueryActiveShellView([MarshalAs(UnmanagedType.Interface)] out object view);
    }
    [ComImport, Guid("1af3a467-214f-4298-908e-06b03e0b39f9"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface FolderView2 {
        void GetCurrentViewMode(); void SetCurrentViewMode(); void GetFolder(); void Item(); void ItemCount(); void Items();
        void GetSelectionMarkedItem(); void GetFocusedItem(); void GetItemPosition(); void GetSpacing(); void GetDefaultSpacing();
        void GetAutoArrange(); void SelectItem(); void SelectAndPositionItems();
        void SetGroupBy(); void GetGroupBy(); void SetViewProperty(); void GetViewProperty(); void SetTileViewProperties();
        void SetExtendedTileViewProperties(); void SetText(); void SetCurrentFolderFlags(uint mask, uint flags);
    }
    static void SetIcons(bool hide) {
        object windows = null, desktop = null, browser = null, view = null;
        try {
            windows = Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("9ba05972-f6a8-11cf-a442-00a0c90a8f39")));
            object[] args = { 0, null /*VT_EMPTY*/, 8 /*SWC_DESKTOP*/, 0, 1 /*SWFO_NEEDDISPATCH*/ };
            var modifier = new System.Reflection.ParameterModifier(5); modifier[3] = true;
            desktop = windows.GetType().InvokeMember("FindWindowSW", System.Reflection.BindingFlags.InvokeMethod, null, windows, args, new[] { modifier }, null, null);
            Guid service = new Guid("4c96be40-915c-11cf-99d3-00aa004ae837"), iid = typeof(Browser).GUID;
            Marshal.ThrowExceptionForHR(((Provider)desktop).QueryService(ref service, ref iid, out browser));
            ((Browser)browser).QueryActiveShellView(out view);
            ((FolderView2)view).SetCurrentFolderFlags(0x1000 /*FWF_NOICONS*/, hide ? 0x1000u : 0u);
        }
        finally {
            foreach (object o in new object[] { view, browser, desktop, windows }) if (o != null && Marshal.IsComObject(o)) Marshal.ReleaseComObject(o);
        }
    }
    public static bool Restore(int autoHide, int hideIcons, int arranging) {
        bool ok = true;
        if (arranging >= 0 && !SystemParametersInfo(0x0083, arranging != 0 ? 1u : 0u, arranging != 0 ? (IntPtr)1 : IntPtr.Zero, 3)) ok = false;
        IntPtr tray = FindWindow("Shell_TrayWnd", null);
        if (autoHide >= 0) {
            if (tray == IntPtr.Zero) ok = false;
            else {
                AppBar data = new AppBar { size = Marshal.SizeOf(typeof(AppBar)), hwnd = tray, param = (IntPtr)autoHide };
                SHAppBarMessage(10, ref data);
                if (SHAppBarMessage(4, ref data).ToInt64() != autoHide) ok = false;
            }
        }
        EnumWindows(delegate(IntPtr h, IntPtr p) {
            var name = new System.Text.StringBuilder(64); GetClassName(h, name, 64);
            if (name.ToString() == "Shell_TrayWnd" || name.ToString() == "Shell_SecondaryTrayWnd") ShowWindowAsync(h, 8 /*SW_SHOWNA*/);
            return true;
        }, IntPtr.Zero);
        if (hideIcons >= 0) { try { SetIcons(hideIcons != 0); } catch { ok = false; } }
        IntPtr result;
        SendMessageTimeout((IntPtr)0xffff, 0x001a, IntPtr.Zero, "TraySettings", 2, 1000, out result);
        SendMessageTimeout((IntPtr)0xffff, 0x001a, IntPtr.Zero, "Environment", 2, 1000, out result);
        return ok;
    }
}
'@ -ErrorAction Stop
}

# Extras kept when Logical Lunge goes: what lives in the install folder (WezTerm with its tools, Everything) is copied
# to the user's programs first, since that folder is deleted. Copied, not moved: an open terminal holds its files (they
# go with the folder once it closes). Returns the new folder, or $null when there is nothing to keep.
function Copy-LLKeptTerminal([string]$App, [string]$Local) {
    $src = Join-Path $App 'tools\wezterm'
    if (-not (Test-Path -LiteralPath (Join-Path $src 'wezterm-gui.exe'))) { return $null }
    $dest = Assert-LLSafePath (Join-Path $Local 'Programs\WezTerm') $Local
    New-Item -ItemType Directory -Path $dest -Force -ErrorAction Stop | Out-Null
    Copy-Item -Path (Join-Path $src '*') -Destination $dest -Recurse -Force -ErrorAction Stop
    $bin = Join-Path $App 'tools\bin'
    if (Test-Path -LiteralPath $bin) {
        $newBin = Join-Path $dest 'bin'
        New-Item -ItemType Directory -Path $newBin -Force -ErrorAction Stop | Out-Null
        Copy-Item -Path (Join-Path $bin '*') -Destination $newBin -Recurse -Force -ErrorAction Stop
    }
    return $dest
}

# Everything: its process and service hold the install folder; both stop, the copy goes to the user's programs and the
# service comes back from there. Returns the new Everything.exe, or $null.
function Copy-LLKeptEverything([string]$App, [string]$Local) {
    $exe = Join-Path $App 'tools\everything\Everything.exe'
    if (-not (Test-Path -LiteralPath $exe)) { return $null }
    $dest = Assert-LLSafePath (Join-Path $Local 'Programs\Everything') $Local
    Get-CimInstance Win32_Process -Filter "Name = 'Everything.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -eq $exe } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    [void](Invoke-LLBoundedCommand $exe @('-uninstall-service') 30000)
    New-Item -ItemType Directory -Path $dest -Force -ErrorAction Stop | Out-Null
    Copy-Item -Path (Join-Path (Split-Path $exe) '*') -Destination $dest -Recurse -Force -ErrorAction Stop
    $new = Join-Path $dest 'Everything.exe'
    [void](Invoke-LLBoundedCommand $new @('-install-service') 30000)
    return $new
}

function New-LLShortcut([string]$Path, [string]$Target) {
    $shell = New-Object -ComObject WScript.Shell
    try {
        $link = $shell.CreateShortcut($Path)
        $link.TargetPath = $Target
        $link.WorkingDirectory = Split-Path $Target
        $link.Save()
    }
    finally { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell) }
}

# Everything of Logical Lunge's data but the logs (kept in every case: what happened stays readable after it is gone)
function Remove-LLDataKeepLogs([string]$Data, [string]$Local) {
    if (-not (Test-Path -LiteralPath $Data)) { return }
    [void](Assert-LLSafePath $Data $Local)
    foreach ($entry in @(Get-ChildItem -LiteralPath $Data -Force -ErrorAction Stop)) {
        if ($entry.Name -ieq 'logs') { continue }
        Remove-LLTree $entry.FullName $Data
    }
}
