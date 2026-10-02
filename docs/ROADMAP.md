# Roadmap

Logical Lunge is one install, one uninstall, one autostart and one entry in Settings → Apps. Under the hood it
runs a few cooperating processes, so a crash in one part never takes the others down:

| Process | Language | Role |
|---|---|---|
| `lunge.exe` | C# (.NET Framework 4.8) | Root of the desktop: starts and watches the parts; keyboard and mouse hooks, workspace slides and window animations, focus, rounded corners, notifications, splash, screenshots, wallpapers, night light, shortcuts, updates |
| `lunge-tiling.exe` | Rust | Tiling window manager (Hyprland-style dwindle), window borders, IPC |
| `lunge-shell.exe` | Rust | Bar and panels: native (Direct2D / DirectComposition) in `native-ui`, WebView2 in `web-ui` |
| `lunge-wallpaper.exe` | Rust | Live wallpaper player (Media Foundation, GPU decoding) |

## Two editions

- **native-ui** — the fastest, most stable line. The bar, the Super menu and the notification cards are native;
  each remaining web panel is deleted once its native port lands. No web fallbacks.
- **web-ui** — every panel in WebView2; keeps the web widgets.

Both are built and released from their own branches; `main` holds the shared installer, the setup app and the
release workflows.

## Done

- Single installer and setup app: one UAC prompt, every Windows setting backed up, clean uninstaller, in-place
  updates that ask for permission before the desktop closes.
- Self-healing desktop: the core restarts a crashed or hung part; the Windows taskbar comes back while the bar is
  missing.
- Live wallpapers (store, video files, Lively packages, Wallpaper Engine video projects), screen saver gallery
  with bulk import, Windows notifications as Logical Lunge cards, do-not-disturb.
- Native bar, Super menu and notification cards (`native-ui`).
- 24/7 hardening: bounded caches and logs, stuck-animation recovery, non-blocking event stream, timeouts on
  window manager calls, temp file sweep.

## Next (native-ui)

1. Native ports, each removing its HTML: session screen, update card, dock, on-screen keyboard, sidebar
   (quick settings, notification centre, media, wallpaper and live wallpaper pages, shortcut editor), settings.
   After the last one WebView2 leaves `native-ui` entirely.
2. Super menu text box: mouse selection and drag, double-click, undo, IME placement, text cursors, right-to-left
   text, width and shape animations, accessibility (UI Automation).
3. Readings that still start a PowerShell per call (radios, Ethernet, Bluetooth, status, keep-awake) move into
   the core.

## Later

- Desktop widgets.
- UI scale setting.
- Logical Lunge screen savers and downloadable extras.
- Screenshots and clean-machine tests on Windows 10 and Windows 11 for every release.

## Compatibility notes

- **Windows 10 / 11**: both supported. Windows 11 24H2 changed the desktop's window layout; the live wallpaper
  player handles the old and the new one.
- **Multiple monitors / DPI**: every part is per-monitor DPI aware (v2).
- **ARM64**: not built yet; every part would need an ARM64 build.
