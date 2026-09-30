using System;
using System.Collections.Generic;

internal static class WorkspaceConfigTests
{
    static void Check(bool ok, string label)
    {
        if (!ok) throw new Exception(label);
    }

    static void Main()
    {
        const string yaml = "anims:\n  workspaces: { duration: 520 }\nworkspaces:\n  - name: '1'\n    display_name: 'Home'\n    bind_to_monitor: 1\n    keep_alive: true\n  - name: '2'\n    bind_to_monitor: 2\nwindow_rules:\n  - commands: ['ignore']\n";
        List<WorkspaceConfigText.Entry> entries;
        string error;
        Check(WorkspaceConfigText.TryRead(yaml, out entries, out error), "read existing config");
        Check(entries.Count == 2 && entries[0].Monitor == 1 && entries[1].Monitor == 2, "read bindings");
        string next;
        var map = new Dictionary<int, int> { { 1, 1 }, { 2, 0 }, { 3, 0 } };
        Check(WorkspaceConfigText.TryRewrite(yaml, 3, 2, map, 2, out next, out error), "rewrite config");
        Check(next.Contains("display_name: 'Home'") && next.Contains("keep_alive: true"), "preserve workspace options");
        Check(next.Contains("  - name: '2'\n    bind_to_monitor: 0"), "zero-based monitor index");
        Check(next.Contains("  - name: '3'\n    bind_to_monitor: 0"), "new workspace");
        Check(next.Contains("anims:\n  workspaces: { duration: 520 }"), "leave unrelated YAML alone");
        Check(next.Contains("window_rules:\n  - commands: ['ignore']"), "leave following section alone");
        Check(WorkspaceConfigText.TryRead(next, out entries, out error) && entries.Count == 3, "roundtrip");
        Check(entries[0].Number == 2 && entries[1].Number == 3 && entries[2].Number == 1, "rotate workspace order");
        Check(WorkspaceConfigText.TryRewrite(yaml + "shortcuts: []\n", 2, 1, new Dictionary<int, int>(), 2, out next, out error) &&
            next.Contains("window_rules:\n  - commands: ['ignore']\nshortcuts: []"), "stop at inline top-level section");
        Check(WorkspaceConfigText.TryRewrite(yaml, null, 2, new Dictionary<int, int>(), 2, out next, out error), "order-only save");
        Check(next.Contains("  - name: '2'\n    bind_to_monitor: 2") && next.Contains("  - name: '1'\n    display_name: 'Home'\n    bind_to_monitor: 1"), "order-only save preserves monitor bindings");
        var ten = "workspaces:\n";
        for (int i = 1; i <= 10; i++) ten += "  - name: '" + i + "'\n    bind_to_monitor: 0\n";
        Check(WorkspaceConfigText.TryRewrite(ten, 10, 7, new Dictionary<int, int>(), 2, out next, out error), "rotate ten workspaces");
        Check(WorkspaceConfigText.TryRead(next, out entries, out error), "read rotated ten workspaces");
        Check(string.Join(" ", entries.ConvertAll(e => e.Number.ToString()).ToArray()) == "7 8 9 10 1 2 3 4 5 6", "requested 7 to 6 order");
        Check(WorkspaceConfigText.TryRewrite(yaml, 100, 100, new Dictionary<int, int>(), 2, out next, out error), "support hundred workspace labels");
        Check(WorkspaceConfigText.TryRead(next, out entries, out error) && entries.Count == 100 && entries[0].Number == 100 && entries[1].Number == 1, "hundred workspace order");
        Check(!WorkspaceConfigText.TryRewrite(yaml, 101, 1, new Dictionary<int, int>(), 2, out next, out error), "reject oversized count");
        Check(!WorkspaceConfigText.TryRewrite(yaml, 2, null, new Dictionary<int, int> { { 2, 2 } }, 2, out next, out error), "reject missing monitor");
        Check(!WorkspaceConfigText.TryRewrite(yaml, 2, null, new Dictionary<int, int> { { 1, 0 }, { 2, 0 } }, 2, out next, out error), "keep every monitor populated");
        Check(!WorkspaceConfigText.TryRewrite(yaml, 1, null, new Dictionary<int, int>(), 2, out next, out error), "reject fewer workspaces than monitors");
        Console.WriteLine("Workspace config tests passed");
    }
}
