# Roadmap

Logical Lunge is one install, one uninstall, one autostart and one entry in Settings → Apps. Under the hood it
runs a few cooperating processes, so a crash in one part never takes the others down:

| Process | Language | Role |
|---|---|---|
| `lunge.exe` | C# (.NET Framework 4.8) | Root of the desktop: starts and watches the parts; keyboard and mouse hooks, workspace slides and window animations, focus, rounded corners, notifications and dialogs, Windows parts takeover, wallpapers, night light, shortcuts, updates |
| `lunge-tiling.exe` | Rust | Tiling window manager (Hyprland-style dwindle), window borders, IPC |
| `lunge-shell.exe` | Rust | The shell: every surface native (Direct2D / DirectComposition, plain Win32) in `native-ui`; WebView2 widgets in `web-ui` |
| `lunge-wallpaper.exe` / `LogicalLunge.scr` | Rust | Live wallpaper player and video screen saver (Media Foundation, GPU decoding) |

## Two editions

- **native-ui** — the fastest, most stable line. Every panel is native; WebView2 and Tauri are gone. No web
  fallbacks.
- **web-ui** — every panel in WebView2; keeps the web widgets.

Both are built and released from their own branches; `main` holds the shared installer, the setup app and the
release workflows. Releases follow semantic versioning per edition (feat → MINOR, fix → PATCH, breaking → MAJOR).

## Done

- Single installer and setup app (x64 / x86): one UAC prompt, every Windows setting backed up, rollback on failure,
  clean uninstaller, in-place updates that ask for permission before the desktop closes.
- Self-healing desktop: the core restarts a crashed or hung part; Windows' taskbar comes back while the bar is
  missing.
- Native ports (`native-ui`): bar, Super menu, notification cards, right panel, settings, Dock, on-screen keyboard,
  session screen, update card, shared menu and dialog; then Tauri and WebView2 removed from the shell.
- Super menu text box: mouse selection, undo/redo, IME, cursors, right-to-left text, width and shape animations;
  app list with Steam / Epic / itch shortcuts and live refresh on installs.
- Desktop: our own right-click menu everywhere (desktop, icons, apps, bar, cards), desktop widgets (`native-ui`),
  Windows parts switched off at their source while running (taskbar, banners, Snap, Win-key shell) and restored.
- Keyboard ownership (no Win combination reaches Windows' shell) and the shortcut editor (custom apps, conflicts,
  staged save).
- Errors at the source: our programs show cards and dialogs instead of Windows' boxes; one launch helper.
- Live wallpapers and video screen saver with one store and one gallery, imports (video, Lively, Wallpaper Engine
  video), screen saver gallery with bulk import.
- Interface scale (85–150 %) and Windows' Text size followed by native text.
- Core readings (radios, Wi-Fi, Ethernet, Bluetooth, status, keep-awake) and the Super menu's run / app list moved
  from PowerShell into the core; Everything rescans for non-NTFS locations.
- 24/7 hardening: bounded caches and logs, stuck-animation recovery, non-blocking event stream, timeouts on
  window manager calls, temp file sweep.
- Legacy 0.1.x stack removed.

## Next

1. The same installer, setup app and semantic-versioned release workflow for the author's other Windows projects.
2. Screenshots and clean-machine tests on Windows 10 and Windows 11 for every release.
3. Web demo kept in sync with `web-ui` automatically (workflow added; needs the `DEMO_PUSH_TOKEN` secret and mocks
   for the newer panels).

## Later

- Desktop widgets for `web-ui`.
- Accessibility (UI Automation) for the native surfaces.
- Downloadable extras.
- ARM64 builds.

## Compatibility notes

- **Windows 10 / 11**: both supported. Windows 11 24H2 changed the desktop's window layout; the live wallpaper
  player handles the old and the new one.
- **Multiple monitors / DPI**: every part is per-monitor DPI aware (v2); the interface scale multiplies on top.
- **ARM64**: not built yet; every part would need an ARM64 build.
