# Roadmap

Logical Lunge is **one app**: one install, one uninstall, one autostart, one entry in Settings → Apps, one config
folder, one log folder, and Task Manager shows a single *lunge*. Under the hood it is a small family of cooperating
processes, so a crash in one part never takes the others down:

| Process | Source | Language | Role |
|---|---|---|---|
| `lunge.exe` | `core/` | C# (.NET Framework 4.8, built with the in-box `csc`) | Root of the desktop: starts the other parts as its children and restarts them if they crash or hang. Keyboard/mouse hooks, workspace slides and window animations, focus-follows-mouse, rounded corners, toasts, splash, screenshot/Lens tools, wallpapers, gamma / night light, shortcuts |
| `lunge-tiling.exe` | `tiling/` | Rust | Tiling window manager (Hyprland dwindle), window borders, IPC server (ws://127.0.0.1:6123) |
| `lunge-shell.exe` | `shell/` | Rust + Tauri (WebView2) | Hosts the widgets in `ui/`: bar, sidebar, overview, session screen, toasts, on-screen keyboard |

## Done

- **Single installer** with a first-install wizard (focus color, language, clock, extras), one UAC prompt,
  every Windows setting backed up, automatic rollback on an error or Ctrl+C, clean uninstaller, no Python needed
  on the target (PyInstaller-packed tools).
- **One repository, own names** (0.2): the window manager and the widget host live in this repo as `tiling/` and
  `shell/`, stripped to what the desktop uses (no tray icons, settings windows, marketplace, update checks or
  packaging of their own). Borders are drawn inside the window manager (`wm-borders`).
- **The core is the root** (0.2): the sign-in task starts only `lunge.exe`; it opens the splash, the window
  manager and the shell. *Reload desktop* (`lunge.exe --restart-desktop`) restarts everything cleanly; an
  intentional exit hands the desktop back to Windows (taskbar and Start menu).
- **Self-healing**: crashed or hung parts are restarted; while the bar is missing the Windows taskbar and Start
  menu come back; a black box logs what the machine was doing when the desktop slows down.

## Next

1. **Vendored UI runtime.** The widgets load their UI libraries from the app itself instead of a CDN (faster
   start, works offline, safe to run elevated).
2. **Elevated core and window manager** (after 1): hotkeys keep working while an administrator window such as
   Task Manager is focused; apps are still started without elevation.
3. **Settings window** (gear in the sidebar): focus color, animations, shortcuts, night light, health.
4. **Core split into domain files** (input, animation, windows, shell, health) and dead code removed.
5. **Maybe — one executable.** Weighed against crash isolation.
6. **Screenshots + clean-machine tests** on Windows 10 and Windows 11 for every release.

## Compatibility notes

- **Windows 11**: the window manager, WebView2, the DWM thumbnail animations, `IDesktopWallpaper` and the
  low-level hooks all work the same. Windows 11 adds snap layouts (maximize-button flyout, drag-to-top bar)
  — the installer turns them off. Windows 11 draws its own rounded corners; ours are applied on top and
  look the same.
- **Multiple monitors / DPI**: every part is per-monitor DPI aware (v2).
- **ARM64**: the Rust parts build for ARM64; the core and the packaged tools would need ARM64 builds too.
