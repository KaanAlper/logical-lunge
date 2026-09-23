<div align="center">

# Logical Lunge

**end-4's illogical-impulse Hyprland desktop, rebuilt for Windows — with one command.**

end-4 illogical-impulse Hyprland masaüstü, Windows için — tek komutla.

[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%C2%B7%2011-0078D4?style=flat-square&logo=windows&logoColor=white)](#compatibility)
[![WM](https://img.shields.io/badge/WM-GlazeWM-8B5CF6?style=flat-square)](https://github.com/glzr-io/glazewm)
[![Shell](https://img.shields.io/badge/shell-Zebar-D0BCFF?style=flat-square)](https://github.com/glzr-io/zebar)
[![Design](https://img.shields.io/badge/design-illogical--impulse-4F378B?style=flat-square)](https://github.com/end-4/dots-hyprland)
[![Languages](https://img.shields.io/badge/UI-13%20languages-2A59FF?style=flat-square)](#languages)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue?style=flat-square)](LICENSE)

<!-- Screenshots are added after the first clean-machine test run (docs/screenshots/). -->

**[English](#english) · [Türkçe](#türkçe)**

</div>

> [!NOTE]
> Forked from / built on **[GlazeWM](https://github.com/glzr-io/glazewm)**, **[Zebar](https://github.com/glzr-io/zebar)** and **[end-4's illogical-impulse (dots-hyprland)](https://github.com/end-4/dots-hyprland)**. All three are GPL-3.0; so is this project. See [Credits](#credits--teşekkürler).

## ⚡ Quick install · Hızlı kurulum

Open **PowerShell** and paste · **PowerShell**'i aç ve yapıştır:

```powershell
irm https://raw.githubusercontent.com/KaanAlper/logical-lunge/main/install.ps1 | iex
```

One admin prompt; everything else downloads and installs silently. · Tek yönetici onayı; gerisi kendiliğinden, sessizce iner ve kurulur.

---

## English

### What this is

A complete Windows desktop that looks and behaves like the **illogical-impulse** (`ii`) Quickshell setup on Hyprland — the bar, the right sidebar, the Super-key overview, the animations and the keybinds — not a theme, a working shell:

- **Tiling** with Hyprland's dwindle layout ported into our GlazeWM fork: a new window splits the window under the mouse along its longer side and opens on the half the mouse is over (`force_split = 0`); `movewindow` splits the neighbouring window the same way. New windows never flash at the screen centre — they appear only once in place.
- **ii bar**: workspaces with app icons, resources (RAM / swap / CPU / **CPU & GPU temperature**), media with album art and seeking, tray with drag-to-pin, clock, battery, scroll-to-change brightness (left edge) and volume (right edge) with an OSD.
- **ii right sidebar**: Android-style quick toggles (Wi-Fi, Ethernet, Bluetooth, keep-awake, mic, audio, night light with schedule + intensity, dark mode, screenshot, on-screen keyboard, do-not-disturb) with slide-down cards, notifications, calendar with month/year picker, to-do and pomodoro timer.
- **Shortcuts editor** and **wallpaper picker** built into the sidebar (per-monitor or one image spanning all monitors — Superpaper-style *superscreen*).
- **Super overview**: fuzzy app search (localized names + icons), calculator (`sqrt(9)`, `5!`, `2^10`, `50%`), `/actions`, `$shell`, `?web`, **Google Lens** region search, **music recognition** (Shazam), workspace previews.
- **Screenshot tool** (Print): select a region, then annotate — pen, circle, rectangle, colors — copy or save as.
- **Ctrl+Print**: the whole monitor under the mouse, copied to the clipboard and saved to `Pictures\Screenshots` without asking.
- **Clipboard history** (Super+V, ii's cliphist): text and images, searchable, opens in the Super search box with the `;` prefix.
- **Alt+Tab switcher**: live window previews across all workspaces, most recently used first, drawn by the helper so it opens instantly under load.
- **Session screen** (power button in the sidebar): dimmed screen with lock / sleep / sign out / restart / **UEFI-BIOS** / shut down.
- **Updates**: the sidebar's update button checks GitHub releases, shows a card with a progress bar, then *Install now* / *Later*. A downloaded update is remembered; installing asks for permission once and restarts the desktop.
- **Animations**: smooth workspace slides, window open / close / move animations (Hyprland `emphasizedDecel` curves), popups that slide in and out.
- **Terminal**: WezTerm configured exactly like ii's kitty (JetBrains Mono Nerd Font, beam cursor, ii's wallpaper-generated Material You colors) running **fish** with **starship**.
- **Boot straight into the desktop**: a wallpaper splash covers Windows until the shell is ready; the Windows taskbar and Start menu never show (Super is owned by the shell *only while it runs*).

### Why this exists

Recreating ii on Windows means fixing things nobody warns you about:

1. **Lone Super opens Start**, and Ctrl+Super spam leaks it anyway → the helper owns the Win key completely and re-injects it only for combos the shell doesn't handle (Win+L still works; Win+V opens our clipboard history; Win+D / Win+M are blocked because they break the tiling layout).
2. **GlazeWM's focus/move jumps to the other monitor** when there is no window in that direction → focus and move stay inside the workspace; moving at an edge re-splits the layout like Hyprland.
3. **Workspace switches lag** by 100–200 ms with many windows → the slide starts from DWM thumbnails before the WM finishes, at high process priority.
4. **Apps reset rounded window regions**, dialogs flicker under focus-follows-mouse, Windows error boxes pop up → all handled (rounded corners, real-mouse-movement focus, error dialogs turned into ii-style toasts).
5. **Everything had to survive a reboot and a different PC** → one installer, one UAC prompt, every Windows setting backed up and restored on uninstall.

### Install

**One command** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/KaanAlper/logical-lunge/main/install.ps1 | iex
```

Windows asks for permission **once**. The installer silently downloads whatever is missing — the WebView2 and Visual C++ runtimes, and pinned, tested versions of WezTerm, the Nerd Font, fish (MSYS2), starship, eza, tacky-borders and LibreHardwareMonitor, installs the shell, backs up and applies the Windows settings, and starts the desktop.

| Option (set before running) | Effect |
|---|---|
| `$env:LL_NO_TERMINAL = 1` | Skip WezTerm + fish + fonts |
| `$env:LL_NO_SENSORS = 1` | Skip the PawnIO driver (no CPU temperature) |

**Uninstall:** *Settings → Apps → Logical Lunge → Uninstall*, or run `~\.glzr\logical-lunge\uninstall.ps1`. Every Windows setting goes back to what it was; your own configs (GlazeWM / WezTerm / fish / starship, shortcuts, night light, downloaded wallpapers) are kept.

### Default keybinds

| Keys | Action |
|---|---|
| `Super` | Overview / search |
| `Super + ←↑→↓` | Focus inside the workspace (the mouse follows) |
| `Super + Shift + ←↑→↓` | Move window / re-split the layout |
| `Super + Ctrl + ←/→` | Previous / next workspace (slide) |
| `Super + Ctrl + Shift + ←/→` | Move window to previous / next workspace |
| `Super + 1…0` | Workspace 1–10 |
| `Super + Enter` / `Super + T` | Terminal (WezTerm + fish) |
| `Super + W / E / C / X` | Browser / files / code editor / text editor |
| `Super + F` | Fullscreen |
| `Alt + F4` | Close window |
| `Print` | Screenshot + annotate |
| `Ctrl + Print` | Whole monitor to clipboard + `Pictures\Screenshots` |
| `Super + V` | Clipboard history (press again to close) |
| `Alt + Tab` | Window switcher (Shift reverses, arrows / Enter / Esc / mouse work) |

All of them can be changed in **sidebar → ⌨ Shortcuts** (read-only until you unlock it; "reset to defaults" included).

### Compatibility

| | Status |
|---|---|
| Windows 10 22H2 (19045) | Developed and tested here |
| Windows 11 | Supported by every component (GlazeWM, Zebar, WebView2, IDesktopWallpaper); Windows-11-only snap layouts are switched off by the installer. First clean-machine test pending. |
| ARM64 | Not yet (helpers are built x64) |

### Languages

The UI follows the system language: English, Deutsch, Français, Español, Italiano, Português, Русский, Українська, Polski, 日本語, 中文, 한국어, العربية (right-to-left) and Türkçe. Dates and month/day names use the system locale.

### How it works

```
GlazeWM (tiling, IPC)  ←→  ll-helper (C#: keys, slides, animations, dwindle, focus, rounding, toasts,
                                       splash, snip, Lens, wallpapers, gamma/night light, shortcuts)
Zebar  (WebView2 widgets: bar, sidebar, overview, toast, OSK — React)
```

**Roadmap — one program.** The next step merges GlazeWM + Zebar + ll-helper into a single `Logical Lunge` executable built from forks, dropping what the shell doesn't use (Zebar's widget manager / marketplace window and tray icon, runtime config editing, GlazeWM's tray icon) — see [docs/ROADMAP.md](docs/ROADMAP.md).

### Build from source

```powershell
git clone https://github.com/KaanAlper/logical-lunge; cd logical-lunge
.\build.ps1                  # -> dist\LogicalLunge-<version>.zip
$env:LL_SOURCE = "$PWD\dist\LogicalLunge-$(Get-Content VERSION)"; .\install.ps1
```

Needs the Windows 10 SDK (for `media-art.exe`), Python 3.12 (packaged helpers are built with PyInstaller — the target PC needs no Python) and Node.js (translations).

The GlazeWM and Zebar forks (branch `logical-lunge`: Hyprland dwindle layout, no tray icons, unused providers removed) are built too when they are checked out next to this repo:

```powershell
git clone -b logical-lunge https://github.com/KaanAlper/glazewm ..\logical-lunge-forks\glazewm
git clone -b logical-lunge https://github.com/KaanAlper/zebar   ..\logical-lunge-forks\zebar
```

This needs Rust (nightly for Zebar) and pnpm.

---

## Türkçe

### Bu nedir

Hyprland üzerindeki **illogical-impulse** (`ii`) Quickshell kurulumunun Windows karşılığı. Bar, sağ panel, Super menüsü, animasyonlar ve kısayollar birebir; tema değil, çalışan bir masaüstü kabuğu:

- **Döşeme**: Hyprland `force_split = 0` gibi farenin altındaki yarıya açılan dwindle ("altın oran") bölmeleri.
- **ii bar**: uygulama simgeli workspace'ler, kaynaklar (RAM / swap / CPU / **CPU & GPU sıcaklığı**), kapaklı ve sarılabilir medya, sürükleyerek sabitlenen tepsi, saat, pil, sol kenarda kaydırınca parlaklık, sağ kenarda ses (OSD'li).
- **ii sağ panel**: Android tarzı hızlı ayarlar (Wi-Fi, Ethernet, Bluetooth, uyanık tut, mikrofon, ses, zamanlamalı ve yoğunluk ayarlı gece ışığı, karanlık mod, ekran alıntısı, ekran klavyesi, sessiz) ve alta kayan kartlar; bildirimler; ay/yıl seçicili takvim; yapılacaklar; zamanlayıcı.
- Panelde **kısayol düzenleyici** ve **duvar kağıdı seçici** (monitör başına ya da tüm monitörlere yayılan tek resim, Superpaper'daki gibi).
- **Super menüsü**: bulanık uygulama arama, hesap makinesi, `/eylemler`, `$komut`, `?web`, **Google Lens**, **müzik tanıma** (Shazam), workspace önizlemeleri.
- **Ekran alıntısı** (Print): alan seç, üzerine kalem / çember / dikdörtgen / renkle çiz, kopyala ya da kaydet.
- **Animasyonlar**: kaygan workspace geçişleri, pencere açma/kapama/taşıma animasyonları, kayarak açılıp kapanan popup'lar.
- **Terminal**: ii'nin kitty ayarlarıyla birebir WezTerm (JetBrains Mono Nerd Font, çizgi imleç, duvar kağıdından üretilen Material You renkleri) içinde **fish** + **starship**.
- **Doğrudan masaüstüne açılış**: kabuk hazır olana kadar duvar kağıdı perdesi; Windows görev çubuğu ve Başlat menüsü hiç görünmez (Super tuşu yalnızca program açıkken kabuğundur).

### Kurulum

```powershell
irm https://raw.githubusercontent.com/KaanAlper/logical-lunge/main/install.ps1 | iex
```

Windows **bir kez** izin ister, gerisini yükleyici yapar. **Kaldırma:** *Ayarlar → Uygulamalar → Logical Lunge → Kaldır*. Değiştirilen tüm Windows ayarları eski haline döner; kendi ayar dosyaların silinmez.

### Uyumluluk

Windows 10 22H2 üzerinde geliştirildi ve denendi. Windows 11'i tüm bileşenler destekliyor; yükleyici Windows 11'e özel snap düzenlerini kapatıyor. Temiz bir Windows 11'de ilk test henüz yapılmadı.

---

## Credits / Teşekkürler

| Project | Used for | License |
|---|---|---|
| [end-4/dots-hyprland](https://github.com/end-4/dots-hyprland) (illogical-impulse) | The whole design, layouts, animations, terminal/kitty/fish/starship config, color generator | GPL-3.0 |
| [glzr-io/glazewm](https://github.com/glzr-io/glazewm) | Tiling window manager | GPL-3.0 |
| [glzr-io/zebar](https://github.com/glzr-io/zebar) | Widget runtime (WebView2) | GPL-3.0 |
| [lukeyou05/tacky-borders](https://github.com/lukeyou05/tacky-borders) | Rounded focus borders | MIT |
| [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) + [PawnIO](https://github.com/namazso/PawnIO.Setup) | CPU / GPU temperature | MPL-2.0 / GPL-2.0 |
| [WezTerm](https://github.com/wezterm/wezterm) · [fish](https://fishshell.com) · [starship](https://starship.rs) · [eza](https://github.com/eza-community/eza) · [MSYS2](https://www.msys2.org) | Terminal | MIT · GPL-2.0 · ISC · EUPL-1.2 · BSD-3-Clause |
| [Nerd Fonts](https://github.com/ryanoasis/nerd-fonts) (JetBrains Mono) | Terminal font | OFL-1.1 |
| [NirSoft ControlMyMonitor](https://www.nirsoft.net/utils/control_my_monitor.html) | DDC/CI brightness | Freeware |
| [shazamio](https://github.com/shazamio/ShazamIO) · [materialyoucolor](https://github.com/T-Dynamos/materialyoucolor-python) | Music recognition · Material You colors | MIT · MIT |
| [Wallhaven](https://wallhaven.cc) | Wallpaper suggestions (safe-for-work only) | per image |

## License

GPL-3.0 — see [LICENSE](LICENSE).
