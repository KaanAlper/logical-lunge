# Windows görev çubuğunu ve Windows'un kendi ses/parlaklık OSD'sini gizler
# (işlevlerini Zebar'daki ii bar'ı ve OSD'si devraldı).
# GlazeWM başlarken çalışır; explorer yeniden başlarsa tekrar gizler.
# Geri getirmek için: show-taskbar.ps1
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class TB {
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern IntPtr FindWindowEx(IntPtr p, IntPtr a, string c, string t);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
}
'@

while ($true) {
    foreach ($cls in 'Shell_TrayWnd', 'Shell_SecondaryTrayWnd') {
        $h = [TB]::FindWindow($cls, $null)
        while ($h -ne [IntPtr]::Zero) {
            if ([TB]::IsWindowVisible($h)) { [TB]::ShowWindow($h, 0) | Out-Null }
            $h = [TB]::FindWindowEx([IntPtr]::Zero, $h, $cls, $null)
        }
    }

    # Win10 ses/parlaklık/medya flyout'u: NativeHWNDHost > DirectUIHWND.
    # Küçültülmüş (SW_MINIMIZE) host bir daha görünmez — HideVolumeOSD'nin yöntemi.
    $h = [TB]::FindWindow('NativeHWNDHost', $null)
    while ($h -ne [IntPtr]::Zero) {
        if ([TB]::FindWindowEx($h, [IntPtr]::Zero, 'DirectUIHWND', $null) -ne [IntPtr]::Zero -and -not [TB]::IsIconic($h)) {
            [TB]::ShowWindow($h, 6) | Out-Null
        }
        $h = [TB]::FindWindowEx([IntPtr]::Zero, $h, 'NativeHWNDHost', $null)
    }

    Start-Sleep -Milliseconds 700
}
