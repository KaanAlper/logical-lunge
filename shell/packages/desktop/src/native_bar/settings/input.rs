//! The settings window's mouse and keyboard: what a click, a drag, the
//! wheel or a key does to its controls.

use serde_json::{json, Value};
use windows::Win32::{
  Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
  Graphics::Gdi::ValidateRect,
  UI::{
    Input::KeyboardAndMouse::{
      GetKeyState, ReleaseCapture, SetCapture, VK_BACK, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR,
      VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
    },
    WindowsAndMessaging::*,
  },
};

use super::{
  core_run, explorer, night, pages, set_pref, Act, Cursor, Event, Fld, Hit, Key, Sel, Sl, Sw, PAGES, TIMER_SETTINGS_COMMIT, WHEEL_STEP,
};
use crate::native_bar::{
  core_api,
  menu::{Item, MenuFocus},
  Msg, Ui,
};

impl Ui {
  /// A message for the settings window (None: not it).
  pub(in crate::native_bar) fn settings_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    let s = self.settings.as_mut()?;
    if s.hwnd != hwnd {
      return None;
    }
    if msg == WM_PAINT {
      unsafe {
        let _ = ValidateRect(hwnd, None);
      }
      return Some(Some(LRESULT(0)));
    }
    if s.closing {
      return Some(None);
    }
    let scale = s.scale;
    let pos = |lp: LPARAM| {
      let x = (lp.0 & 0xFFFF) as i16 as f32 / scale;
      let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / scale;
      (x, y)
    };
    let r = match msg {
      WM_MOUSEMOVE => {
        let (x, y) = pos(lp);
        self.settings_mouse_move(x, y);
        Some(LRESULT(0))
      }
      WM_MOUSELEAVE_MSG => {
        if let Some(s) = self.settings.as_mut() {
          s.hover = None;
          s.ws.hover_pct = None;
        }
        self.settings_paint();
        Some(LRESULT(0))
      }
      WM_LBUTTONDOWN => {
        let (x, y) = pos(lp);
        self.settings_down(x, y);
        Some(LRESULT(0))
      }
      WM_LBUTTONUP => {
        let (x, y) = pos(lp);
        self.settings_up(x, y);
        Some(LRESULT(0))
      }
      WM_MOUSEWHEEL => {
        let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as f32;
        self.settings_wheel(delta);
        Some(LRESULT(0))
      }
      WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
        let cursor = self.settings.as_ref().map_or(Cursor::Arrow, |s| {
          if s.drag.is_some() {
            return Cursor::Resize;
          }
          s.hover.and_then(|h| s.region(h)).map_or(Cursor::Arrow, |r| r.cursor)
        });
        let id = match cursor {
          Cursor::Arrow => IDC_ARROW,
          Cursor::Hand => IDC_HAND,
          Cursor::Text => IDC_IBEAM,
          Cursor::Resize => IDC_SIZEWE,
        };
        unsafe {
          if let Ok(c) = LoadCursorW(None, id) {
            SetCursor(c);
          }
        }
        Some(LRESULT(1))
      }
      WM_KEYDOWN | WM_SYSKEYDOWN => {
        let handled = self.settings_key(wp.0 as u16);
        if !handled && msg == WM_SYSKEYDOWN {
          None // Alt+F4 comes back as WM_CLOSE
        } else {
          Some(LRESULT(0))
        }
      }
      WM_CHAR => {
        self.settings_char(wp.0 as u16);
        Some(LRESULT(0))
      }
      WM_CAPTURECHANGED => {
        if let Some(s) = self.settings.as_mut() {
          s.drag = None;
          s.pressed = None;
        }
        None
      }
      WM_CLOSE => {
        self.settings_close();
        Some(LRESULT(0))
      }
      _ => None,
    };
    Some(r)
  }

  fn settings_mouse_move(&mut self, x: f32, y: f32) {
    let Some(s) = self.settings.as_mut() else { return };
    crate::native_bar::track_leave(s.hwnd);
    if let Some(drag) = s.drag {
      self.settings_drag_to(drag, x);
      return;
    }
    let mut changed = false;
    let hit = s.hit_at(x, y).map(|r| r.hit);
    if hit != s.hover {
      s.hover = hit;
      changed = true;
    }
    // the number line's lens
    let lens = match s.region(Hit::WsLine) {
      Some(r) if r.r.contains(x, y) || (hit == Some(Hit::WsLine)) => Some(((x - r.r.x) / r.r.w).clamp(0.0, 1.0)),
      Some(r) if matches!(hit, Some(Hit::WsDivider(_))) => Some(((x - r.r.x) / r.r.w).clamp(0.0, 1.0)),
      _ => None,
    };
    if lens != s.ws.hover_pct {
      s.ws.hover_pct = lens;
      changed = true;
    }
    if changed {
      self.settings_paint();
    }
  }

  fn settings_drag_to(&mut self, drag: Hit, x: f32) {
    let Some(s) = self.settings.as_mut() else { return };
    match drag {
      Hit::Slider(sl) => {
        let Some(r) = s.region(drag).map(|r| r.r) else { return };
        let (min, max, step) = pages::slider_range(sl);
        let k = ((x - r.x) / r.w).clamp(0.0, 1.0);
        let v = ((min as f32 + k * (max - min) as f32) / step as f32).round() as i32 * step;
        self.settings_slider_set(sl, v.clamp(min, max));
      }
      Hit::WsDivider(i) => {
        let Some(line) = s.region(Hit::WsLine).map(|r| r.r) else { return };
        let k = ((x - line.x) / line.w).clamp(0.0, 1.0);
        s.ws.hover_pct = Some(k);
        s.ws.drag_divider(i, k);
        self.settings_paint();
      }
      _ => {}
    }
  }

  fn settings_slider_set(&mut self, sl: Sl, v: i32) {
    let Some(s) = self.settings.as_mut() else { return };
    let cur = pages::slider_value(s, sl);
    if cur == v {
      return;
    }
    match sl {
      Sl::ToastInfo => s.s["toastInfo"] = json!(v),
      Sl::ToastError => s.s["toastError"] = json!(v),
      Sl::NightLevel => s.night_level = v,
    }
    s.pending.retain(|(k, _)| *k != sl);
    s.pending.push((sl, v));
    // settings.html: 150 ms after the last move (60 ms for the night light)
    let wait = if sl == Sl::NightLevel { 60 } else { 150 };
    unsafe { SetTimer(self.msg_hwnd, TIMER_SETTINGS_COMMIT, wait, None) };
    self.settings_paint();
  }

  fn settings_down(&mut self, x: f32, y: f32) {
    let Some(s) = self.settings.as_mut() else { return };
    s.focus_ring = false;
    // our own click closes the shared menu (it keeps our focus)
    if self.menu_is_open() {
      self.menu_close();
      return;
    }
    let Some(s) = self.settings.as_mut() else { return };
    let hit = s.hit_at(x, y).map(|r| (r.hit, r.focus));
    let Some((hit, focusable)) = hit else {
      s.set_focus(None);
      self.settings_paint();
      return;
    };
    if focusable {
      s.set_focus(Some(hit));
    }
    match hit {
      Hit::Slider(_) | Hit::WsDivider(_) => {
        s.drag = Some(hit);
        unsafe {
          SetCapture(s.hwnd);
        }
        self.settings_drag_to(hit, x);
      }
      Hit::Field(_) => self.settings_paint(),
      _ => {
        s.pressed = Some(hit);
        unsafe {
          SetCapture(s.hwnd);
        }
        self.settings_paint();
      }
    }
  }

  fn settings_up(&mut self, x: f32, y: f32) {
    let Some(s) = self.settings.as_mut() else { return };
    let pressed = s.pressed.take();
    let dragged = s.drag.take();
    unsafe {
      let _ = ReleaseCapture();
    }
    if dragged.is_some() {
      self.settings_paint();
      return;
    }
    let Some(pressed) = pressed else { return };
    let here = s.hit_at(x, y).map(|r| r.hit);
    if here == Some(pressed) {
      self.settings_press(pressed);
    } else {
      self.settings_paint();
    }
  }

  fn settings_wheel(&mut self, delta: f32) {
    if self.menu_is_open() {
      return;
    }
    let Some(s) = self.settings.as_mut() else { return };
    s.scroll_by(-delta / 120.0 * WHEEL_STEP);
    s.hover = None;
    self.settings_paint();
  }

  /// A click (or Enter / Space) on a control.
  fn settings_press(&mut self, hit: Hit) {
    let Some(s) = self.settings.as_mut() else { return };
    match hit {
      Hit::Nav(i) => {
        if s.page != i {
          s.page = i;
          self.settings_page_opened();
        }
      }
      Hit::Act(a) => return self.settings_act(a),
      Hit::Seg(key, i) => return self.settings_seg(key, i),
      Hit::Switch(sw) => {
        let on = match sw {
          Sw::Animations => s.s["animations"].as_bool() != Some(false),
          Sw::Gestures => s.s["gestures"].as_bool() != Some(false),
          Sw::WinToasts => s.s["winToasts"].as_bool() != Some(false),
          Sw::Takeover => s.s["takeover"].as_bool() != Some(false),
          Sw::Night => s.night.as_ref().is_some_and(|n| n["on"].as_bool() == Some(true)),
        };
        match sw {
          Sw::Animations => set_pref("animations", json!(!on), false),
          Sw::Gestures => set_pref("gestures", json!(!on), false),
          Sw::WinToasts => set_pref("winToasts", json!(!on), false),
          Sw::Takeover => set_pref("takeover", json!(!on), false),
          Sw::Night => night(&["--nightlight", "toggle"]),
        }
      }
      Hit::Step(f, d) => s.ws.step(f, d),
      Hit::Select(sel) => return self.settings_dropdown(sel),
      Hit::Swatch(i) => {
        let c = pages::COLORS[i].1;
        return self.settings_color(c.to_string());
      }
      Hit::WsCustom(i) => s.ws.cycle_custom(i),
      Hit::Field(_) | Hit::Slider(_) | Hit::WsLine | Hit::WsDivider(_) => {}
    }
    self.settings_paint();
  }

  fn settings_seg(&mut self, key: Key, i: usize) {
    let Some(s) = self.settings.as_mut() else { return };
    match key {
      Key::Theme => {
        let light = i == 1;
        core_api::post_async(format!("/pref?k=theme&v={}", if light { "light" } else { "dark" }));
        self.set_light(light);
        // set_light repaints when it changed
      }
      Key::Clock => {
        let v = if i == 0 { "24" } else { "12" };
        if s.str_of("clock") != v {
          set_pref("clock", json!(v), true);
        }
      }
      Key::UiScale => {
        let v = crate::native_bar::scale::STEPS[i.min(crate::native_bar::scale::STEPS.len() - 1)];
        // the core writes it, moves the window manager's top gap with the
        // bar and announces ll:prefs: every window is made again at once
        if s.s["uiScale"].as_u64() != Some(v as u64) {
          set_pref("uiScale", json!(v), false);
        }
      }
      Key::NightMode => {
        let v = ["manual", "after", "range"][i.min(2)];
        night(&["--nightlight-set", "mode", v]);
      }
      Key::WsMode => s.ws.mode = i.min(1),
    }
    self.settings_paint();
  }

  /// A drop-down box: the shared menu under it, the current item checked.
  fn settings_dropdown(&mut self, sel: Sel) {
    let Some(s) = self.settings.as_ref() else { return };
    let Some(anchor) = s.region(Hit::Select(sel)).map(|r| r.r) else { return };
    let (labels, current) = pages::list_for(s, sel);
    let tr = |t: &str| self.model.tr(t);
    let items: Vec<Item> = labels.iter().enumerate().map(|(i, l)| Item::new(&i.to_string(), None, tr(l)).checked(i == current)).collect();
    let mut wr = RECT::default();
    unsafe {
      let _ = GetWindowRect(s.hwnd, &mut wr);
    }
    let at = POINT { x: wr.left + (anchor.x * s.scale).round() as i32, y: wr.top + ((anchor.bottom() + 4.0) * s.scale).round() as i32 };
    self.menu_open(at, MenuFocus::Keep, items, move |ui, id| {
      if let Ok(i) = id.parse::<usize>() {
        ui.settings_pick(sel, i);
      }
    });
  }

  fn settings_pick(&mut self, sel: Sel, i: usize) {
    let Some(s) = self.settings.as_mut() else { return };
    match sel {
      Sel::Lang => {
        let code = pages::LANGS.get(i).map_or("system", |l| l.0);
        if s.str_of("language") != code && !(code == "system" && s.str_of("language").is_empty()) {
          set_pref("language", json!(code), true);
        }
      }
      Sel::SegMon(seg) => s.ws.set_segment_monitor(seg, i),
    }
    self.settings_paint();
  }

  fn settings_color(&mut self, c: String) {
    let Some(s) = self.settings.as_mut() else { return };
    if s.color_saving {
      return;
    }
    s.color_saving = true;
    s.color_error = false;
    std::thread::spawn(move || {
      let path = format!("/focus-color?v={}", c.replace('#', "%23"));
      let saved = core_api::post(&path)
        .filter(|(code, _)| *code == 200)
        .and_then(|(_, body)| serde_json::from_slice::<Value>(&body).ok())
        .filter(|v| v["ok"].as_bool() == Some(true))
        .and_then(|v| v["focusColor"].as_str().map(str::to_string));
      crate::native_bar::send(Msg::Settings(Event::Color(saved)));
    });
    self.settings_paint();
  }

  fn settings_act(&mut self, a: Act) {
    let Some(s) = self.settings.as_mut() else { return };
    match a {
      Act::Close => return self.settings_close(),
      Act::ApplyNow => core_run(&["--restart-shell"]),
      Act::HexApply => {
        let v = if s.hex.starts_with('#') { s.hex.to_lowercase() } else { format!("#{}", s.hex.to_lowercase()) };
        if v.len() == 7 && v[1..].chars().all(|c| c.is_ascii_hexdigit()) {
          s.hex_bad = false;
          return self.settings_color(v);
        }
        s.hex_bad = true;
      }
      Act::SampleToasts => {
        self.toast_add(json!({ "kind": "info", "title": "Örnek bildirim", "body": "Bilgi bildirimleri", "icon": "notifications" }));
        self.toast_add(json!({ "kind": "error", "title": "Örnek uyarı", "body": "Uyarılar ve hatalar", "icon": "warning" }));
        return;
      }
      Act::EditKeys => {
        // the shortcut editor is the right panel's page
        self.settings_close();
        std::thread::spawn(move || {
          std::thread::sleep(std::time::Duration::from_millis(180));
          crate::bus::publish(crate::bus::Event::SidebarOpenPage("keys".into()));
        });
        return;
      }
      Act::EditConfig => core_run(&["--edit-config"]),
      Act::RestartDesktop => core_run(&["--restart-desktop"]),
      Act::BlackBox => {
        std::thread::spawn(|| {
          core_api::run_core_output(&["--black-box"]);
          if let Some(v) = core_api::run_core_output(&["--health"]).and_then(|o| serde_json::from_str::<Value>(&o).ok()) {
            crate::native_bar::send(Msg::Settings(Event::Health(v)));
          }
        });
      }
      Act::OpenLogs => explorer(s.health.as_ref().and_then(|h| h["logsDir"].as_str()).unwrap_or_else(|| s.s["logsDir"].as_str().unwrap_or_default())),
      Act::OpenConfigDir => explorer(&s.str_of("configDir")),
      Act::WmReload => core_run(&["--wm", "wm-reload-config"]),
      Act::WmRedraw => core_run(&["--wm", "wm-redraw"]),
      Act::CheckUpdates => {
        self.settings_close();
        self.update_event(crate::native_bar::update::Event::Check(true));
        return;
      }
      Act::SourceCode => core_run(&["--open", "https://github.com/KaanAlper/logical-lunge"]),
      Act::WsCountSave => {
        if let Some(args) = s.ws.count_args() {
          save_workspaces(args);
        }
      }
      Act::WsFirstSave => {
        if let Some(args) = s.ws.first_args() {
          save_workspaces(args);
        }
      }
      Act::WsApplyDivider => {
        if let Some(args) = s.ws.divider_args() {
          save_workspaces(args);
        }
      }
      Act::WsCustomSave => {
        if let Some(args) = s.ws.custom_args() {
          save_workspaces(args);
        }
      }
    }
    self.settings_paint();
  }

  fn settings_char(&mut self, unit: u16) {
    if self.menu_is_open() {
      return;
    }
    let Some(s) = self.settings.as_mut() else { return };
    let Some(Hit::Field(f)) = s.focus else { return };
    let Some(c) = char::from_u32(unit as u32) else { return };
    if c.is_control() {
      return;
    }
    let (text, ok, max): (&mut String, bool, usize) = match f {
      Fld::Hex => (&mut s.hex, c == '#' || c.is_ascii_hexdigit(), 7),
      Fld::WsCount => (&mut s.ws.count_text, c.is_ascii_digit(), 3),
      Fld::WsFirst => (&mut s.ws.first_text, c.is_ascii_digit(), 3),
      Fld::NightFrom => (&mut s.night_from, c.is_ascii_digit() || c == ':', 5),
      Fld::NightTo => (&mut s.night_to, c.is_ascii_digit() || c == ':', 5),
    };
    if !ok || text.chars().count() >= max {
      return;
    }
    text.push(c);
    // typed times get their colon
    if matches!(f, Fld::NightFrom | Fld::NightTo) && text.len() == 2 && !text.contains(':') {
      text.push(':');
    }
    if f == Fld::Hex {
      s.hex_bad = false;
    }
    s.ws.fields_changed(f);
    self.settings_paint();
  }

  /// true: the key was the window's
  fn settings_key(&mut self, vk: u16) -> bool {
    // the shared menu (a drop-down) has the keys while open
    if self.menu_key(vk) {
      return true;
    }
    let Some(s) = self.settings.as_mut() else { return false };
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    // a text field
    if let Some(Hit::Field(f)) = s.focus {
      if vk == VK_BACK.0 {
        let text = match f {
          Fld::Hex => &mut s.hex,
          Fld::WsCount => &mut s.ws.count_text,
          Fld::WsFirst => &mut s.ws.first_text,
          Fld::NightFrom => &mut s.night_from,
          Fld::NightTo => &mut s.night_to,
        };
        text.pop();
        s.ws.fields_changed(f);
        self.settings_paint();
        return true;
      }
      if vk == VK_RETURN.0 {
        match f {
          Fld::Hex => self.settings_act(Act::HexApply),
          Fld::WsCount => self.settings_act(Act::WsCountSave),
          Fld::WsFirst => self.settings_act(Act::WsFirstSave),
          Fld::NightFrom | Fld::NightTo => {
            s.blur_field(f);
            self.settings_paint();
          }
        }
        return true;
      }
      if (vk == VK_UP.0 || vk == VK_DOWN.0) && matches!(f, Fld::WsCount | Fld::WsFirst) {
        s.ws.step(f, if vk == VK_UP.0 { 1 } else { -1 });
        self.settings_paint();
        return true;
      }
    }
    match vk {
      k if k == VK_ESCAPE.0 => {
        self.settings_close();
      }
      k if k == VK_TAB.0 => {
        s.focus_ring = true;
        let all = s.focusables();
        if all.is_empty() {
          return true;
        }
        let at = s.focus.and_then(|f| all.iter().position(|h| *h == f));
        let next = match (at, shift) {
          (None, false) => 0,
          (None, true) => all.len() - 1,
          (Some(i), false) => (i + 1) % all.len(),
          (Some(i), true) => (i + all.len() - 1) % all.len(),
        };
        s.set_focus(Some(all[next]));
        // the regions move with the scroll: reveal after a paint
        self.settings_paint();
        if let Some(s) = self.settings.as_mut() {
          s.reveal(all[next]);
        }
        self.settings_paint();
      }
      k if k == VK_RETURN.0 || k == VK_SPACE.0 => {
        s.focus_ring = true;
        if let Some(f) = s.focus {
          self.settings_press(f);
        }
      }
      k if k == VK_LEFT.0 || k == VK_RIGHT.0 || k == VK_UP.0 || k == VK_DOWN.0 => {
        s.focus_ring = true;
        let horizontal = k == VK_LEFT.0 || k == VK_RIGHT.0;
        let d = if k == VK_LEFT.0 || k == VK_UP.0 { -1 } else { 1 };
        match s.focus {
          Some(Hit::Nav(i)) if !horizontal => {
            let next = (i as i32 + d).rem_euclid(PAGES.len() as i32) as usize;
            s.focus = Some(Hit::Nav(next));
            s.page = next;
            self.settings_page_opened();
            self.settings_paint();
          }
          Some(Hit::Seg(key, i)) if horizontal => {
            let n = pages::seg_len(key);
            let next = (i as i32 + d).clamp(0, n as i32 - 1) as usize;
            if next != i {
              s.focus = Some(Hit::Seg(key, next));
              self.settings_seg(key, next);
            }
          }
          Some(Hit::Slider(sl)) if horizontal => {
            let (min, max, step) = pages::slider_range(sl);
            let v = (pages::slider_value(s, sl) + d * step).clamp(min, max);
            self.settings_slider_set(sl, v);
          }
          Some(Hit::WsDivider(i)) if horizontal => {
            s.ws.nudge_divider(i, d);
            self.settings_paint();
          }
          Some(Hit::Swatch(i)) if horizontal => {
            let next = (i as i32 + d).clamp(0, pages::COLORS.len() as i32 - 1) as usize;
            s.focus = Some(Hit::Swatch(next));
            self.settings_paint();
          }
          _ if !horizontal => {
            s.scroll_by(d as f32 * WHEEL_STEP);
            self.settings_paint();
          }
          _ => {}
        }
      }
      k if k == VK_NEXT.0 || k == VK_PRIOR.0 => {
        let page = s.view_h() - 60.0;
        s.scroll_by(if k == VK_NEXT.0 { page } else { -page });
        self.settings_paint();
      }
      k if k == VK_HOME.0 || k == VK_END.0 => {
        let to = if k == VK_HOME.0 { 0.0 } else { s.max_scroll() };
        s.scroll = to;
        self.settings_paint();
      }
      _ => return false,
    }
    true
  }
}

/// `--set-workspaces ...`, then `--settings-get` for what the core made of it.
fn save_workspaces(args: Vec<String>) {
  std::thread::spawn(move || {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = core_api::run_core_output(&refs).and_then(|o| serde_json::from_str::<Value>(&o).ok());
    let event = match result {
      Some(r) if r["ok"].as_bool() == Some(true) => {
        match core_api::run_core_output(&["--settings-get"]).and_then(|o| serde_json::from_str::<Value>(&o).ok()) {
          Some(fresh) if fresh["workspaces"].is_array() => Event::Workspaces(Ok(fresh)),
          _ => Event::Workspaces(Err("Yeni ayar okunamadı.".into())),
        }
      }
      Some(r) => Event::Workspaces(Err(r["error"].as_str().unwrap_or("Değişiklik uygulanamadı.").to_string())),
      None => Event::Workspaces(Err("Değişiklik uygulanamadı.".into())),
    };
    crate::native_bar::send(Msg::Settings(event));
  });
}

/// The mouse left the window (TrackMouseEvent).
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;

