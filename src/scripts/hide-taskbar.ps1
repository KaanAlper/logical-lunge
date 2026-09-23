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
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint c);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr l);
  static string Cls(IntPtr h) { var s = new System.Text.StringBuilder(64); GetClassName(h, s, 64); return s.ToString(); }
  // Başlat düğmesi: görev çubuğunun sahip olduğu üst düzey Button (FindWindow onu bulmuyor)
  public static void StartButtons(bool show) {
    EnumWindows((h, l) => { if (Cls(h) == "Button" && Cls(GetWindow(h, 4)).StartsWith("Shell_") && IsWindowVisible(h) != show) ShowWindow(h, show ? 5 : 0); return true; }, IntPtr.Zero);
  }
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

    # Win10 Başlat düğmesi görev çubuğunun sahip olduğu AYRI bir üst pencere (Button); görev çubuğu gizlenince
    # ikinci monitördeki sol altta tek başına kalıyordu.
    [TB]::StartButtons($false)

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
