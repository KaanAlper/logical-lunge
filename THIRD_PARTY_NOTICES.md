# Third-party notices

Logical Lunge is licensed under the GNU General Public License v3.0 (see [LICENSE](LICENSE)). Parts of it are derived
from, or follow the behaviour of, the open-source projects below. Their notices are kept here as their licenses
require.

## Code in this repository derived from other projects

### GlazeWM: `tiling/` (`lunge-tiling`)

- Source: https://github.com/glzr-io/glazewm
- Copyright: glzr-io and GlazeWM contributors
- License: GNU General Public License v3.0 (full text: [tiling/LICENSE](tiling/LICENSE))
- Changes: renamed to `lunge-tiling`; Hyprland-style dwindle layout and moves; window borders drawn in-process;
  tray icon, update checks and packaging removed; Logical Lunge's config, log and install locations.

### Zebar: `shell/` (`lunge-shell`)

- Source: https://github.com/glzr-io/zebar
- Copyright: glzr-io and Zebar contributors
- License: GNU General Public License v3.0 (full text: [shell/LICENSE](shell/LICENSE))
- Changes: renamed to `lunge-shell`; settings window, marketplace, tray icon, client API and packaging removed;
  widgets load only from the app's own `ui` folder; Logical Lunge's data and log locations; the browser context
  menu, developer tools and browser shortcuts are disabled in widgets.

### Zebar client library: `ui/lib/shell-client.js`

- Source: the `zebar` npm package 3.3.1 (https://github.com/glzr-io/zebar, `packages/client-api`)
- Copyright: glzr-io and Zebar contributors
- License: GNU General Public License v3.0
- Changes: only the API and providers the widgets use; no console logging of provider output; the window manager
  provider is renamed `tiling`, talks to `ui/lib/tiling-client.js` (a new IPC client) and coalesces its queries.

### Libraries bundled into the widgets at build time (`ui/package.json`)

| Project | License |
|---|---|
| [React](https://github.com/facebook/react) / React DOM 18.3.1 | MIT, Copyright (c) Meta Platforms, Inc. and affiliates |
| [Tauri JS API](https://github.com/tauri-apps/tauri) 2.10.1 | MIT or Apache-2.0, Copyright (c) Tauri Programme within The Commons Conservancy |
| [Zod](https://github.com/colinhacks/zod) 3.24.2 | MIT, Copyright (c) Colin McDonnell |
| [Luxon](https://github.com/moment/luxon) 3.4.4 | MIT, Copyright (c) JS Foundation and other contributors |

### Fonts: `ui/fonts/`

| Font | License |
|---|---|
| [Google Sans Flex](https://fonts.google.com/specimen/Google+Sans+Flex) | SIL Open Font License 1.1 |
| [Rubik](https://github.com/googlefonts/rubik) | SIL Open Font License 1.1 |
| [Material Symbols Rounded](https://github.com/google/material-design-icons) | Apache-2.0 |

### tacky-borders: `tiling/packages/wm-borders`

- Source: https://github.com/lukeyou05/tacky-borders
- License: MIT (full text: [tiling/packages/wm-borders/LICENSE-tacky-borders](tiling/packages/wm-borders/LICENSE-tacky-borders))

```
MIT License

Copyright (c) 2024 luke-you

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### end-4 / dots-hyprland (illogical-impulse): `ui/`, `config/`, `tools/termcolors/`

- Source: https://github.com/end-4/dots-hyprland
- License: GNU General Public License v3.0
- Used for: the whole visual design and layouts of the widgets, animations, the terminal / fish / starship
  configuration and the Material You color generator.

### Hyprland: behaviour of the tiling layout and animations

- Source: https://github.com/hyprwm/Hyprland
- The dwindle layout, window moves and animation timing of `lunge-tiling` and `lunge.exe` follow Hyprland's
  behaviour.

```
BSD 3-Clause License

Copyright (c) 2022-2026, vaxerski
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Downloaded by the installer (not part of this repository)

| Project | Used for | License |
|---|---|---|
| [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) | CPU / GPU temperature (`tools\temps`) | MPL-2.0 |
| [PawnIO](https://github.com/namazso/PawnIO.Setup) | Driver for CPU temperature (optional) | GPL-2.0 |
| [WezTerm](https://github.com/wezterm/wezterm) | Terminal (optional) | MIT |
| [fish](https://fishshell.com) via [MSYS2](https://www.msys2.org) | Shell (optional) | GPL-2.0 / BSD-3-Clause |
| [starship](https://starship.rs) · [eza](https://github.com/eza-community/eza) · [fzf](https://github.com/junegunn/fzf) | Prompt, `ls`, picker (optional) | ISC · EUPL-1.2 · MIT |
| [Nerd Fonts](https://github.com/ryanoasis/nerd-fonts) (JetBrains Mono) | Terminal font (optional) | OFL-1.1 |
| [NirSoft ControlMyMonitor](https://www.nirsoft.net/utils/control_my_monitor.html) | DDC/CI brightness | Freeware |
| [gum](https://github.com/charmbracelet/gum) | Installer prompts (downloaded to %TEMP%, removed afterwards) | MIT |

The packaged tools (`lunge-songrec`, `lunge-termcolors`) bundle [shazamio](https://github.com/shazamio/ShazamIO) (MIT)
and [materialyoucolor](https://github.com/T-Dynamos/materialyoucolor-python) (MIT) with their dependencies.
