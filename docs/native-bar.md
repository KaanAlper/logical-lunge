# Native bar

The bar is the only widget that is always on screen, yet today it costs a whole browser tab: every widget is a
WebView2 page (one renderer each, ~88 MB), plus the shared browser and GPU processes (~230 MB). The native bar draws
the same ii bar with Direct2D + DirectWrite into DirectComposition surfaces, inside `lunge-shell`, with no WebView.

Goals: same look and behaviour as `ui/bar.html` (nothing lost, see the map below); a few MB instead of ~100; animations
that cost no CPU per frame; works on machines without a GPU driver (WARP) and without Explorer (shell mode).

## Where it lives

`lunge-shell` (`shell/packages/desktop/src/native_bar/`), on its own UI thread with a Win32 message loop.

- **Data**: the shell's providers (cpu, memory, battery, network, audio, media, systray) are created through
  `ProviderManager` like a widget would, and their emissions are also forwarded to the bar thread. No second copy of
  any provider.
- **Window manager**: WebSocket client to `ws://127.0.0.1:6123` (same protocol as `ui/lib/tiling-client.js`):
  `sub -e all` plus `query monitors / workspaces / focused / paused / binding-modes`, and `command` for actions.
- **Core**: HTTP to `127.0.0.1:6131` (`/cmd?a=ws-N` for the slide, `/bar-alive`, `/apps.json`, `/winicon`,
  `/prefs.json`, `/cmd?a=...` for sidebar / overview / OSK toggles, which the core forwards to the widgets as `ll:*`
  events through its SSE stream).

## Rendering

- One D3D11 device (hardware, else WARP) + one D2D device context + one `IDCompositionDesktopDevice` for all bars.
- Each monitor: a window with `WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, title
  `Logical Lunge · bar` (the core finds the bar by this title: slides, no-activate, focus guard, taskbar fallback,
  splash, bar-alive), per-monitor DPI v2.
- Visual tree: background + groups (static surface, redrawn only when content changes), workspace indicator
  (rounded-rect clip whose edges are animated), popups / OSD (own visuals: offset + opacity + clip animations).
- Animations are `IDCompositionAnimation` curves built from the same cubic beziers as the web bar (sampled into
  cubic segments), so they run in the compositor: the bar thread can sleep while a popup slides.
- Text: DirectWrite with the bundled WOFF2 fonts unpacked in memory (`IDWriteFactory5::UnpackFontFile`):
  Google Sans Flex (wght 450), Rubik fallback, Material Symbols Rounded with its FILL / wght / opsz axes
  (icons are ligatures: drawing the text `search` draws the icon).
- Images (app icons, window icons, tray icons, album art): WIC decode into D2D bitmaps, cached.

## Responsibility map (ui/bar.html → native)

| # | Web bar | Native bar |
|---|---|---|
| 1 | i18n.json (13 languages, RTL for Arabic), language from prefs.json | same file, same exact-match + pattern lookup; RTL mirrors the layout |
| 2 | Theme dark / light (`ll.theme` in WebView localStorage, shared by `storage` event) | theme moves to the core's prefs (read by bar and widgets; toggles go through the core) |
| 3 | Clock 12 / 24 h (prefs), date `EEEE, dd/MM` in the UI language | same (ICU-free: Win32 `GetDateFormatEx` with the locale) |
| 4 | Per-monitor bar, OSD only on one monitor (wheel bar or focused monitor) | one window per monitor, OSD rule unchanged |
| 5 | Window grows for popups / OSD, always-on-top while grown | popups are separate layered visuals of the same window; window grows the same way |
| 6 | Workspace click / wheel → core slide (`/cmd?a=ws-N`), fallback `lunge.exe --slide`, fallback WM command | same chain |
| 7 | Brightness wheel (left edge): one axis gamma 0..100 then brightness 0..100; DDC / WMI via `brightness.ps1`, gamma via `lunge.exe --gamma` | same commands, same caches and 30 s refresh |
| 8 | Volume wheel (right edge) ±5, unmute on up | audio provider functions |
| 9 | `bar-alive` heartbeat every 30 s | same request from the bar thread (proves the UI thread is alive) |
| 10 | Hover popups: resources (RAM / swap / CPU / temps via `lunge-temps.exe --read` every 2 s while open), media (art via `lunge-media.exe`, seek `--seek`, prev / play / next, time ticking) with 200 ms close delay and slide in / out | same |
| 11 | OSD: volume (any change), mic mute, brightness, gamma; 1 s | same |
| 12 | Left: search button (overview), active window (class + title, click → left sidebar) at full width, paused pill, binding-mode pills, scroll hint | same |
| 13 | Middle: resources rings (warn colours), media ring + title • artist (click play/pause, right next, middle prev), 10 workspaces per page, merged occupied runs, trailing active indicator (100 / 300 ms OutSine), app icons (apps.json → `/winicon` fallback), tooltips, right click → overview; clock • date; util buttons (snip, OSK, theme); battery | same |
| 14 | Right: indicators (muted, mic off, network type / Wi-Fi strength) → right sidebar; tray (pinned in bar, rest in the ▾ panel, drag to pin with ghost, first 4 pinned, key = first tooltip word; left / double / middle / right click) | same; pins move from localStorage to the shell's state file |
| 15 | Shorten levels by width (≤1100 / ≤1440) | same |
| 16 | `ll:bar-click` on mouse down (other popups close) | through the core's event stream |
| 17 | `ll:overview-toggle`, `ll:sidebar-right-toggle`, `ll:sidebar-left-toggle`, `ll:osk-toggle`, `ll:osd-wheel` | through the core's event stream |

Kept as they are: the WM reserves the bar area through `gaps.outer_gap.top` in config.yaml; the core's
integration points all key on the window title.

## Switching and fallback

`bar: native | web` in config.yaml (default native once it is complete). If the native bar cannot start (no D3D and no
WARP, a crash loop), the shell starts the web bar widget instead, so there is always a bar.

## Phases

1. Skeleton: window per monitor, device, DComp tree, fonts, clock, workspaces from the WM, layout and shorten levels.
2. All indicators and actions: resources, media, battery, network, audio, tray, wheels, OSD.
3. Popups (resources, media, tray panel with drag to pin) and all animations in the compositor.
4. Integration: config switch and fallback, theme / pins / i18n / prefs, heartbeat, DPI and monitor changes,
   measurements against the web bar (memory, CPU at idle, frame times of the slide).
5. Web widgets only while open (low-memory mode): the shell creates sidebar / overview / settings on demand and closes
   them after use, so no WebView2 process runs while nothing web is on screen.
