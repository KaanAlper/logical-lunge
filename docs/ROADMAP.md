# Roadmap: from three programs to one

Today Logical Lunge is **one install, one uninstall, one autostart and one entry in Settings → Apps**, but
under the hood it still runs three programs that talk to each other:

| Process | Language | Role |
|---|---|---|
| `glazewm.exe` | Rust | Tiling window manager, IPC server (ws://127.0.0.1:6123) |
| `zebar.exe` | Rust + Tauri (WebView2) | Hosts the widgets: bar, sidebar, overview, toast, on-screen keyboard |
| `ll-helper.exe` | C# (.NET Framework 4.8, built with the in-box `csc`) | Keyboard/mouse hooks, workspace slides and window animations, dwindle, focus-follows-mouse, rounded corners, toasts, splash, screenshot/Lens tools, wallpapers, gamma / night light, shortcuts |

## What the shell does not use (to be removed in the forks)

**Zebar**
- The widget manager / marketplace window (`packages/settings-ui`) and its tray icon + tray menu — the
  shell has exactly one widget pack and starts it itself.
- Marketplace downloads, pack publishing (`zebar publish`), preset selection UI.
- Hot reloading of arbitrary user packs and the `settings.json` startup list (hard-code our five widgets).
- Providers the shell never calls: `weather`, `ip`, `keyboard`, `disk`, `komorebi`.

**GlazeWM**
- Tray icon and its menu (the bar is the UI).
- Its own keybinding engine for the combos ll-helper already owns (Super+arrows, Super+number,
  Super+Ctrl+arrows) — kept only for the extra combos listed in `config.yaml`.
- Kept: `wm-watcher` (restores windows if the WM crashes), IPC, window rules.

## Plan

1. **Done — single installer.** `install.ps1` → one UAC prompt, pinned upstream versions, every Windows
   setting backed up, clean uninstaller, no Python needed on the target (PyInstaller-packed helpers).
2. **Forks — in progress.** Local forks (branch `logical-lunge`, next to this repo in `..\logical-lunge-forks`):
   - GlazeWM: tray icon and its tray-only dependencies removed (`tray-icon`, `image`, `auto-launch`) — 8.0 → 6.2 MB.
   - Zebar: tray icon / widget-manager entry point removed; `disk`, `ip`, `keyboard`, `komorebi`, `weather` providers and the`n     `komorebi-util` crate removed.
   - `build.ps1` builds them and ships them in `logical-lunge\bin`; `setup.ps1` uses them instead of the upstream MSIs.
   - Still to do: remove the marketplace installer / `publish` CLI / settings-ui from Zebar, publish the forks on GitHub,
     build them in GitHub Actions.
3. **One executable.** A Rust host (`logical-lunge.exe`) that:
   - runs GlazeWM's WM loop in-process (its `wm` crate as a library on a dedicated thread with its own
     Win32 message pump),
   - runs the Zebar/Tauri runtime on the main thread,
   - talks to both without the websocket IPC hop,
   - starts ll-helper as a child at first, then absorbs it piece by piece (hooks and animations first,
     because they benefit most from sharing state with the WM).
4. **Screenshots + clean-machine tests** on Windows 10 and Windows 11 for every release.

## Compatibility notes

- **Windows 11**: GlazeWM, Zebar, WebView2, the DWM thumbnail animations, `IDesktopWallpaper` and the
  low-level hooks all work the same. Windows 11 adds snap layouts (maximize-button flyout, drag-to-top bar)
  — the installer turns them off. Windows 11 draws its own rounded corners; ours are applied on top and
  look the same.
- **Multiple monitors / DPI**: every helper is per-monitor DPI aware (v2).
- **ARM64**: GlazeWM and Zebar ship ARM64 builds; ll-helper and the packaged tools would need ARM64
  builds too.
