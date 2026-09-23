-- Logical Lunge terminali: end-4 dots-hyprland kitty.conf'un WezTerm karşılığı + fish (MSYS2).
-- Eski ayar: ~/.wezterm.lua.bak-20260923
local wezterm = require 'wezterm'
local act = wezterm.action
local config = wezterm.config_builder()

-- ---- Performans (eski sürümdeki taşırken donma / tane tane yapıştırma için) ----
-- Acrylic + şeffaflık Windows 10'da DWM bulanıklığı yüzünden pencere taşırken donduruyordu: opak, GPU'da çiz.
config.front_end = 'WebGpu'
config.webgpu_power_preference = 'HighPerformance'
config.max_fps = 144
config.animation_fps = 60
config.window_background_opacity = 1.0
config.win32_system_backdrop = 'Disable'
config.enable_scroll_bar = false
config.scrollback_lines = 10000

-- ---- kitty.conf: Font ----
config.font = wezterm.font_with_fallback({
  'JetBrainsMono Nerd Font',
  'JetBrains Mono',
  'Cascadia Code',
})
config.font_size = 11.0

-- ---- kitty.conf: Cursor (beam) ----
config.default_cursor_style = 'BlinkingBar'
config.cursor_blink_rate = 500
config.cursor_blink_ease_in = 'Constant'
config.cursor_blink_ease_out = 'Constant'
config.cursor_thickness = '1.5px' -- foot.ini beam-thickness=1.5

-- ---- kitty.conf: Padding (window_margin_width 21.75) ----
config.window_padding = { left = '21.75pt', right = '21.75pt', top = '21.75pt', bottom = '21.75pt' }

-- ---- kitty.conf: No stupid close confirmation ----
config.window_close_confirmation = 'NeverPrompt'
config.skip_close_confirmation_for_processes_named = { 'fish', 'bash', 'powershell.exe', 'pwsh.exe', 'cmd.exe' }

-- Pencere çerçevesi yok (GlazeWM döşer, köşeleri helper yuvarlar), sekme çubuğu yalnızca birden fazla sekmede
config.window_decorations = 'RESIZE'
config.hide_tab_bar_if_only_one_tab = true
config.use_fancy_tab_bar = false
config.tab_bar_at_bottom = true
config.adjust_window_size_when_changing_font_size = false

-- ---- kitty.conf: Use fish shell (MSYS2) ----
config.default_prog = { 'C:\\msys64\\usr\\bin\\fish.exe', '-l' }
config.set_environment_variables = {
  MSYS2_PATH_TYPE = 'inherit', -- Windows PATH'i (git, python, scoop...) fish'te de olsun
  MSYSTEM = 'UCRT64',
  CHERE_INVOKING = '1',        -- açıldığı klasörde kalsın
}
config.default_cwd = wezterm.home_dir
config.launch_menu = {
  { label = 'fish', args = { 'C:\\msys64\\usr\\bin\\fish.exe', '-l' } },
  { label = 'PowerShell', args = { 'powershell.exe', '-NoLogo' } },
  { label = 'cmd', args = { 'cmd.exe' } },
}

-- ---- Renkler: ii kitty-theme.conf (duvar kağıdından Material You, starship indeksleri dahil) ----
-- tools\termcolors\wezterm-colors.py ~/.config/wezterm/ll-colors.lua'yı üretir; dosya değişince WezTerm anında yeniler.
local colors_file = wezterm.home_dir .. '/.config/wezterm/ll-colors.lua'
local ok, generated = pcall(dofile, colors_file)
if ok and type(generated) == 'table' then
  config.colors = generated
  wezterm.add_to_config_reload_watch_list(colors_file)
else
  config.colors = { foreground = '#e6e0e9', background = '#141218', cursor_bg = '#e6e0e9', cursor_border = '#e6e0e9' }
end
-- ---- kitty.conf: kısayollar ----
config.keys = {
  -- map ctrl+c copy_or_interrupt: seçim varsa kopyala, yoksa Ctrl+C gönder
  {
    key = 'c', mods = 'CTRL',
    action = wezterm.action_callback(function(window, pane)
      local sel = window:get_selection_text_for_pane(pane)
      if sel and sel ~= '' then
        window:perform_action(act.CopyTo 'Clipboard', pane)
        window:perform_action(act.ClearSelection, pane)
      else
        window:perform_action(act.SendKey { key = 'c', mods = 'CTRL' }, pane)
      end
    end),
  },
  { key = 'v', mods = 'CTRL', action = act.PasteFrom 'Clipboard' },
  { key = 'V', mods = 'CTRL|SHIFT', action = act.PasteFrom 'Clipboard' },
  -- map ctrl+f / kitty_mod+f: ara
  { key = 'f', mods = 'CTRL', action = act.Search { CaseInSensitiveString = '' } },
  { key = 'F', mods = 'CTRL|SHIFT', action = act.Search { CaseInSensitiveString = '' } },
  -- Scroll
  { key = 'PageUp', action = act.ScrollByPage(-1) },
  { key = 'PageDown', action = act.ScrollByPage(1) },
  -- Zoom
  { key = '+', mods = 'CTRL', action = act.IncreaseFontSize },
  { key = '=', mods = 'CTRL', action = act.IncreaseFontSize },
  { key = '-', mods = 'CTRL', action = act.DecreaseFontSize },
  { key = '_', mods = 'CTRL', action = act.DecreaseFontSize },
  { key = '0', mods = 'CTRL', action = act.ResetFontSize },
  -- Sekme / kabuk menüsü
  { key = 'T', mods = 'CTRL|SHIFT', action = act.SpawnTab 'CurrentPaneDomain' },
  { key = 'L', mods = 'CTRL|SHIFT', action = act.ShowLauncherArgs { flags = 'LAUNCH_MENU_ITEMS' } },
}

return config
