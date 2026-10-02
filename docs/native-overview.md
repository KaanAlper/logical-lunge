# Native Super menu

The Super menu (ii's overview: search on top, workspace grid below) is the most used panel, so it should open without a
frame of delay and cost nothing while closed. In the native edition it is drawn with Direct2D / DirectComposition on the
native bar's UI thread, sharing its device, fonts, icons, theme and app list; it is the only Super menu there (the web
page `ui/overview.html` is gone from this edition and lives on in the web edition).

## Responsibility map (the web edition's ui/overview.html → native)

| # | Web menu | Native menu |
|---|---|---|
| 1 | Found by the core through its title `lunge-overview`: Super shows it (Win32 show + foreground, transparent until the page read its mode), Super again / Super+V / window and workspace shortcuts hide it and give focus back | same title and class-independent contract; the window paints inside `WM_SHOWWINDOW`, so the core skips the transparent reveal for it |
| 2 | Open mode from the core (`/overview-mode`, `overview-mode.txt`): `;` = clipboard (Super+V), else plain search | same flag, read on show |
| 3 | `ll:overview-toggle` (bar search button, right click on workspaces, the Dock's search button): open, or close if open and focused | the bar's own buttons call it directly; a widget's event (the Dock) is handed to the bar by the shell (`native_bar::widget_overview_toggle`) |
| 4 | Closes on Esc (first Esc clears the query), click on the backdrop, focus loss, `ll:bar-click`, running a result (unless it stays) | same |
| 5 | Search bar: prefix shape (MaterialShape per prefix) + icon, width 210 → 360 / 560 with text (300 ms elementMove), placeholder "Ara, hesapla veya çalıştır" | same; own text editing (caret, selection, Ctrl+A / C / V / X / Backspace, Home / End, word jumps; IME text arrives as WM_CHAR) |
| 6 | Google Lens button (`lunge.exe --lens` after closing), song recognition button (`lunge.exe --songrec ...`, running animation, result toast with Spotify / YouTube actions) | same commands |
| 7 | Results: clipboard `;`, actions `/`, command `$`, web `?`, calculator `=` (and automatic), apps `>` (and default), run + web search | `native_bar/search.rs` (done): same results, checked against the web functions (`tools/dev/search-parity.mjs`: every input of up to three symbols built from the web's own name tables plus 5 000 longer ones, 137 441 in all, and 85 800 app / query scores from queries derived from the app names; no difference). The comparison found three web bugs, fixed on both sides: `max--` overwrote the page's `Math.max` (the menu then failed to draw), `2,pi` gave 3.14 (JavaScript's comma operator), `sqrt tau` worked but `log tau` did not |
| 8 | Keys: ↑ ↓ select, Enter run, Tab completes, Delete removes a clipboard entry, hover selects, click runs, selected row scrolled into view | same |
| 9 | Running: apps → `lunge.exe --focus-under-cursor` then `explorer <path>`; commands / URLs → `lunge.exe --ps scripts\run.ps1 <mode> <text>` and a toast with the result; clipboard → `--clip-set` / `--clip-del`; actions (theme, lock, sleep, logout, restart, shutdown, WM reload, rebuild app list) | same commands; toasts through the `ll:toast` event |
| 10 | Workspace grid (no query): 2 × 5 pages of 10 around the focused workspace, each scaled 0.18 of its own monitor, windows as boxes with the app icon (apps.json, else `/winicon`), focused / active / drop highlights, minimized left out | same data from the WM connection the bar already has (plus window geometry) |
| 11 | Grid actions: click a workspace → close + slide (`lunge.exe --slide n`, fallback WM focus); click a window → WM focus + close; drag a window onto a workspace → WM move | same |
| 12 | Enter animation (260 ms emphasized decelerate: opacity, -14 px, scale 0.98) | DirectComposition animation (`overview_enter`; none when animations are off) |
| 13 | Theme, i18n (all texts), 12 / 24 h clock in clipboard times | shared with the native bar |

## No web menu

There is no switch and no fallback to a WebView2 menu in this edition: a failing bar (and its menu) is built again by
the bar's guard. Still to do on the native side: mouse selection in the text box (drag, double-click), undo, IME
composition placement, the I-beam and hand cursors, right-to-left layout for Arabic, and the width / shape morphs.

## Phases

1. Search logic and tests (done).
2. Window, core contract, search bar with text editing, result list, running results, keys.
3. Workspace grid with icons, click and drag.
4. Animations (entrance done), song recognition / Lens buttons, measurements. The web menu is removed from this edition.
