using System;
using System.Collections.Generic;
using System.Management;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;

// Match a WM DISPLAYn adapter to its EDID model name. EnumDisplayDevices
// provides the monitor interface ID; WmiMonitorID provides UserFriendlyName.
// Both APIs are optional here: a disconnected or virtual display keeps its
// WM device name if Windows does not expose a model.
internal static class MonitorFriendlyNames
{
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct DisplayDevice
    {
        public int cb;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string DeviceName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceString;
        public int StateFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceID;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceKey;
    }

    [DllImport("user32.dll", EntryPoint = "EnumDisplayDevicesW", CharSet = CharSet.Unicode)]
    static extern bool EnumDisplayDevices(string adapter, uint index, ref DisplayDevice device, uint flags);

    static string Key(string interfaceId)
    {
        var match = Regex.Match(interfaceId ?? "", @"DISPLAY#([^#]+)#([^#]+)#", RegexOptions.IgnoreCase);
        return match.Success ? "DISPLAY\\" + match.Groups[1].Value + "\\" + match.Groups[2].Value : null;
    }

    // WMI monitor rows (WmiMonitorID, WmiMonitorBrightness, ...) are named "<key>_<n>". The keys of the monitors on
    // one adapter ("\\.\DISPLAY1"); empty when Windows exposes none.
    internal static List<string> InstanceKeys(string adapter)
    {
        var keys = new List<string>();
        try {
            for (uint i = 0; i < 8; i++) {
                var device = new DisplayDevice { cb = Marshal.SizeOf(typeof(DisplayDevice)) };
                if (!EnumDisplayDevices(adapter, i, ref device, 1)) break;
                var key = Key(device.DeviceID);
                if (key != null) keys.Add(key);
            }
        } catch { }
        return keys;
    }

    internal static bool IsInstanceOf(string instanceName, string key)
    {
        return instanceName != null && key != null &&
            (instanceName.StartsWith(key + "_", StringComparison.OrdinalIgnoreCase) || instanceName.Equals(key, StringComparison.OrdinalIgnoreCase));
    }

    internal static Dictionary<string, string> Read(IEnumerable<string> adapterNames)
    {
        var models = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        try {
            using (var search = new ManagementObjectSearcher("root\\wmi", "SELECT InstanceName, UserFriendlyName, UserFriendlyNameLength FROM WmiMonitorID WHERE Active = True"))
            using (var rows = search.Get())
                foreach (ManagementBaseObject row in rows) {
                    var codes = row["UserFriendlyName"] as ushort[];
                    int length = Convert.ToInt32(row["UserFriendlyNameLength"]);
                    if (codes == null || length <= 0) continue;
                    var name = new StringBuilder();
                    for (int i = 0; i < Math.Min(length, codes.Length) && codes[i] != 0; i++) name.Append((char)codes[i]);
                    if (name.Length > 0) models[Convert.ToString(row["InstanceName"])] = name.ToString();
                }
        } catch { return new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase); }

        var result = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var adapter in adapterNames) {
            foreach (var key in InstanceKeys(adapter)) {
                foreach (var model in models)
                    if (IsInstanceOf(model.Key, key)) {
                        result[adapter] = model.Value;
                        break;
                    }
                if (result.ContainsKey(adapter)) break;
            }
        }
        return result;
    }
}
