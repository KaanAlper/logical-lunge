# hide-taskbar.ps1 döngüsünü durdurur ve Windows görev çubuğunu geri getirir.
Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" |
    Where-Object { $_.CommandLine -like '*hide-taskbar.ps1*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }

Add-Type @'
using System; using System.Runtime.InteropServices;
public static class TB2 {
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern IntPtr FindWindowEx(IntPtr p, IntPtr a, string c, string t);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
}
'@
$h = [TB2]::FindWindow('NativeHWNDHost', $null)
while ($h -ne [IntPtr]::Zero) {
    if ([TB2]::FindWindowEx($h, [IntPtr]::Zero, 'DirectUIHWND', $null) -ne [IntPtr]::Zero) { [TB2]::ShowWindow($h, 9) | Out-Null }
    $h = [TB2]::FindWindowEx([IntPtr]::Zero, $h, 'NativeHWNDHost', $null)
}

foreach ($cls in 'Shell_TrayWnd', 'Shell_SecondaryTrayWnd') {
    $h = [TB2]::FindWindow($cls, $null)
    while ($h -ne [IntPtr]::Zero) {
        [TB2]::ShowWindow($h, 5) | Out-Null
        $h = [TB2]::FindWindowEx([IntPtr]::Zero, $h, $cls, $null)
    }
}
