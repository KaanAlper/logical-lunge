//! The session screen:
//! lock, sleep, reload the desktop, sign out, restart, firmware settings,
//! shut down.
//!
//! One window per monitor, made when it opens and destroyed when it closes
//! (nothing stays in memory between uses). Each has three visuals: the
//! radial scrim, drawn on a 32 px surface the compositor stretches over the
//! monitor; the row of buttons; the "Esc" hint. The primary monitor's window
//! takes the keyboard (arrows / Tab, Enter / Space, Esc); the others only
//! the mouse.

use std::{os::windows::process::CommandExt, sync::atomic::Ordering, time::Instant};

use windows::{
  core::{w, Interface},
  Foundation::Numerics::Matrix3x2,
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
      Direct2D::{
        Common::D2D1_GRADIENT_STOP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
        D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_COLOR_SPACE_SRGB, D2D1_EXTEND_MODE_CLAMP,
        D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES,
      },
      DirectComposition::{
        IDCompositionSurface, IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3,
        DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR, DCOMPOSITION_BORDER_MODE_HARD,
      },
      Gdi::ValidateRect,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
      Input::KeyboardAndMouse::{VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB},
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  anim::{self, POP_IN, POP_OUT},
  core_api,
  fonts::TextStyle,
  gfx::{self, pt, Gfx, Rect, Rgba},
  view::{Align, Painter, Theme},
  Layer, Ui, CLASS, SESSION_PRIMARY, TIMER_SESSION_CLOSE,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// session.html `.sbtn` / `#row`
const BTN: f32 = 116.0;
const GAP: f32 = 16.0;
const PAD: f32 = 22.0;
const ROW_R: f32 = 34.0;
const ROW_W: f32 = 2.0 * PAD + 7.0 * BTN + 6.0 * GAP;
const ROW_H: f32 = BTN + 2.0 * PAD;
/// room around the row for its shadow
const SHADOW: f32 = 48.0;
/// the scrim's surface (stretched over the monitor)
const SCRIM_PX: u32 = 32;
const HINT_W: f32 = 480.0;
const HINT_H: f32 = 22.0;
/// `#hint { bottom: 34px }`
const HINT_BOTTOM: f32 = 34.0;
/// focus that leaves this soon after the menu got it is Windows settling,
/// not the user (the web menu's 300 ms)
const BLUR_GRACE_MS: u128 = 300;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
  Lock,
  Sleep,
  Reload,
  SignOut,
  Restart,
  Firmware,
  ShutDown,
}

struct Item {
  icon: &'static str,
  label: &'static str,
  danger: bool,
  act: Act,
}

const ITEMS: [Item; 7] = [
  Item { icon: "lock", label: "Kilitle", danger: false, act: Act::Lock },
  Item { icon: "dark_mode", label: "Uyku", danger: false, act: Act::Sleep },
  // a part that hangs or closed: the window manager, shell and core start clean
  Item { icon: "refresh", label: "Masaüstünü yenile", danger: false, act: Act::Reload },
  Item { icon: "logout", label: "Çıkış", danger: false, act: Act::SignOut },
  Item { icon: "restart_alt", label: "Yeniden başlat", danger: false, act: Act::Restart },
  Item { icon: "developer_board", label: "UEFI / BIOS", danger: false, act: Act::Firmware },
  Item { icon: "power_settings_new", label: "Kapat", danger: true, act: Act::ShutDown },
];

fn run(act: Act) {
  let (program, args): (&str, &[&str]) = match act {
    Act::Lock => ("rundll32", &["user32.dll,LockWorkStation"]),
    Act::Sleep => ("rundll32", &["powrprof.dll,SetSuspendState", "0,1,0"]),
    Act::Reload => {
      core_api::run_core(&["--restart-desktop"]);
      return;
    }
    Act::SignOut => ("shutdown", &["/l"]),
    Act::Restart => ("shutdown", &["/r", "/t", "0"]),
    Act::Firmware => ("shutdown", &["/r", "/fw", "/t", "0"]),
    Act::ShutDown => ("shutdown", &["/s", "/t", "0"]),
  };
  if let Err(err) = std::process::Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).spawn() {
    tracing::warn!("Session: {:?}: {:?}", act, err);
  }
}

/// Which button is at `(x, y)` in row coordinates (DIP from the row's
/// top left, without the shadow); a selected button sits 4 DIP higher.
fn button_at(x: f32, y: f32) -> Option<usize> {
  if y < PAD - 4.0 || y > PAD + BTN || x < PAD {
    return None;
  }
  let i = ((x - PAD) / (BTN + GAP)) as usize;
  let left = PAD + i as f32 * (BTN + GAP);
  (i < ITEMS.len() && x <= left + BTN).then_some(i)
}

/// The row's shadow, background and buttons, drawn at (SHADOW, SHADOW).
fn paint_row(p: &mut Painter, t: &Theme, sel: usize, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
  let row = Rect::new(SHADOW, SHADOW, ROW_W, ROW_H);
  // box-shadow: 0 30px 90px rgba(0 0 0 / 65%), 0 6px 18px rgba(0 0 0 / 45%)
  for k in 1..=12 {
    let g = k as f32 * 3.0;
    let r = Rect::new(row.x - g, row.y + 10.0 - g, row.w + 2.0 * g, row.h + 2.0 * g);
    p.fill_round(r, ROW_R + g, Rgba(0, 0, 0, 0.04))?;
  }
  p.fill_round(row, ROW_R, t.layer0)?;
  p.stroke_round(row.inset(0.5, 0.5), ROW_R, t.border, 1.0)?;
  for (i, item) in ITEMS.iter().enumerate() {
    let on = i == sel;
    let x = row.x + PAD + i as f32 * (BTN + GAP);
    let y = row.y + PAD - if on { 4.0 } else { 0.0 };
    let (bg, fg) = match (on, item.danger) {
      (true, true) => (t.error, Rgba::hex(0x690005)),
      (true, false) => (t.primary, t.on_primary),
      _ => (t.sec_container, t.on_sec_container),
    };
    p.fill_round(Rect::new(x, y, BTN, BTN), if on { 44.0 } else { 30.0 }, bg)?;
    // a column: 34 px icon, 10 px gap, the label
    p.icon(item.icon, x + BTN / 2.0, y + 44.0, 34.0, true, fg)?;
    let style = TextStyle { size: 14.0, weight: 450.0 };
    p.text(&tr(item.label), Rect::new(x + 4.0, y + 71.0, BTN - 8.0, 20.0), style, fg, Align::Center, false)?;
  }
  Ok(())
}

pub(super) struct Win {
  pub hwnd: HWND,
  primary: bool,
  /// the monitor's DPI scale (scrim, hint)
  scale: f32,
  /// the row's: smaller than `scale` on a monitor too narrow for it
  row_scale: f32,
  /// the row surface's top left in the window (pixels)
  row_at: (f32, f32),
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  scrim: Layer,
  row: Layer,
  hint: Layer,
}

impl Win {
  /// Row coordinates (DIP) of a client point.
  fn row_point(&self, lp: LPARAM) -> (f32, f32) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    ((x - self.row_at.0) / self.row_scale - SHADOW, (y - self.row_at.1) / self.row_scale - SHADOW)
  }
}

pub(super) struct Session {
  pub wins: Vec<Win>,
  sel: usize,
  /// the pointer is over a button (hand cursor)
  hot: bool,
  /// when the primary window got the keyboard
  active_at: Option<Instant>,
  /// fading out: destroyed by TIMER_SESSION_CLOSE
  closing: bool,
}

fn make_win(gfx: &Gfx, rect: RECT, dpi: u32, primary: bool) -> anyhow::Result<Win> {
  let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
  let scale = crate::native_bar::scale::of_dpi(dpi);
  let fit = ((w as f32 / scale - 32.0) / (ROW_W + 2.0 * SHADOW)).clamp(0.4, 1.0);
  let row_scale = scale * fit;
  let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | if primary { WINDOW_EX_STYLE(0) } else { WS_EX_NOACTIVATE };
  unsafe {
    let hwnd = CreateWindowExW(ex, CLASS, w!("lunge-session"), WS_POPUP, rect.left, rect.top, w, h, None, None, GetModuleHandleW(None)?, None)?;
    let made = (|| -> windows::core::Result<Win> {
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
      let (rw, rh) = (((ROW_W + 2.0 * SHADOW) * row_scale).ceil(), ((ROW_H + 2.0 * SHADOW) * row_scale).ceil());
      let row = Layer::new(gfx, rw as u32, rh as u32)?;
      let row_at = (((w as f32 - rw) / 2.0).round(), ((h as f32 - rh) / 2.0).round());
      row.visual.SetOffsetX2(row_at.0)?;
      row.visual.SetOffsetY2(row_at.1)?;
      let (hw, hh) = ((HINT_W * scale).ceil(), (HINT_H * scale).ceil());
      let hint = Layer::new(gfx, hw as u32, hh as u32)?;
      hint.visual.SetOffsetX2(((w as f32 - hw) / 2.0).round())?;
      hint.visual.SetOffsetY2((h as f32 - (HINT_BOTTOM + HINT_H) * scale).round())?;
      root.AddVisual(&scrim.visual, false, None)?;
      root.AddVisual(&row.visual, false, None)?;
      root.AddVisual(&hint.visual, false, None)?;
      target.SetRoot(&root)?;
      Ok(Win { hwnd, primary, scale, row_scale, row_at, _target: target, _root: root, scrim, row, hint })
    })();
    made.map_err(|err| {
      let _ = DestroyWindow(hwnd);
      err.into()
    })
  }
}

/// `radial-gradient(ellipse at center, rgba(8 6 12 / 62%), rgba(4 3 8 / 82%))`:
/// the ellipse reaches the corners, so it stretches with the surface.
fn paint_scrim(surface: &IDCompositionSurface) -> windows::core::Result<()> {
  gfx::draw_surface(surface, 1.0, |dc| unsafe {
    let stops = [
      D2D1_GRADIENT_STOP { position: 0.0, color: Rgba(8, 6, 12, 0.62).into() },
      D2D1_GRADIENT_STOP { position: 1.0, color: Rgba(4, 3, 8, 0.82).into() },
    ];
    let collection = dc.CreateGradientStopCollection(
      &stops,
      D2D1_COLOR_SPACE_SRGB,
      D2D1_COLOR_SPACE_SRGB,
      D2D1_BUFFER_PRECISION_8BPC_UNORM,
      D2D1_EXTEND_MODE_CLAMP,
      D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
    )?;
    let half = SCRIM_PX as f32 / 2.0;
    let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
      center: pt(half, half),
      gradientOriginOffset: pt(0.0, 0.0),
      radiusX: half * std::f32::consts::SQRT_2,
      radiusY: half * std::f32::consts::SQRT_2,
    };
    let brush = dc.CreateRadialGradientBrush(&props, None, &collection)?;
    dc.FillRectangle(&Rect::new(0.0, 0.0, SCRIM_PX as f32, SCRIM_PX as f32).d2d(), &brush);
    Ok(())
  })
}

impl Ui {
  /// `ll:session-toggle` (the sidebar's session button)
  pub(super) fn session_toggle(&mut self) {
    match &self.session {
      Some(s) if !s.closing => self.session_close(),
      _ => self.session_open(),
    }
  }

  fn session_open(&mut self) {
    self.session_destroy();
    let mut wins = Vec::new();
    for (left, top, right, bottom, dpi) in super::monitor_layout() {
      let rect = RECT { left, top, right, bottom };
      match make_win(&self.gfx, rect, dpi, left == 0 && top == 0) {
        Ok(w) => wins.push(w),
        Err(err) => tracing::warn!("Session: window: {:?}", err),
      }
    }
    if wins.is_empty() {
      return;
    }
    // no monitor at (0, 0) (an unusual layout): the first one takes the keyboard
    if !wins.iter().any(|w| w.primary) {
      wins[0].primary = true;
    }
    if let Some(p) = wins.iter().find(|w| w.primary) {
      SESSION_PRIMARY.store(p.hwnd.0 as isize, Ordering::Release);
    }
    self.session = Some(Session { wins, sel: 0, hot: false, active_at: None, closing: false });
    if let Err(err) = self.session_paint(true) {
      tracing::warn!("Session: paint: {:?}", err);
      self.session_destroy();
      return;
    }
    if let Err(err) = self.session_enter() {
      tracing::debug!("Session: entrance: {:?}", err);
    }
    let Some(s) = &self.session else { return };
    unsafe {
      for w in &s.wins {
        let _ = ShowWindow(w.hwnd, if w.primary { SW_SHOW } else { SW_SHOWNOACTIVATE });
      }
      if let Some(p) = s.wins.iter().find(|w| w.primary) {
        let _ = SetForegroundWindow(p.hwnd);
      }
    }
    tracing::info!("Session: opened on {} monitor(s)", s.wins.len());
  }

  /// Draws the rows (and, the first time, the scrims and hints).
  fn session_paint(&mut self, all: bool) -> anyhow::Result<()> {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, model, session, .. } = self;
    let Some(s) = session.as_ref() else { return Ok(()) };
    let tr = |t: &str| model.tr(t);
    let hint = tr("Esc ile kapat");
    let mut requests = Vec::new();
    for w in &s.wins {
      gfx::draw_surface(&w.row.surface, w.row_scale, |dc| {
        let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
        if let Err(err) = paint_row(&mut p, &theme, s.sel, &tr) {
          tracing::warn!("Session: row: {:?}", err);
        }
        Ok(())
      })?;
      if all {
        paint_scrim(&w.scrim.surface)?;
        gfx::draw_surface(&w.hint.surface, w.scale, |dc| {
          let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
          let style = TextStyle { size: 13.0, weight: 450.0 };
          if let Err(err) = p.text(&hint, Rect::new(0.0, 0.0, HINT_W, HINT_H), style, Rgba(255, 255, 255, 0.55), Align::Center, false) {
            tracing::warn!("Session: hint: {:?}", err);
          }
          Ok(())
        })?;
      }
    }
    unsafe { gfx.dcomp.Commit()? };
    Ok(())
  }

  /// session.html's entrance: the scrim fades in (240 ms), the row rises
  /// 24 px and grows from 94 % (380 ms, --emphDecel). Played by the
  /// compositor; with animations off everything is simply there.
  fn session_enter(&self) -> windows::core::Result<()> {
    let Some(s) = &self.session else { return Ok(()) };
    if !self.model.animations {
      return Ok(());
    }
    let dcomp = &self.gfx.dcomp;
    unsafe {
      for w in &s.wins {
        for layer in [&w.scrim, &w.row, &w.hint] {
          let v: IDCompositionVisual3 = layer.visual.cast()?;
          v.SetOpacity(&anim::build(dcomp, 0.0, 1.0, 240.0, POP_IN)?)?;
        }
        let (rw, rh) = ((ROW_W + 2.0 * SHADOW) * w.row_scale, (ROW_H + 2.0 * SHADOW) * w.row_scale);
        let grow = dcomp.CreateScaleTransform()?;
        grow.SetCenterX2(rw / 2.0)?;
        grow.SetCenterY2(rh / 2.0)?;
        let size = anim::build(dcomp, 0.94, 1.0, 380.0, POP_IN)?;
        grow.SetScaleX(&size)?;
        grow.SetScaleY(&size)?;
        let rise = dcomp.CreateTranslateTransform()?;
        rise.SetOffsetY(&anim::build(dcomp, 24.0 * w.row_scale, 0.0, 380.0, POP_IN)?)?;
        let moves = dcomp.CreateTransformGroup(&[Some(grow.cast()?), Some(rise.cast()?)])?;
        w.row.visual.SetTransform(&moves)?;
      }
      dcomp.Commit()
    }
  }

  /// Fades out (or, with animations off, goes at once).
  pub(super) fn session_close(&mut self) {
    match self.session.as_mut() {
      Some(s) if !s.closing => s.closing = true,
      _ => return,
    }
    if !self.model.animations || self.session_fade_out().is_err() {
      self.session_destroy();
      return;
    }
    unsafe { SetTimer(self.msg_hwnd, TIMER_SESSION_CLOSE, 150, None) };
  }

  fn session_fade_out(&self) -> windows::core::Result<()> {
    let Some(s) = &self.session else { return Ok(()) };
    let dcomp = &self.gfx.dcomp;
    unsafe {
      for w in &s.wins {
        for layer in [&w.scrim, &w.row, &w.hint] {
          let v: IDCompositionVisual3 = layer.visual.cast()?;
          v.SetOpacity(&anim::build(dcomp, 1.0, 0.0, 140.0, POP_OUT)?)?;
        }
      }
      dcomp.Commit()
    }
  }

  pub(super) fn session_destroy(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SESSION_CLOSE);
    }
    SESSION_PRIMARY.store(0, Ordering::Release);
    if let Some(s) = self.session.take() {
      for w in s.wins {
        unsafe {
          let _ = DestroyWindow(w.hwnd);
        }
      }
    }
  }

  /// A message for one of the session windows (None: not one of them).
  pub(super) fn session_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    let s = self.session.as_mut()?;
    let i = s.wins.iter().position(|w| w.hwnd == hwnd)?;
    if msg == WM_PAINT {
      unsafe {
        let _ = ValidateRect(hwnd, None);
      }
      return Some(Some(LRESULT(0)));
    }
    if s.closing {
      return Some(None);
    }
    let n = ITEMS.len();
    let (result, then) = match msg {
      WM_MOUSEMOVE => {
        let (x, y) = s.wins[i].row_point(lp);
        let hit = button_at(x, y);
        s.hot = hit.is_some();
        match hit.filter(|b| *b != s.sel) {
          Some(b) => {
            s.sel = b;
            (Some(LRESULT(0)), Then::Paint)
          }
          None => (Some(LRESULT(0)), Then::Nothing),
        }
      }
      WM_SETCURSOR if s.hot && (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
        unsafe {
          if let Ok(hand) = LoadCursorW(None, IDC_HAND) {
            SetCursor(hand);
          }
        }
        (Some(LRESULT(1)), Then::Nothing)
      }
      WM_LBUTTONDOWN => {
        let (x, y) = s.wins[i].row_point(lp);
        let then = match button_at(x, y) {
          Some(b) => Then::Run(ITEMS[b].act),
          // the scrim closes; the row's padding does nothing
          None if !(0.0..=ROW_W).contains(&x) || !(0.0..=ROW_H).contains(&y) => Then::Close,
          None => Then::Nothing,
        };
        (Some(LRESULT(0)), then)
      }
      WM_KEYDOWN | WM_SYSKEYDOWN => {
        let key = wp.0 as u16;
        if key == VK_ESCAPE.0 {
          (Some(LRESULT(0)), Then::Close)
        } else if key == VK_RIGHT.0 || key == VK_TAB.0 {
          s.sel = (s.sel + 1) % n;
          (Some(LRESULT(0)), Then::Paint)
        } else if key == VK_LEFT.0 {
          s.sel = (s.sel + n - 1) % n;
          (Some(LRESULT(0)), Then::Paint)
        } else if key == VK_RETURN.0 || key == VK_SPACE.0 {
          // no letter keys: a stray S or R used to shut down or restart
          (Some(LRESULT(0)), Then::Run(ITEMS[s.sel].act))
        } else if msg == WM_SYSKEYDOWN {
          (None, Then::Nothing) // Alt+F4 comes back as WM_CLOSE
        } else {
          (Some(LRESULT(0)), Then::Nothing)
        }
      }
      WM_ACTIVATE => {
        let mut then = Then::Nothing;
        if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
          // focus went elsewhere after the menu had it: close (the web menu's blur)
          if s.wins[i].primary && s.active_at.is_some_and(|t| t.elapsed().as_millis() > BLUR_GRACE_MS) {
            then = Then::Close;
          }
        } else {
          s.active_at = Some(Instant::now());
        }
        (None, then)
      }
      WM_CLOSE => (Some(LRESULT(0)), Then::Close),
      _ => (None, Then::Nothing),
    };
    match then {
      Then::Nothing => {}
      Then::Paint => {
        if let Err(err) = self.session_paint(false) {
          tracing::warn!("Session: paint: {:?}", err);
        }
      }
      Then::Close => self.session_close(),
      Then::Run(act) => {
        self.session_close();
        run(act);
      }
    }
    Some(result)
  }
}

/// What a session window's message leads to once its state is updated.
enum Then {
  Nothing,
  Paint,
  Close,
  Run(Act),
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn buttons_are_found_in_row_coordinates() {
    assert_eq!(button_at(PAD + 1.0, PAD + 1.0), Some(0));
    assert_eq!(button_at(PAD + BTN + GAP / 2.0, PAD + 10.0), None, "the gap between buttons");
    assert_eq!(button_at(PAD + 6.0 * (BTN + GAP) + BTN - 1.0, PAD + BTN - 1.0), Some(6));
    assert_eq!(button_at(PAD + 7.0 * (BTN + GAP), PAD + 10.0), None, "past the last one");
    assert_eq!(button_at(PAD + 1.0, PAD - 3.0), Some(0), "a raised button reaches 4 DIP higher");
    assert_eq!(button_at(1.0, 1.0), None, "the row's padding");
  }

  #[test]
  fn the_row_holds_seven_actions_with_one_dangerous() {
    assert_eq!(ITEMS.len(), 7);
    assert_eq!(ITEMS.iter().filter(|i| i.danger).count(), 1);
    assert_eq!(ITEMS[ITEMS.len() - 1].act, Act::ShutDown);
  }
}
