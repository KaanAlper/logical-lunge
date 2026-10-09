//! The settings window (the web edition's settings.html): opened from the
//! right panel's gear (`ll:settings-toggle`) in the middle of the primary
//! monitor. Pages: look (theme, accent, motion, notifications), workspaces
//! (count, order, monitor assignments), language and clock, shortcuts,
//! night light, system health, advanced, about.
//!
//! The core does every change (`lunge.exe --settings-get / --set-pref /
//! --set-workspaces / --nightlight-set / --health ...`, `/focus-color`,
//! `/pref`), always off the UI thread; answers come back as
//! [`Event`]s. The window is made when it opens and destroyed when it
//! closes. It does not close when it loses the keyboard (as on the web);
//! Esc, the close button, the gear again or the Super menu close it.
//!
//! Keyboard: Tab / Shift+Tab move between controls, Enter / Space press
//! them, arrows change a choice, a slider or a monitor boundary (Up / Down
//! on the page list change the page), PageUp / PageDown / Home / End
//! scroll, Esc closes an open list, then the window.

mod input;
mod pages;
mod widgets;
mod workspaces;

use std::sync::atomic::Ordering;

use serde_json::{json, Value};
use windows::{
  core::{Interface, HSTRING},
  Win32::{
    Foundation::HWND,
    Graphics::DirectComposition::{IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
  },
};

use super::{
  anim::{self, POP_IN, POP_OUT, SPRING_IN},
  core_api, gfx,
  view::{Painter, Theme},
  Layer, Msg, Ui, CLASS, SETTINGS_HWND, TIMER_SETTINGS_CLOSE, TIMER_SETTINGS_COMMIT, TIMER_SETTINGS_HEALTH,
  TIMER_SETTINGS_SAVED,
};

pub(super) use widgets::{Cursor, Region};
use workspaces::Ws;

/// "Logical Lunge · settings": the core keeps it above workspace slides
/// (and `--raise` finds it by this title)
const TITLE: &str = "Logical Lunge · settings";
/// settings.css `.win` (the card) and the room around it for its shadow
pub(super) const CARD_W: f32 = 860.0;
pub(super) const CARD_H: f32 = 600.0;
pub(super) const M: f32 = 16.0;
pub(super) const NAV_W: f32 = 220.0;
pub(super) const HEAD_H: f32 = 64.0;
pub(super) const BODY_PAD: f32 = 28.0;
/// the "apply now" bar while a language / clock change waits for a restart
pub(super) const APPLY_H: f32 = 66.0;
const WHEEL_STEP: f32 = 56.0;

/// settings.html SECTIONS: (icon, label)
pub(super) const PAGES: [(&str, &str); 8] = [
  ("palette", "Görünüm"),
  ("grid_view", "Workspace"),
  ("translate", "Dil ve saat"),
  ("keyboard", "Kısayollar"),
  ("nightlight", "Gece ışığı"),
  ("monitor_heart", "Sistem sağlığı"),
  ("tune", "Gelişmiş"),
  ("info", "Hakkında"),
];
pub(super) const PAGE_WORKSPACES: usize = 1;
pub(super) const PAGE_NIGHT: usize = 4;
pub(super) const PAGE_HEALTH: usize = 5;

/// Segmented choices.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Key {
  Theme,
  Clock,
  UiScale,
  BorderStyle,
  NightMode,
  WsMode,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Sw {
  Animations,
  Gestures,
  WinToasts,
  Takeover,
  Night,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Sl {
  ToastInfo,
  ToastError,
  NightLevel,
}

/// Text fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Fld {
  Hex,
  WsCount,
  WsFirst,
  NightFrom,
  NightTo,
}

/// Drop-down lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Sel {
  Lang,
  /// the monitor of a workspace range (its index)
  SegMon(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Act {
  Close,
  ApplyNow,
  HexApply,
  SampleToasts,
  EditKeys,
  EditConfig,
  RestartDesktop,
  BlackBox,
  OpenLogs,
  OpenConfigDir,
  WmReload,
  WmRedraw,
  CheckUpdates,
  SourceCode,
  WsCountSave,
  WsFirstSave,
  WsApplyDivider,
  WsCustomSave,
}

/// What a control is: what the mouse and the keyboard act on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(super) enum Hit {
  Nav(usize),
  Act(Act),
  Seg(Key, usize),
  Switch(Sw),
  Slider(Sl),
  Field(Fld),
  Step(Fld, i32),
  Select(Sel),
  Swatch(usize),
  /// the workspace number line (its lens follows the pointer)
  WsLine,
  WsDivider(usize),
  WsCustom(usize),
}

/// Answers from the core.
pub(super) enum Event {
  Loaded(Value),
  /// a preference was saved: (key, value, needs a shell restart)
  Pref(String, Value, bool),
  /// the accent colour was saved (None: it was not)
  Color(Option<String>),
  Night(Value),
  Health(Value),
  /// workspaces saved: the fresh settings, or why not
  Workspaces(Result<Value, String>),
}

pub(super) struct Settings {
  pub hwnd: HWND,
  /// DPI scale (smaller on a monitor too small for the window)
  scale: f32,
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  layer: Layer,
  pub page: usize,
  /// `--settings-get` (null until it answers)
  pub s: Value,
  /// a language / clock change waits for the shell to restart
  pub dirty: bool,
  pub hex: String,
  pub hex_bad: bool,
  pub color_saving: bool,
  pub color_error: bool,
  pub night: Option<Value>,
  pub night_level: i32,
  pub night_from: String,
  pub night_to: String,
  pub health: Option<Value>,
  pub ws: Ws,
  /// slider values not yet sent (sent together a moment after the last move)
  pending: Vec<(Sl, i32)>,
  pub scroll: f32,
  pub content_h: f32,
  pub regions: Vec<Region>,
  pub hover: Option<Hit>,
  pub focus: Option<Hit>,
  /// focus rings only once the keyboard is used (as :focus-visible)
  pub focus_ring: bool,
  /// the control the mouse went down on (a click is down and up on it)
  pressed: Option<Hit>,
  /// a slider or a monitor boundary being dragged
  drag: Option<Hit>,
  closing: bool,
}

/// Is the theme light (its first layer is)?
pub(super) fn is_light(t: &Theme) -> bool {
  let c = t.layer0;
  (c.0 as u32 + c.1 as u32 + c.2 as u32) > 3 * 128
}

fn make_window(gfx: &gfx::Gfx) -> anyhow::Result<(HWND, f32, IDCompositionTarget, IDCompositionVisual2, Layer)> {
  let layout = super::monitor_layout();
  let (left, top, right, bottom, dpi) = layout
    .iter()
    .copied()
    .find(|m| m.0 == 0 && m.1 == 0)
    .or_else(|| layout.first().copied())
    .ok_or_else(|| anyhow::anyhow!("no monitor"))?;
  let (mw, mh) = ((right - left) as f32, (bottom - top) as f32);
  let dpi_scale = crate::native_bar::scale::of_dpi(dpi);
  let (ww, wh) = (CARD_W + 2.0 * M, CARD_H + 2.0 * M);
  let fit = ((mw / dpi_scale - 24.0) / ww).min((mh / dpi_scale - 24.0) / wh).clamp(0.5, 1.0);
  let scale = dpi_scale * fit;
  let (pw, ph) = ((ww * scale).ceil() as i32, (wh * scale).ceil() as i32);
  let (x, y) = (left + ((mw as i32 - pw) / 2), top + ((mh as i32 - ph) / 2));
  let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
  unsafe {
    let hwnd = CreateWindowExW(ex, CLASS, &HSTRING::from(TITLE), WS_POPUP, x, y, pw, ph, None, None, GetModuleHandleW(None)?, None)?;
    let made = (|| -> windows::core::Result<(IDCompositionTarget, IDCompositionVisual2, Layer)> {
      let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let root = gfx.dcomp.CreateVisual()?;
      let layer = Layer::new(gfx, pw as u32, ph as u32)?;
      root.AddVisual(&layer.visual, false, None)?;
      target.SetRoot(&root)?;
      Ok((target, root, layer))
    })();
    match made {
      Ok((target, root, layer)) => Ok((hwnd, scale, target, root, layer)),
      Err(err) => {
        let _ = DestroyWindow(hwnd);
        Err(err.into())
      }
    }
  }
}

/// `lunge.exe <args>` on another thread; its JSON answer becomes an event.
fn core_json(args: Vec<String>, done: impl FnOnce(Option<Value>) -> Option<Event> + Send + 'static) {
  std::thread::spawn(move || {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let v = core_api::run_core_output(&refs).and_then(|o| serde_json::from_str::<Value>(&o).ok());
    if let Some(e) = done(v) {
      super::send(Msg::Settings(e));
    }
  });
}

fn core_run(args: &[&str]) {
  let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
  std::thread::spawn(move || {
    let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    core_api::run_core_output(&refs);
  });
}

fn explorer(dir: &str) {
  super::launch::open(dir, "", super::launch::Verb::Open);
}

fn load() {
  core_json(vec!["--settings-get".into()], |v| v.map(Event::Loaded));
}

fn load_health() {
  core_json(vec!["--health".into()], |v| v.map(Event::Health));
}

fn set_pref(key: &str, value: Value, restart: bool) {
  let text = match &value {
    Value::String(s) => s.clone(),
    other => other.to_string(),
  };
  let key = key.to_string();
  core_json(vec!["--set-pref".into(), key.clone(), text], move |v| {
    (v?["ok"].as_bool() == Some(true)).then(|| Event::Pref(key, value, restart))
  });
}

fn night(args: &[&str]) {
  core_json(args.iter().map(|s| s.to_string()).collect(), |v| v.map(Event::Night));
}

/// `HH:MM` (what the time fields take)
fn valid_time(s: &str) -> bool {
  let b = s.as_bytes();
  b.len() == 5
    && b[2] == b':'
    && s[..2].parse::<u32>().is_ok_and(|h| h < 24)
    && s[3..].parse::<u32>().is_ok_and(|m| m < 60)
}

impl Settings {
  fn str_of(&self, k: &str) -> String {
    self.s[k].as_str().unwrap_or_default().to_string()
  }

  /// The controls that take the keyboard, in order.
  fn focusables(&self) -> Vec<Hit> {
    let mut seen = Vec::new();
    for r in &self.regions {
      if r.focus && !seen.contains(&r.hit) {
        seen.push(r.hit);
      }
    }
    seen
  }

  fn region(&self, hit: Hit) -> Option<&Region> {
    self.regions.iter().find(|r| r.hit == hit)
  }

  /// The control under (x, y) (window DIP): the last drawn wins (a row's
  /// own button over the row).
  fn hit_at(&self, x: f32, y: f32) -> Option<&Region> {
    self.regions.iter().rev().find(|r| r.r.contains(x, y) && r.visible)
  }

  fn view_h(&self) -> f32 {
    CARD_H - HEAD_H - if self.dirty { APPLY_H } else { 0.0 }
  }

  fn max_scroll(&self) -> f32 {
    (self.content_h - self.view_h()).max(0.0)
  }

  fn scroll_by(&mut self, d: f32) {
    self.scroll = (self.scroll + d).clamp(0.0, self.max_scroll());
  }

  /// Scrolls so that the focused control is in view.
  fn reveal(&mut self, hit: Hit) {
    let Some(r) = self.region(hit).map(|r| r.r) else { return };
    let top = M + HEAD_H;
    let bottom = top + self.view_h();
    if r.y < top && !matches!(hit, Hit::Nav(_) | Hit::Act(Act::Close) | Hit::Act(Act::ApplyNow)) {
      self.scroll_by(r.y - top - 12.0);
    } else if r.bottom() > bottom && !matches!(hit, Hit::Nav(_) | Hit::Act(Act::Close) | Hit::Act(Act::ApplyNow)) {
      self.scroll_by(r.bottom() - bottom + 12.0);
    }
  }

  /// Leaving a text field: the time fields are applied, the numbers
  /// cleaned up.
  fn blur_field(&mut self, f: Fld) {
    match f {
      Fld::NightFrom | Fld::NightTo => {
        let (key, text) = if f == Fld::NightFrom { ("from", self.night_from.clone()) } else { ("to", self.night_to.clone()) };
        let saved = self.night.as_ref().and_then(|n| n[key].as_str().map(str::to_string));
        if valid_time(&text) && saved.as_deref() != Some(text.as_str()) {
          night(&["--nightlight-set", key, &text]);
        } else if !valid_time(&text) {
          // back to what is set
          let def = if key == "from" { "20:00" } else { "07:00" };
          let v = saved.unwrap_or_else(|| def.to_string());
          if f == Fld::NightFrom { self.night_from = v } else { self.night_to = v }
        }
      }
      Fld::WsCount | Fld::WsFirst => self.ws.clean_fields(),
      Fld::Hex => {}
    }
  }

  fn set_focus(&mut self, hit: Option<Hit>) {
    if self.focus == hit {
      return;
    }
    if let Some(Hit::Field(f)) = self.focus {
      self.blur_field(f);
    }
    self.focus = hit;
  }
}

impl Ui {
  /// `ll:settings-toggle` (the right panel's gear)
  /// Shown and not closing.
  pub(super) fn settings_is_open(&self) -> bool {
    self.settings.as_ref().is_some_and(|s| !s.closing)
  }

  pub(super) fn settings_toggle(&mut self) {
    match &self.settings {
      Some(s) if !s.closing => self.settings_close(),
      _ => self.settings_open(),
    }
  }

  fn settings_open(&mut self) {
    self.settings_destroy();
    let (hwnd, scale, target, root, layer) = match make_window(&self.gfx) {
      Ok(w) => w,
      Err(err) => {
        tracing::warn!("Settings: window: {:?}", err);
        return;
      }
    };
    SETTINGS_HWND.store(hwnd.0 as isize, Ordering::Release);
    self.settings = Some(Settings {
      hwnd,
      scale,
      _target: target,
      _root: root,
      layer,
      page: self.settings_page,
      s: Value::Null,
      dirty: false,
      hex: String::new(),
      hex_bad: false,
      color_saving: false,
      color_error: false,
      night: None,
      night_level: 50,
      night_from: "20:00".into(),
      night_to: "07:00".into(),
      health: None,
      ws: Ws::default(),
      pending: Vec::new(),
      scroll: 0.0,
      content_h: 0.0,
      regions: Vec::new(),
      hover: None,
      focus: None,
      focus_ring: false,
      pressed: None,
      drag: None,
      closing: false,
    });
    load();
    night(&["--nightlight", "status"]);
    self.settings_page_opened();
    self.settings_paint();
    if let Err(err) = self.settings_enter() {
      tracing::debug!("Settings: entrance: {:?}", err);
    }
    unsafe {
      let _ = ShowWindow(hwnd, SW_SHOW);
      let _ = SetForegroundWindow(hwnd);
      // the gear's panel just closed: Windows may not hand over the keyboard
      if GetForegroundWindow() != hwnd {
        core_api::run_core(&["--raise", TITLE]);
      }
    }
  }

  /// settings.css `.win.open`: from 96 % and transparent (220 ms fade,
  /// 320 ms --elementMove growth); nothing moves with animations off.
  fn settings_enter(&self) -> windows::core::Result<()> {
    let Some(s) = &self.settings else { return Ok(()) };
    if !self.model.animations {
      return Ok(());
    }
    let dcomp = &self.gfx.dcomp;
    unsafe {
      let v: IDCompositionVisual3 = s.layer.visual.cast()?;
      v.SetOpacity(&anim::build(dcomp, 0.0, 1.0, 220.0, POP_IN)?)?;
      let grow = dcomp.CreateScaleTransform()?;
      grow.SetCenterX2((CARD_W / 2.0 + M) * s.scale)?;
      grow.SetCenterY2((CARD_H / 2.0 + M) * s.scale)?;
      let size = anim::build(dcomp, 0.96, 1.0, 320.0, SPRING_IN)?;
      grow.SetScaleX(&size)?;
      grow.SetScaleY(&size)?;
      s.layer.visual.SetTransform(&grow)?;
      dcomp.Commit()
    }
  }

  pub(super) fn settings_close(&mut self) {
    match self.settings.as_mut() {
      Some(s) if !s.closing => {
        s.closing = true;
        if let Some(Hit::Field(f)) = s.focus {
          s.blur_field(f);
        }
      }
      _ => return,
    }
    self.menu_close();
    self.settings_flush();
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SETTINGS_HEALTH);
    }
    if !self.model.animations || self.settings_fade_out().is_err() {
      self.settings_destroy();
      return;
    }
    unsafe { SetTimer(self.msg_hwnd, TIMER_SETTINGS_CLOSE, 190, None) };
  }

  fn settings_fade_out(&self) -> windows::core::Result<()> {
    let Some(s) = &self.settings else { return Ok(()) };
    let dcomp = &self.gfx.dcomp;
    unsafe {
      let v: IDCompositionVisual3 = s.layer.visual.cast()?;
      v.SetOpacity(&anim::build(dcomp, 1.0, 0.0, 180.0, POP_OUT)?)?;
      let shrink = dcomp.CreateScaleTransform()?;
      shrink.SetCenterX2((CARD_W / 2.0 + M) * s.scale)?;
      shrink.SetCenterY2((CARD_H / 2.0 + M) * s.scale)?;
      let size = anim::build(dcomp, 1.0, 0.96, 200.0, POP_OUT)?;
      shrink.SetScaleX(&size)?;
      shrink.SetScaleY(&size)?;
      s.layer.visual.SetTransform(&shrink)?;
      dcomp.Commit()
    }
  }

  /// Gone at once (closed, a monitor or graphics change).
  pub(super) fn settings_destroy(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SETTINGS_CLOSE);
      let _ = KillTimer(self.msg_hwnd, TIMER_SETTINGS_HEALTH);
      let _ = KillTimer(self.msg_hwnd, TIMER_SETTINGS_SAVED);
    }
    self.settings_flush();
    SETTINGS_HWND.store(0, Ordering::Release);
    if let Some(s) = self.settings.take() {
      self.settings_page = s.page;
      unsafe {
        let _ = DestroyWindow(s.hwnd);
      }
    }
  }

  /// Sends the slider values still waiting.
  fn settings_flush(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SETTINGS_COMMIT);
    }
    let Some(s) = self.settings.as_mut() else { return };
    for (sl, v) in s.pending.drain(..) {
      match sl {
        Sl::ToastInfo => set_pref("toastInfo", json!(v), false),
        Sl::ToastError => set_pref("toastError", json!(v), false),
        Sl::NightLevel => night(&["--nightlight-set", "level", &v.to_string()]),
      }
    }
  }

  pub(super) fn settings_timer(&mut self, id: usize) {
    match id {
      TIMER_SETTINGS_CLOSE => self.settings_destroy(),
      TIMER_SETTINGS_COMMIT => self.settings_flush(),
      TIMER_SETTINGS_HEALTH => {
        // only while looked at (each look is a process)
        if let Some(s) = &self.settings {
          if s.page == PAGE_HEALTH && unsafe { GetForegroundWindow() } == s.hwnd {
            load_health();
          }
        }
      }
      TIMER_SETTINGS_SAVED => {
        unsafe {
          let _ = KillTimer(self.msg_hwnd, TIMER_SETTINGS_SAVED);
        }
        if let Some(s) = self.settings.as_mut() {
          s.ws.saved = false;
        }
        self.settings_paint();
      }
      _ => {}
    }
  }

  /// The page changed (or the window opened on it): what it reads.
  fn settings_page_opened(&mut self) {
    let Some(s) = self.settings.as_mut() else { return };
    s.scroll = 0.0;
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SETTINGS_HEALTH);
    }
    match s.page {
      PAGE_HEALTH => {
        load_health();
        unsafe { SetTimer(self.msg_hwnd, TIMER_SETTINGS_HEALTH, 3000, None) };
      }
      PAGE_NIGHT => night(&["--nightlight", "status"]),
      _ => {}
    }
  }

  pub(super) fn settings_event(&mut self, e: Event) {
    let Some(s) = self.settings.as_mut() else { return };
    match e {
      Event::Loaded(v) => {
        s.s = v;
        s.ws.sync(&s.s);
      }
      Event::Pref(k, v, restart) => {
        s.s[k.as_str()] = v;
        if restart {
          s.dirty = true;
        }
      }
      Event::Color(Some(c)) => {
        s.color_saving = false;
        s.s["focusColor"] = json!(c);
      }
      Event::Color(None) => {
        s.color_saving = false;
        s.color_error = true;
      }
      Event::Night(v) => {
        s.night_level = v["level"].as_i64().unwrap_or(50) as i32;
        if s.focus != Some(Hit::Field(Fld::NightFrom)) {
          s.night_from = v["from"].as_str().unwrap_or("20:00").to_string();
        }
        if s.focus != Some(Hit::Field(Fld::NightTo)) {
          s.night_to = v["to"].as_str().unwrap_or("07:00").to_string();
        }
        s.night = Some(v);
      }
      Event::Health(v) => s.health = Some(v),
      Event::Workspaces(Ok(fresh)) => {
        s.s["workspaces"] = fresh["workspaces"].clone();
        s.s["monitors"] = fresh["monitors"].clone();
        s.ws.busy = false;
        s.ws.saved = true;
        s.ws.error.clear();
        s.ws.sync(&s.s);
        unsafe { SetTimer(self.msg_hwnd, TIMER_SETTINGS_SAVED, 2500, None) };
      }
      Event::Workspaces(Err(why)) => {
        s.ws.busy = false;
        s.ws.error = why;
      }
    }
    self.settings_paint();
  }

  /// The theme or the accent changed.
  pub(super) fn settings_restyle(&mut self) {
    if self.settings.is_some() {
      self.settings_paint();
    }
  }

  pub(super) fn settings_paint(&mut self) {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, model, settings, .. } = self;
    let Some(s) = settings.as_mut() else { return };
    let tr = |t: &str| model.tr(t);
    let mut requests = Vec::new();
    let scale = s.scale;
    let surface = s.layer.surface.clone();
    let drawn = gfx::draw_surface(&surface, scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = pages::paint(&mut p, &theme, s, &tr) {
        tracing::warn!("Settings: paint: {:?}", err);
      }
      Ok(())
    });
    let committed = drawn.and_then(|_| unsafe { gfx.dcomp.Commit() });
    if let Err(err) = committed {
      tracing::warn!("Settings: draw: {:?}", err);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn times_are_hours_and_minutes() {
    assert!(valid_time("20:00"));
    assert!(valid_time("07:05"));
    assert!(!valid_time("24:00"));
    assert!(!valid_time("7:00"));
    assert!(!valid_time("12:60"));
    assert!(!valid_time(""));
  }
}
