# ii "Keep awake" (idle inhibitor): çalıştığı sürece ekran ve sistem uykuya geçmez.
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class KA { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint f); }
'@
# ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
[KA]::SetThreadExecutionState(0x80000003) | Out-Null
while ($true) { Start-Sleep -Seconds 60 }
