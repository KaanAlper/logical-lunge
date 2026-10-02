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
- **Core**: HTTP to `127.0.0.1:6131`: `/cmd?a=ws-N` for the slide, `/bar-alive`, `/apps.json`, `/winicon`,
  `/pref?k=theme&v=...` and `/tray-pins`. The core's event stream (`/events`, the same one the web widgets get through
  the toast widget) brings `ll:theme-*` and `ll:tray-pins`; the bar reconnects to it on its own.
- **Web widgets**: the buttons that open them (search, active window, OSK, indicators, right click on the workspaces)
  and a press anywhere else on the bar (`ll:bar-click`) emit the same Tauri events as the web bar, in process.

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
| 2 | Theme dark / light (`ll.theme` in WebView localStorage, shared by `storage` event) | one theme in prefs.json `theme`: any toggle writes it through the core, the core emits `ll:theme-*`, the toast widget copies it into localStorage for the web widgets |
| 3 | Clock 12 / 24 h (prefs), date `EEEE, dd/MM` in the UI language | same (ICU-free: Win32 `GetDateFormatEx` with the locale) |
| 4 | Per-monitor bar, OSD only on one monitor (wheel bar or focused monitor) | one window per monitor, OSD rule unchanged |
| 5 | Window grows for popups / OSD, always-on-top while grown | popups are separate layered visuals of the same window; window grows the same way |
| 6 | Workspace click / wheel → core slide (`/cmd?a=ws-N`), fallback `lunge.exe --slide`, fallback WM command | same chain |
| 7 | Brightness wheel (left edge): one axis gamma 0..100 then brightness 0..100; DDC / WMI and gamma through the core's `/brightness` and `/gamma` routes (in-process, latest value of a burst wins) | same routes, same caches and 30 s refresh |
| 8 | Volume wheel (right edge) ±5, unmute on up | audio provider functions |
| 9 | `bar-alive` heartbeat every 30 s | same request from the bar thread (proves the UI thread is alive) |
| 10 | Hover popups: resources (RAM / swap / CPU / temps via `lunge-temps.exe --read` every 2 s while open), media (art via `lunge-media.exe`, seek `--seek`, prev / play / next, time ticking) with 200 ms close delay and slide in / out | same |
| 11 | OSD: volume (any change), mic mute, brightness, gamma; 1 s | same |
| 12 | Left: search button (overview), active window (class + title, click → left sidebar) at full width, paused pill, binding-mode pills, scroll hint | same |
| 13 | Middle: resources rings (warn colours), media ring + title • artist (click play/pause, right next, middle prev), 10 workspaces per page, merged occupied runs, trailing active indicator (100 / 300 ms OutSine), app icons (apps.json → `/winicon` fallback), tooltips, right click → overview; clock • date; util buttons (snip, OSK, theme); battery | same |
| 14 | Right: indicators (muted, mic off, network type / Wi-Fi strength) → right sidebar; tray (pinned in bar, rest in the ▾ panel, drag to pin with ghost, first 4 pinned, key = first tooltip word; left / double / middle / right click) | same; pins live in `state\tray-pins.json` (core `/tray-pins`, used by both bars; the old localStorage layout is moved over once). Without a tooltip the key is the owner's exe name |
| 15 | Shorten levels by width (≤1100 / ≤1440) | same |
| 16 | `ll:bar-click` on mouse down (other popups close) | same event, except on the presses that toggle a panel; the sidebar listens too (this bar never takes focus, so it gets no blur) |
| 17 | `ll:overview-toggle`, `ll:sidebar-right-toggle`, `ll:sidebar-left-toggle`, `ll:osk-toggle`, `ll:osd-wheel` | same Tauri events (the OSD is the bar's own) |

Kept as they are: the WM reserves the bar area through `gaps.outer_gap.top` in config.yaml; the core's
integration points all key on the window title.

## Fallback

The web bar is removed (it is kept in the `web-ui` branch). When the native bar fails (a panic in its window procedure,
the graphics device not coming back for a minute, its thread ending), its UI thread ends and a guard thread builds a new
bar after 1-2 s; the last state of every source (providers, window manager, app list) is given to it at once, so it is
full from its first frame. The shell and the other panels are not touched. After three failures in two minutes, or when
the shell itself died at its last three starts, the shell goes on without a bar and the core brings Windows' taskbar and
Start menu back (after 20 s without a bar).

`LL_NATIVE_BAR=off` starts no bar; `LL_NATIVE_BAR=demo` runs a test bar next to the running one;
`LL_NATIVE_BAR_FAIL_AFTER=<s>` makes the first bar panic after that many seconds (checked 2026-09-27: the new bar was up
1.1 s later with its content, the process kept running).

## Measured (2026-09-27, two monitors, same session)

| | web bar | native bar |
|---|---|---|
| `lunge-shell` + its WebView2 processes | 17 processes, 822 MB private, 435 threads, 7426 handles | 15 processes, 725 MB private, 410 threads, 6837 handles |
| `lunge-shell` alone | 63 MB | 140 MB (D3D / D2D / DirectWrite; room to shrink) |

GDI objects stay flat over hours with either bar.

## Phases

Status: 1-4 done and checked on screen (popups, tooltips, drag to pin and back, outside click, second monitor); the web
bar is removed. Phase 5 is superseded: the other panels become native too (docs/native-overview.md).

1. Skeleton: window per monitor, device, DComp tree, fonts, clock, workspaces from the WM, layout and shorten levels.
2. All indicators and actions: resources, media, battery, network, audio, tray, wheels, OSD.
3. Popups (resources, media, tray panel with drag to pin) and all animations in the compositor.
4. Integration: config switch and fallback, theme / pins / i18n / prefs, heartbeat, DPI and monitor changes,
   measurements against the web bar (memory, CPU at idle, frame times of the slide).
5. Web widgets only while open (low-memory mode): the shell creates sidebar / overview / settings on demand and closes
   them after use, so no WebView2 process runs while nothing web is on screen.
