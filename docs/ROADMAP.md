# Roadmap: from three programs to one

Today Logical Lunge is **one install, one uninstall, one autostart, one entry in Settings → Apps** and no
foreign tray icons or settings windows, but under the hood it runs three cooperating programs:

| Process | Language | Role |
|---|---|---|
| `glazewm.exe` | Rust | Tiling window manager, window borders, IPC server (ws://127.0.0.1:6123) |
| `zebar.exe` | Rust + Tauri (WebView2) | Hosts the widgets: bar, sidebar, overview, session screen, toast, on-screen keyboard |
| `ll-helper.exe` | C# (.NET Framework 4.8, built with the in-box `csc`) | Keyboard/mouse hooks, workspace slides and window animations, focus-follows-mouse, rounded corners, toasts, splash, screenshot/Lens tools, wallpapers, gamma / night light, shortcuts, watchdogs |

## Done: one app built from stripped forks

**GlazeWM** (fork `KaanAlper/glazewm`, branch `logical-lunge`)
- Tray icon and its tray-only dependencies removed.
- Hyprland dwindle layout (placement under the mouse, `movewindow`, space goes back to the split partner).
- tacky-borders' drawing engine (MIT) built in as the `wm-borders` crate: borders only on managed windows, placed
  in the same step as the window, configured by the `borders:` section of the WM config; a crash in the engine
  never takes the WM down. The separate tacky-borders program is gone.
- Starts while the screen is locked, waits for a busy IPC port after a crash, never leaves windows hidden
  after a failed start, uncloaks every window on exit.

**Zebar** (fork `KaanAlper/zebar`, branch `logical-lunge`)
- Tray icon, widget manager / settings window (`packages/settings-ui`), client package, marketplace, starter pack,
  `zebar publish`, templates, preview widgets and unused providers (`weather`, `ip`, `keyboard`, `disk`,
  `komorebi`) removed. Built with `cargo` only.
- The asset server always comes up; a broken `settings.json` falls back to the shell's widgets.

**ll-helper** — watchdogs: restarts GlazeWM (crash or 15 s hang) and Zebar, restarts itself on a crash or a
frozen UI, and is restarted by Zebar's notification bridge if it dies; falls back to the Windows taskbar and Start
menu while the bar is missing; *Reload desktop* in the session screen, sidebar and Start menu.

## Plan

1. **Done — single installer.** `install.ps1` → one UAC prompt, pinned upstream versions, every Windows
   setting backed up, clean uninstaller, no Python needed on the target (PyInstaller-packed helpers).
2. **Done — stripped forks**, built in GitHub Actions with every release (above).
3. **Next — ll-helper as the root.** It starts and stops GlazeWM and Zebar (instead of GlazeWM's startup
   commands), so *reload* can restart a single part; the pollers that start a PowerShell per reading
   (brightness, Bluetooth) move into it; windows keep their workspaces across a WM restart.
4. **Maybe — one executable.** A Rust host running GlazeWM's WM loop and the Zebar/Tauri runtime in one
   process, absorbing ll-helper piece by piece. Weighed against crash isolation: today a crash in one part
   never takes the others down.
5. **Screenshots + clean-machine tests** on Windows 10 and Windows 11 for every release.

## Compatibility notes

- **Windows 11**: GlazeWM, Zebar, WebView2, the DWM thumbnail animations, `IDesktopWallpaper` and the
  low-level hooks all work the same. Windows 11 adds snap layouts (maximize-button flyout, drag-to-top bar)
  — the installer turns them off. Windows 11 draws its own rounded corners; ours are applied on top and
  look the same.
- **Multiple monitors / DPI**: every helper is per-monitor DPI aware (v2).
- **ARM64**: GlazeWM and Zebar ship ARM64 builds; ll-helper and the packaged tools would need ARM64
  builds too.
