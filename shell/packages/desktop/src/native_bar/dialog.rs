//! The one dialog of Logical Lunge: every question, and every message that
//! needs an answer (Yes/No, OK/Cancel, custom buttons, an optional
//! checkbox), from the shell, the core (`POST /dialog`, `lunge.exe --ask`),
//! our scripts and the window manager. Windows' message boxes are never
//! used; plain notices go to the notification cards.
//!
//! ```ignore
//! ui.dialog_open(Spec::question(tr("Silinsin mi?"), body, vec![tr("Sil"), tr("Vazgeç")]).cancel(1),
//!   |ui, answer| if answer.button == Some(0) { ... });
//! ```
//!
//! One window over the monitor with the foreground window (else the
//! pointer): a scrim drawn on a 32 px surface the compositor stretches, and
//! the panel. It takes the keyboard: Tab / Shift+Tab move between the
//! checkbox and the buttons, Left / Right between the buttons, Enter presses
//! the focused button, Space presses it or toggles the checkbox, Esc picks
//! the cancel button. Dialogs asked while one is open wait their turn.

use std::collections::VecDeque;

use windows::{
  core::{w, Interface},
  Foundation::Numerics::Matrix3x2,
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      Direct2D::Common::D2D1_COLOR_F,
      DirectComposition::{
        IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3, DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR,
        DCOMPOSITION_BORDER_MODE_HARD,
      },
      Gdi::{GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL},
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
      HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
      Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_DOWN, VK_ESCAPE,
        VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
      },
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  anim::{self, POP_IN, POP_OUT},
  fonts::TextStyle,
  gfx::{self, Gfx, Rect, Rgba},
  popup,
  view::{Align, Painter, Theme},
  Layer, Ui, CLASS, DIALOG_HWND, TIMER_DIALOG_CLOSE, TIMER_DIALOG_WAIT,
};

/// A fullscreen app, a Direct3D game or a presentation is in front.
pub(super) fn fullscreen_busy() -> bool {
  use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
  };
  matches!(
    unsafe { SHQueryUserNotificationState() },
    Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
  )
}

const PANEL_W: f32 = 440.0;
const PAD: f32 = 24.0;
const RADIUS: f32 = 26.0;
/// room around the panel for its shadow
const SHADOW: f32 = 32.0;
const ICON: f32 = 26.0;
const TITLE: TextStyle = TextStyle { size: 18.0, weight: 600.0 };
const BODY: TextStyle = TextStyle { size: 14.0, weight: 400.0 };
const LABEL: TextStyle = TextStyle { size: 14.0, weight: 550.0 };
const CHECK_LABEL: TextStyle = TextStyle { size: 13.5, weight: 450.0 };
const BTN_H: f32 = 40.0;
const BTN_MIN_W: f32 = 88.0;
const BTN_PAD: f32 = 20.0;
const BTN_GAP: f32 = 8.0;
const CHECK_BOX: f32 = 18.0;
const CHECK_H: f32 = 32.0;
const BODY_MAX_H: f32 = 360.0;
const SCRIM_PX: u32 = 32;
/// the core's mark on input it sends itself ("LLK1"): its hooks let it pass
const LL_MARK: usize = 0x4C4C_4B31;
pub const MAX_BUTTONS: usize = 3;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
  Info,
  Warning,
  Error,
  Question,
}

impl Kind {
  pub fn parse(s: &str) -> Option<Kind> {
    match s {
      "info" => Some(Kind::Info),
      "warning" => Some(Kind::Warning),
      "error" => Some(Kind::Error),
      "question" => Some(Kind::Question),
      _ => None,
    }
  }

  fn icon(self) -> &'static str {
    match self {
      Kind::Info => "info",
      Kind::Warning => "warning",
      Kind::Error => "error",
      Kind::Question => "help",
    }
  }
}

/// What to ask.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
  pub kind: Kind,
  pub title: String,
  pub body: String,
  /// 1 to 3, left to right
  pub buttons: Vec<String>,
  /// pressed by Enter at first (focused when the dialog opens)
  pub default: usize,
  /// what Esc picks; with one button it is that button
  pub cancel: Option<usize>,
  /// an optional checkbox under the text, and its first state
  pub check: Option<String>,
  pub checked: bool,
}

impl Spec {
  pub fn new(kind: Kind, title: impl Into<String>, body: impl Into<String>, buttons: Vec<String>) -> Self {
    let one = buttons.len() == 1;
    Spec { kind, title: title.into(), body: body.into(), buttons, default: 0, cancel: one.then_some(0), check: None, checked: false }
  }

  pub fn cancel(mut self, i: usize) -> Self {
    self.cancel = Some(i);
    self
  }

  pub fn default_button(mut self, i: usize) -> Self {
    self.default = i;
    self
  }

  pub fn checkbox(mut self, label: impl Into<String>, checked: bool) -> Self {
    self.check = Some(label.into());
    self.checked = checked;
    self
  }

  /// From the core's `{"dialog": {...}}` event (also its validation).
  pub fn from_json(v: &serde_json::Value) -> Option<Spec> {
    let kind = Kind::parse(v["kind"].as_str().unwrap_or("info"))?;
    let title = v["title"].as_str().unwrap_or("").trim().to_string();
    let body = v["body"].as_str().unwrap_or("").to_string();
    let buttons: Vec<String> = v["buttons"]
      .as_array()?
      .iter()
      .filter_map(|b| b.as_str().map(|s| s.trim().to_string()))
      .filter(|s| !s.is_empty())
      .collect();
    if title.is_empty() && body.is_empty() || buttons.is_empty() || buttons.len() > MAX_BUTTONS {
      return None;
    }
    let n = buttons.len();
    let default = v["default"].as_u64().map(|d| d as usize).filter(|&d| d < n).unwrap_or(0);
    let cancel = match v["cancel"].as_i64() {
      Some(c) if c >= 0 && (c as usize) < n => Some(c as usize),
      Some(_) => None,
      None => (n == 1).then_some(0),
    };
    let check = v["check"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let checked = check.is_some() && v["checked"].as_bool().unwrap_or(false);
    Some(Spec { kind, title, body, buttons, default, cancel, check, checked })
  }
}

/// The answer: the button pressed (None: the dialog was closed without one,
/// e.g. the desktop stopped) and the checkbox's state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Answer {
  pub button: Option<usize>,
  pub checked: bool,
}

type Done = Box<dyn FnOnce(&mut Ui, Answer)>;

/// What has the keyboard: the checkbox, or a button.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Focus {
  Check,
  Button(usize),
}

/// Focus order: the checkbox (when there is one), then the buttons.
fn focus_order(spec: &Spec) -> Vec<Focus> {
  let mut v = Vec::new();
  if spec.check.is_some() {
    v.push(Focus::Check);
  }
  v.extend((0..spec.buttons.len()).map(Focus::Button));
  v
}

fn next_focus(spec: &Spec, now: Focus, back: bool) -> Focus {
  let order = focus_order(spec);
  let i = order.iter().position(|f| *f == now).unwrap_or(0) as i32;
  let n = order.len() as i32;
  order[(i + if back { -1 } else { 1 }).rem_euclid(n) as usize]
}

/// Left / Right among the buttons (stays at the ends).
fn step_button(spec: &Spec, now: Focus, dir: i32) -> Focus {
  let n = spec.buttons.len() as i32;
  match now {
    Focus::Button(i) => Focus::Button((i as i32 + dir).clamp(0, n - 1) as usize),
    Focus::Check => Focus::Button(if dir > 0 { 0 } else { (n - 1) as usize }),
  }
}

/// The panel's parts, in DIPs from the panel surface's top left.
#[derive(Clone, Debug, Default)]
struct Parts {
  size: (f32, f32),
  body_h: f32,
  check: Option<Rect>,
  buttons: Vec<Rect>,
}

pub(super) struct Open {
  pub hwnd: HWND,
  spec: Spec,
  done: Option<Done>,
  focus: Focus,
  checked: bool,
  hot: Option<Focus>,
  /// the keyboard was used: focus rings show
  keys: bool,
  scale: f32,
  /// the panel surface's top left in the window (pixels)
  panel_at: (f32, f32),
  parts: Parts,
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  scrim: Layer,
  panel: Layer,
  /// the window that had the keyboard before
  prev_focus: HWND,
  closing: bool,
}

#[derive(Default)]
pub(super) struct Dialogs {
  pub open: Option<Open>,
  queue: VecDeque<(Spec, Done)>,
}

/// prefs.json "animations" (on unless turned off), read when a dialog opens
fn animations_on() -> bool {
  super::model::prefs(std::path::Path::new(""))["animations"].as_bool() != Some(false)
}

/// The monitor of the foreground window (where the user is), else of the
/// pointer: its rectangle and DPI scale.
fn target_monitor() -> (RECT, f32) {
  unsafe {
    let fg = GetForegroundWindow();
    let mut mon = if fg.is_invalid() { Default::default() } else { MonitorFromWindow(fg, MONITOR_DEFAULTTONULL) };
    if mon.is_invalid() {
      let mut p = POINT::default();
      let _ = GetCursorPos(&mut p);
      mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
    }
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(mon, &mut info);
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    (info.rcMonitor, dx as f32 / 96.0)
  }
}

fn button_w(p: &mut Painter, label: &str) -> anyhow::Result<f32> {
  Ok((p.measure(label, LABEL)?.ceil() + 2.0 * BTN_PAD).max(BTN_MIN_W))
}

/// Measures the panel for a monitor `max_w` DIPs wide.
fn measure(p: &mut Painter, spec: &Spec, max_w: f32) -> anyhow::Result<Parts> {
  let mut widths = Vec::new();
  for b in &spec.buttons {
    widths.push(button_w(p, b)?);
  }
  let row_w: f32 = widths.iter().sum::<f32>() + BTN_GAP * (widths.len() as f32 - 1.0);
  let w = PANEL_W.max(row_w + 2.0 * PAD).min(max_w - 2.0 * SHADOW - 32.0).max(260.0);
  let text_w = w - 2.0 * PAD;
  let mut y = SHADOW + PAD;
  // icon over the title (as Material dialogs)
  y += ICON + 14.0;
  if !spec.title.is_empty() {
    y += p.measure_wrapped(&spec.title, TITLE, text_w, 120.0, false)? + 10.0;
  }
  let body_h = if spec.body.is_empty() { 0.0 } else { p.measure_wrapped(&spec.body, BODY, text_w, BODY_MAX_H, false)? };
  y += body_h;
  let check = spec.check.as_ref().map(|_| {
    let r = Rect::new(SHADOW + PAD - 6.0, y + 12.0, text_w + 12.0, CHECK_H);
    y += 12.0 + CHECK_H;
    r
  });
  y += 24.0;
  // buttons right-aligned
  let mut buttons = Vec::new();
  let mut x = SHADOW + w - PAD - row_w;
  for bw in widths {
    buttons.push(Rect::new(x, y, bw, BTN_H));
    x += bw + BTN_GAP;
  }
  y += BTN_H + PAD;
  Ok(Parts { size: (w + 2.0 * SHADOW, y + SHADOW), body_h, check, buttons })
}

#[allow(clippy::too_many_arguments)]
fn paint(p: &mut Painter, t: &Theme, spec: &Spec, parts: &Parts, focus: Focus, hot: Option<Focus>, checked: bool, keys: bool) -> anyhow::Result<()> {
  let w = parts.size.0 - 2.0 * SHADOW;
  let h = parts.size.1 - 2.0 * SHADOW;
  let bx = Rect::new(SHADOW, SHADOW, w, h);
  popup::frame_shadow(p, bx, RADIUS)?;
  p.fill_round(bx, RADIUS, t.surface_container)?;
  p.stroke_round(bx, RADIUS, t.border, 1.0)?;
  let text_w = w - 2.0 * PAD;
  let mut y = bx.y + PAD;
  let icon_c = match spec.kind {
    Kind::Error | Kind::Warning => t.error,
    _ => t.primary,
  };
  p.icon(spec.kind.icon(), bx.x + PAD + ICON / 2.0, y + ICON / 2.0, ICON, false, icon_c)?;
  y += ICON + 14.0;
  if !spec.title.is_empty() {
    let th = p.text_wrapped(&spec.title, Rect::new(bx.x + PAD, y, text_w, 120.0), TITLE, t.on_layer0, false)?;
    y += th + 10.0;
  }
  if !spec.body.is_empty() {
    p.text_wrapped(&spec.body, Rect::new(bx.x + PAD, y, text_w, parts.body_h + 1.0), BODY, t.on_surface_variant, false)?;
  }
  if let (Some(label), Some(r)) = (&spec.check, parts.check) {
    if hot == Some(Focus::Check) {
      p.fill_round(r, 8.0, t.surface_container_high)?;
    }
    if keys && focus == Focus::Check {
      p.stroke_round(r, 8.0, t.primary, 2.0)?;
    }
    let b = Rect::new(r.x + 6.0, r.y + (r.h - CHECK_BOX) / 2.0, CHECK_BOX, CHECK_BOX);
    if checked {
      p.fill_round(b, 4.0, t.primary)?;
      p.icon("check", b.x + CHECK_BOX / 2.0, b.y + CHECK_BOX / 2.0, 16.0, true, t.on_primary)?;
    } else {
      p.stroke_round(b, 4.0, t.on_surface_variant, 2.0)?;
    }
    p.text(label, Rect::new(b.right() + 10.0, r.y, r.right() - b.right() - 16.0, r.h), CHECK_LABEL, t.on_layer0, Align::Left, false)?;
  }
  for (i, (label, r)) in spec.buttons.iter().zip(&parts.buttons).enumerate() {
    let primary = i == spec.default;
    let (bg, fg) = if primary { (t.primary, t.on_primary) } else { (t.surface_container_high, t.on_layer0) };
    p.fill_round(*r, BTN_H / 2.0, bg)?;
    if hot == Some(Focus::Button(i)) {
      // a light veil on hover (state layer)
      p.fill_round(*r, BTN_H / 2.0, Rgba(fg.0, fg.1, fg.2, 0.10))?;
    }
    if keys && focus == Focus::Button(i) {
      p.stroke_round(r.inset(-3.0, -3.0), BTN_H / 2.0 + 3.0, t.primary, 2.0)?;
    }
    p.text(label, *r, LABEL, fg, Align::Center, false)?;
  }
  Ok(())
}

fn make_window(gfx: &Gfx, rect: RECT) -> windows::core::Result<(HWND, IDCompositionTarget, IDCompositionVisual2, Layer)> {
  let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
  unsafe {
    let hwnd = CreateWindowExW(
      WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
      CLASS,
      w!("Logical Lunge · dialog"),
      WS_POPUP,
      rect.left,
      rect.top,
      w,
      h,
      None,
      None,
      GetModuleHandleW(None)?,
      None,
    )?;
    let made = (|| -> windows::core::Result<(IDCompositionTarget, IDCompositionVisual2, Layer)> {
      let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let root = gfx.dcomp.CreateVisual()?;
      let scrim = Layer::new(gfx, SCRIM_PX, SCRIM_PX)?;
      scrim.visual.SetBitmapInterpolationMode(DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR)?;
      scrim.visual.SetBorderMode(DCOMPOSITION_BORDER_MODE_HARD)?;
      scrim.visual.SetTransform2(&Matrix3x2 {
        M11: w as f32 / SCRIM_PX as f32,
        M12: 0.0,
        M21: 0.0,
        M22: h as f32 / SCRIM_PX as f32,
        M31: 0.0,
        M32: 0.0,
      })?;
      root.AddVisual(&scrim.visual, false, None)?;
      target.SetRoot(&root)?;
      Ok((target, root, scrim))
    })();
    match made {
      Ok((target, root, scrim)) => Ok((hwnd, target, root, scrim)),
      Err(err) => {
        let _ = DestroyWindow(hwnd);
        Err(err)
      }
    }
  }
}

/// Lets the dialog take the keyboard: Windows gives the foreground only to
/// the process with the last input, so an unused key (marked for the core's
/// hook) makes it ours, as the menus do.
fn take_foreground(hwnd: HWND) {
  let key = |flags| INPUT {
    r#type: INPUT_KEYBOARD,
    Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0xE8), dwFlags: flags, dwExtraInfo: LL_MARK, ..Default::default() } },
  };
  unsafe {
    SendInput(&[key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)], std::mem::size_of::<INPUT>() as i32);
    let _ = SetForegroundWindow(hwnd);
  }
}

impl Ui {
  /// Asks `spec`; `done` gets the answer. A dialog asked while another is
  /// up waits until that one is answered.
  pub(super) fn dialog_open(&mut self, spec: Spec, done: impl FnOnce(&mut Ui, Answer) + 'static) {
    self.dialogs.queue.push_back((spec, Box::new(done)));
    if self.dialogs.open.is_none() {
      self.dialog_next();
    }
  }

  /// The core's question: it hears at once that the dialog is up (so it
  /// knows a dialog host exists), then the answer.
  pub(super) fn core_dialog(&mut self, v: serde_json::Value) {
    let Some(id) = v["id"].as_u64() else { return };
    let Some(spec) = Spec::from_json(&v) else {
      tracing::warn!("Dialog: the core's question is not valid: {}", v);
      super::core_api::post_async(format!("/dialog-answer?id={id}&b=-1&c=0"));
      return;
    };
    super::core_api::post_async(format!("/dialog-shown?id={id}"));
    self.dialog_open(spec, move |_, a| {
      let b = a.button.map_or(-1, |b| b as i64);
      super::core_api::post_async(format!("/dialog-answer?id={id}&b={b}&c={}", a.checked as u8));
    });
  }

  /// TIMER_DIALOG_WAIT: whether the questions held back for a game can open.
  pub(super) fn dialog_waited(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_DIALOG_WAIT);
    }
    if self.dialogs.open.is_none() {
      self.dialog_next();
    }
  }

  pub(super) fn dialog_is_open(&self) -> bool {
    self.dialogs.open.is_some()
  }

  fn dialog_next(&mut self) {
    // A fullscreen game, a Direct3D app or a presentation in front: the
    // question waits (as notification cards do) instead of covering it and
    // taking its focus with a forced foreground; it opens when that is over.
    if !self.dialogs.queue.is_empty() && fullscreen_busy() {
      unsafe { SetTimer(self.msg_hwnd, TIMER_DIALOG_WAIT, 2000, None) };
      return;
    }
    while let Some((spec, done)) = self.dialogs.queue.pop_front() {
      match self.dialog_show(spec) {
        Ok(open) => {
          let mut open = open;
          open.done = Some(done);
          self.dialogs.open = Some(open);
          return;
        }
        Err(err) => {
          tracing::warn!("Dialog: {:?}", err);
          done(self, Answer { button: None, checked: false });
        }
      }
    }
  }

  fn dialog_show(&mut self, spec: Spec) -> anyhow::Result<Open> {
    let (rect, scale) = target_monitor();
    let mon_w = (rect.right - rect.left) as f32 / scale;
    let parts = {
      let mut requests = Vec::new();
      let Ui { gfx, fonts, res, icons, .. } = self;
      let mut p = Painter { dc: &gfx.dc, gfx, fonts, res, icons, requests: &mut requests };
      measure(&mut p, &spec, mon_w)?
    };
    let prev_focus = unsafe { GetForegroundWindow() };
    let (hwnd, target, root, scrim) = make_window(&self.gfx, rect)?;
    let (pw, ph) = ((parts.size.0 * scale).ceil(), (parts.size.1 * scale).ceil());
    let made = (|| -> windows::core::Result<(Layer, (f32, f32))> {
      let panel = Layer::new(&self.gfx, pw as u32, ph as u32)?;
      let (w, h) = ((rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32);
      let at = (((w - pw) / 2.0).round(), ((h - ph) / 2.0).round().max(0.0));
      unsafe {
        panel.visual.SetOffsetX2(at.0)?;
        panel.visual.SetOffsetY2(at.1)?;
        root.AddVisual(&panel.visual, true, &scrim.visual)?;
      }
      Ok((panel, at))
    })();
    let (panel, panel_at) = match made {
      Ok(v) => v,
      Err(err) => {
        unsafe {
          let _ = DestroyWindow(hwnd);
        }
        return Err(err.into());
      }
    };
    let focus = Focus::Button(spec.default);
    let checked = spec.checked;
    let open = Open {
      hwnd,
      spec,
      done: None,
      focus,
      checked,
      hot: None,
      keys: false,
      scale,
      panel_at,
      parts,
      _target: target,
      _root: root,
      scrim,
      panel,
      prev_focus,
      closing: false,
    };
    // the scrim: rgba(0 0 0 / 45%), stretched
    gfx::draw_surface(&open.scrim.surface, 1.0, |dc| unsafe {
      let veil: D2D1_COLOR_F = Rgba(0, 0, 0, 0.45).into();
      dc.Clear(Some(&veil));
      Ok(())
    })?;
    self.dialog_paint_open(&open)?;
    self.dialog_enter(&open)?;
    DIALOG_HWND.store(hwnd.0 as isize, std::sync::atomic::Ordering::Release);
    unsafe {
      let _ = ShowWindow(hwnd, SW_SHOW);
    }
    take_foreground(hwnd);
    Ok(open)
  }

  fn dialog_paint_open(&mut self, o: &Open) -> anyhow::Result<()> {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, .. } = self;
    let mut requests = Vec::new();
    gfx::draw_surface(&o.panel.surface, o.scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = paint(&mut p, &theme, &o.spec, &o.parts, o.focus, o.hot, o.checked, o.keys) {
        tracing::warn!("Dialog: paint: {:?}", err);
      }
      Ok(())
    })?;
    unsafe { gfx.dcomp.Commit()? };
    Ok(())
  }

  fn dialog_repaint(&mut self) {
    let Some(o) = self.dialogs.open.take() else { return };
    if let Err(err) = self.dialog_paint_open(&o) {
      tracing::warn!("Dialog: {:?}", err);
    }
    self.dialogs.open = Some(o);
  }

  /// The scrim fades in (180 ms), the panel fades in and grows from 96 %
  /// (220 ms); nothing moves with animations off.
  fn dialog_enter(&self, o: &Open) -> windows::core::Result<()> {
    if !animations_on() {
      return Ok(());
    }
    let dcomp = &self.gfx.dcomp;
    unsafe {
      let s: IDCompositionVisual3 = o.scrim.visual.cast()?;
      s.SetOpacity(&anim::build(dcomp, 0.0, 1.0, 180.0, POP_IN)?)?;
      let v: IDCompositionVisual3 = o.panel.visual.cast()?;
      v.SetOpacity(&anim::build(dcomp, 0.0, 1.0, 220.0, POP_IN)?)?;
      let grow = dcomp.CreateScaleTransform()?;
      grow.SetCenterX2(o.parts.size.0 * o.scale / 2.0)?;
      grow.SetCenterY2(o.parts.size.1 * o.scale / 2.0)?;
      let size = anim::build(dcomp, 0.96, 1.0, 220.0, POP_IN)?;
      grow.SetScaleX(&size)?;
      grow.SetScaleY(&size)?;
      o.panel.visual.SetTransform(&grow)?;
      dcomp.Commit()
    }
  }

  /// Answers the open dialog (`button` None: closed without one), fades it
  /// out and opens the next one.
  pub(super) fn dialog_answer(&mut self, button: Option<usize>) {
    let Some(mut o) = self.dialogs.open.take() else { return };
    if o.closing {
      self.dialogs.open = Some(o);
      return;
    }
    o.closing = true;
    let answer = Answer { button, checked: o.checked };
    let done = o.done.take();
    let faded = animations_on() && self.dialog_fade_out(&o).is_ok();
    // the keyboard goes back where it was
    unsafe {
      if !o.prev_focus.is_invalid() && IsWindow(o.prev_focus).as_bool() {
        let _ = SetForegroundWindow(o.prev_focus);
      }
    }
    if faded {
      self.dialogs.open = Some(o);
      unsafe { SetTimer(self.msg_hwnd, TIMER_DIALOG_CLOSE, 130, None) };
    } else {
      Self::dialog_drop(o);
    }
    if let Some(done) = done {
      done(self, answer);
    }
    if !faded {
      self.dialog_next();
    }
  }

  fn dialog_fade_out(&self, o: &Open) -> windows::core::Result<()> {
    let dcomp = &self.gfx.dcomp;
    unsafe {
      for layer in [&o.scrim, &o.panel] {
        let v: IDCompositionVisual3 = layer.visual.cast()?;
        v.SetOpacity(&anim::build(dcomp, 1.0, 0.0, 120.0, POP_OUT)?)?;
      }
      dcomp.Commit()
    }
  }

  fn dialog_drop(o: Open) {
    DIALOG_HWND.store(0, std::sync::atomic::Ordering::Release);
    unsafe {
      let _ = DestroyWindow(o.hwnd);
    }
  }

  /// TIMER_DIALOG_CLOSE: the faded dialog goes, the next one opens.
  pub(super) fn dialog_closed(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_DIALOG_CLOSE);
    }
    if let Some(o) = self.dialogs.open.take_if(|o| o.closing) {
      Self::dialog_drop(o);
    }
    if self.dialogs.open.is_none() {
      self.dialog_next();
    }
  }

  /// The monitors changed or the bar is going away: every dialog is closed
  /// without an answer (their askers hear None).
  pub(super) fn dialog_destroy_all(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_DIALOG_CLOSE);
    }
    if let Some(mut o) = self.dialogs.open.take() {
      let done = o.done.take();
      let checked = o.checked;
      Self::dialog_drop(o);
      if let Some(done) = done {
        done(self, Answer { button: None, checked });
      }
    }
    while let Some((_, done)) = self.dialogs.queue.pop_front() {
      done(self, Answer { button: None, checked: false });
    }
  }

  fn dialog_press(&mut self, f: Focus) {
    match f {
      Focus::Check => {
        if let Some(o) = self.dialogs.open.as_mut() {
          o.checked = !o.checked;
        }
        self.dialog_repaint();
      }
      Focus::Button(i) => self.dialog_answer(Some(i)),
    }
  }

  /// Panel coordinates (DIPs) of a client point.
  fn dialog_point(o: &Open, lp: LPARAM) -> (f32, f32) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    ((x - o.panel_at.0) / o.scale, (y - o.panel_at.1) / o.scale)
  }

  fn dialog_hit(o: &Open, x: f32, y: f32) -> Option<Focus> {
    if let Some(r) = o.parts.check {
      if r.contains(x, y) {
        return Some(Focus::Check);
      }
    }
    o.parts.buttons.iter().position(|r| r.contains(x, y)).map(Focus::Button)
  }

  /// Messages of the dialog's window (None: not it).
  pub(super) fn dialog_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    let o = self.dialogs.open.as_ref()?;
    if o.hwnd != hwnd {
      return None;
    }
    if msg == WM_PAINT {
      unsafe {
        let _ = ValidateRect(hwnd, None);
      }
      return Some(Some(LRESULT(0)));
    }
    if o.closing {
      return Some(None);
    }
    match msg {
      WM_MOUSEMOVE => {
        let (x, y) = Self::dialog_point(o, lp);
        let hit = Self::dialog_hit(o, x, y);
        if hit != o.hot {
          if let Some(o) = self.dialogs.open.as_mut() {
            o.hot = hit;
          }
          self.dialog_repaint();
        }
        Some(Some(LRESULT(0)))
      }
      WM_SETCURSOR => {
        let hand = o.hot.is_some() && (lp.0 & 0xFFFF) as u32 == HTCLIENT;
        unsafe {
          if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
            SetCursor(c);
          }
        }
        Some(Some(LRESULT(1)))
      }
      WM_LBUTTONUP => {
        let (x, y) = Self::dialog_point(o, lp);
        if let Some(f) = Self::dialog_hit(o, x, y) {
          if let Some(o) = self.dialogs.open.as_mut() {
            o.focus = f;
            o.keys = false;
          }
          self.dialog_press(f);
        }
        // the scrim does nothing: the question waits for an answer
        Some(Some(LRESULT(0)))
      }
      WM_KEYDOWN | WM_SYSKEYDOWN => {
        let key = wp.0 as u16;
        let (spec, focus, cancel) = (o.spec.clone(), o.focus, o.spec.cancel);
        let shift = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let moved = if key == VK_TAB.0 {
          Some(next_focus(&spec, focus, shift))
        } else if key == VK_LEFT.0 || key == VK_UP.0 {
          Some(step_button(&spec, focus, -1))
        } else if key == VK_RIGHT.0 || key == VK_DOWN.0 {
          Some(step_button(&spec, focus, 1))
        } else {
          None
        };
        if let Some(f) = moved {
          if let Some(o) = self.dialogs.open.as_mut() {
            o.focus = f;
            o.keys = true;
          }
          self.dialog_repaint();
        } else if key == VK_RETURN.0 {
          // Enter on the checkbox presses the default button
          let f = if focus == Focus::Check { Focus::Button(spec.default) } else { focus };
          self.dialog_press(f);
        } else if key == VK_SPACE.0 {
          self.dialog_press(focus);
        } else if key == VK_ESCAPE.0 {
          if let Some(c) = cancel {
            self.dialog_answer(Some(c));
          }
        }
        Some(Some(LRESULT(0)))
      }
      _ => Some(None),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn reads_and_checks_the_cores_dialog() {
    let s = Spec::from_json(&json!({"kind":"question","title":"T","body":"B","buttons":["Evet","Hayır"],"default":0,"cancel":1,"check":"Bir daha sorma","checked":true})).unwrap();
    assert_eq!(s.kind, Kind::Question);
    assert_eq!(s.buttons, vec!["Evet", "Hayır"]);
    assert_eq!((s.default, s.cancel), (0, Some(1)));
    assert_eq!(s.check.as_deref(), Some("Bir daha sorma"));
    assert!(s.checked);
    // one button: Esc picks it
    let ok = Spec::from_json(&json!({"kind":"error","title":"x","buttons":["Tamam"]})).unwrap();
    assert_eq!(ok.cancel, Some(0));
    // out-of-range indexes fall back, a negative cancel means none
    let s = Spec::from_json(&json!({"title":"x","buttons":["a","b"],"default":7,"cancel":-1})).unwrap();
    assert_eq!((s.default, s.cancel), (0, None));
  }

  #[test]
  fn refuses_dialogs_that_cannot_be_answered() {
    assert!(Spec::from_json(&json!({"title":"x","buttons":[]})).is_none());
    assert!(Spec::from_json(&json!({"title":"x","buttons":["a","b","c","d"]})).is_none());
    assert!(Spec::from_json(&json!({"title":"","body":"","buttons":["a"]})).is_none());
    assert!(Spec::from_json(&json!({"kind":"shout","title":"x","buttons":["a"]})).is_none());
  }

  #[test]
  fn tab_goes_through_the_checkbox_and_the_buttons() {
    let s = Spec::new(Kind::Question, "t", "b", vec!["a".into(), "b".into()]).checkbox("c", false);
    assert_eq!(next_focus(&s, Focus::Check, false), Focus::Button(0));
    assert_eq!(next_focus(&s, Focus::Button(1), false), Focus::Check);
    assert_eq!(next_focus(&s, Focus::Check, true), Focus::Button(1));
    assert_eq!(step_button(&s, Focus::Button(1), 1), Focus::Button(1));
    assert_eq!(step_button(&s, Focus::Button(1), -1), Focus::Button(0));
    assert_eq!(step_button(&s, Focus::Check, 1), Focus::Button(0));
  }
}
