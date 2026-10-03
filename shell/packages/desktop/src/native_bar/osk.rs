//! The on-screen keyboard:
//! a Turkish Q keyboard that slides up from the bottom of the primary
//! monitor and types into the focused window.
//!
//! The window never takes the keyboard (`WS_EX_NOACTIVATE`, and the bar's
//! window procedure answers `MA_NOACTIVATE`), so a click types into the app
//! the user was writing in. Keys go out with `SendInput`, as the core's
//! `lunge.exe --osk` helper sent them for the web keyboard: characters as
//! Unicode, shortcuts and special keys as virtual keys with the latched
//! modifiers held around them. Made when it opens, destroyed when it
//! closes; the title is the one the core keeps above workspace slides.

use windows::{
  core::{w, Interface},
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{
      DirectComposition::{IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3},
      Gdi::ValidateRect,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
      Input::KeyboardAndMouse::{
        ReleaseCapture, SendInput, SetCapture, TrackMouseEvent, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, TME_LEAVE, TRACKMOUSEEVENT,
        VIRTUAL_KEY,
      },
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  anim::{self, POP_IN, POP_OUT},
  fonts::TextStyle,
  gfx::{self, Gfx, Rect, Rgba},
  view::{Align, Painter, Theme},
  Layer, Ui, CLASS, TIMER_OSK_CLOSE,
};

/// The core keeps this window above workspace slides and never lets it
/// take the focus (`Names.Osk`).
const TITLE: windows::core::PCWSTR = w!("Logical Lunge · osk");

/// osk.html `.key`: 45 px keys, 5 px apart; the function row is 30 px high
const KEY: f32 = 45.0;
const GAP: f32 = 5.0;
const FN_H: f32 = 30.0;
/// `.osk { padding: 10px; border-radius: 18px }`
const PAD: f32 = 10.0;
const RADIUS: f32 = 18.0;
/// `.side button`: 40 px, rounding 17
const SIDE: f32 = 40.0;
/// the rows start after the side buttons, a gap, the 1 px divider and a gap
const ROWS_X: f32 = PAD + SIDE + GAP + 1.0 + GAP;
/// room around the panel for its shadow; `.osk { bottom: 10px }` below it
const SHADOW: f32 = 18.0;
const BOTTOM: f32 = 10.0;
/// the slide out, then the window goes
const CLOSE_MS: u32 = 250;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Toggle {
  Shift,
  Caps,
  Ctrl,
  Alt,
  Win,
  AltGr,
}

#[derive(Clone, Copy, Debug)]
enum Face {
  /// a character: normal / with Shift
  Char(&'static str, &'static str),
  Text(&'static str),
  Icon(&'static str),
  Toggle(&'static str, Toggle),
}

#[derive(Clone, Copy, Debug)]
struct Key {
  face: Face,
  vk: u16,
  /// width in keys (`45 * w + 5 * (w - 1)` px)
  w: f32,
}

const fn ch(t: &'static str, s: &'static str, vk: u16) -> Key {
  Key { face: Face::Char(t, s), vk, w: 1.0 }
}
const fn tx(label: &'static str, vk: u16, w: f32) -> Key {
  Key { face: Face::Text(label), vk, w }
}
const fn ic(name: &'static str, vk: u16, w: f32) -> Key {
  Key { face: Face::Icon(name), vk, w }
}
const fn tg(label: &'static str, t: Toggle, vk: u16, w: f32) -> Key {
  Key { face: Face::Toggle(label, t), vk, w }
}

/// The Turkish Q layout, row by row (the first is the function row).
const ROWS: [&[Key]; 6] = [
  &[
    tx("Esc", 27, 1.0), tx("F1", 112, 1.0), tx("F2", 113, 1.0), tx("F3", 114, 1.0), tx("F4", 115, 1.0),
    tx("F5", 116, 1.0), tx("F6", 117, 1.0), tx("F7", 118, 1.0), tx("F8", 119, 1.0), tx("F9", 120, 1.0),
    tx("F10", 121, 1.0), tx("F11", 122, 1.0), tx("F12", 123, 1.0), tx("Del", 46, 1.0),
  ],
  &[
    ch("\"", "é", 192), ch("1", "!", 49), ch("2", "'", 50), ch("3", "^", 51), ch("4", "+", 52), ch("5", "%", 53),
    ch("6", "&", 54), ch("7", "/", 55), ch("8", "(", 56), ch("9", ")", 57), ch("0", "=", 48), ch("*", "?", 223),
    ch("-", "_", 189), ic("backspace", 8, 2.0),
  ],
  &[
    tx("Tab", 9, 1.5), ch("q", "Q", 81), ch("w", "W", 87), ch("e", "E", 69), ch("r", "R", 82), ch("t", "T", 84),
    ch("y", "Y", 89), ch("u", "U", 85), ch("ı", "I", 73), ch("o", "O", 79), ch("p", "P", 80), ch("ğ", "Ğ", 219),
    ch("ü", "Ü", 221), ch(",", ";", 188),
  ],
  &[
    tg("Caps", Toggle::Caps, 20, 1.8), ch("a", "A", 65), ch("s", "S", 83), ch("d", "D", 68), ch("f", "F", 70),
    ch("g", "G", 71), ch("h", "H", 72), ch("j", "J", 74), ch("k", "K", 75), ch("l", "L", 76), ch("ş", "Ş", 186),
    ch("i", "İ", 222), ic("keyboard_return", 13, 1.8),
  ],
  &[
    tg("Shift", Toggle::Shift, 16, 1.3), ch("<", ">", 226), ch("z", "Z", 90), ch("x", "X", 88), ch("c", "C", 67),
    ch("v", "V", 86), ch("b", "B", 66), ch("n", "N", 78), ch("m", "M", 77), ch("ö", "Ö", 191), ch("ç", "Ç", 220),
    ch(".", ":", 190), tg("Shift", Toggle::Shift, 16, 2.2),
  ],
  &[
    tg("Ctrl", Toggle::Ctrl, 17, 1.3), tg("Win", Toggle::Win, 91, 1.3), tg("Alt", Toggle::Alt, 18, 1.3),
    tx(" ", 32, 6.4), tg("AltGr", Toggle::AltGr, 165, 1.3), ic("arrow_back", 37, 1.0), ic("arrow_upward", 38, 1.0),
    ic("arrow_downward", 40, 1.0), ic("arrow_forward", 39, 1.0),
  ],
];

fn key_w(k: &Key) -> f32 {
  KEY * k.w + GAP * (k.w - 1.0)
}

fn row_h(row: usize) -> f32 {
  if row == 0 { FN_H } else { KEY }
}

fn panel_w() -> f32 {
  let widest = ROWS
    .iter()
    .map(|r| r.iter().map(key_w).sum::<f32>() + GAP * (r.len() as f32 - 1.0))
    .fold(0.0, f32::max);
  ROWS_X + widest + PAD
}

fn panel_h() -> f32 {
  PAD + (0..ROWS.len()).map(row_h).sum::<f32>() + GAP * (ROWS.len() as f32 - 1.0) + PAD
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
  Key(usize, usize),
  Pin,
  Close,
}

/// Every key's rectangle in panel coordinates (DIP from its top left).
fn key_rects() -> Vec<(Hit, Rect)> {
  let mut out = Vec::new();
  let mut y = PAD;
  for (r, row) in ROWS.iter().enumerate() {
    let mut x = ROWS_X;
    for (c, k) in row.iter().enumerate() {
      let w = key_w(k);
      out.push((Hit::Key(r, c), Rect::new(x, y, w, row_h(r))));
      x += w + GAP;
    }
    y += row_h(r) + GAP;
  }
  out.push((Hit::Pin, Rect::new(PAD, PAD, SIDE, SIDE)));
  out.push((Hit::Close, Rect::new(PAD, PAD + SIDE + GAP, SIDE, SIDE)));
  out
}

fn hit_at(x: f32, y: f32) -> Option<Hit> {
  key_rects().into_iter().find(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(h, _)| h)
}

/// Shift, Caps Lock and the latched modifiers (one press each).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct Mods {
  shift: bool,
  caps: bool,
  ctrl: bool,
  alt: bool,
  win: bool,
  altgr: bool,
}

impl Mods {
  fn on(&self, t: Toggle) -> bool {
    match t {
      Toggle::Shift => self.shift,
      Toggle::Caps => self.caps,
      Toggle::Ctrl => self.ctrl,
      Toggle::Alt => self.alt,
      Toggle::Win => self.win,
      Toggle::AltGr => self.altgr,
    }
  }

  fn flip(&mut self, t: Toggle) {
    let v = !self.on(t);
    match t {
      Toggle::Shift => self.shift = v,
      Toggle::Caps => self.caps = v,
      Toggle::Ctrl => self.ctrl = v,
      Toggle::Alt => self.alt = v,
      Toggle::Win => self.win = v,
      Toggle::AltGr => self.altgr = v,
    }
  }

  /// What a character key shows and types: Shift gives the second
  /// character; Caps Lock does so for letters only.
  fn char_of(&self, t: &'static str, s: &'static str) -> &'static str {
    let letter = t.chars().next().is_some_and(char::is_alphabetic);
    if self.shift || (letter && self.caps) { s } else { t }
  }

  /// The latched modifiers' keys in the order they go down (Shift last).
  fn held(&self) -> Vec<u16> {
    let mut v = Vec::new();
    for (on, vk) in [(self.ctrl, 17), (self.alt, 18), (self.win, 91), (self.altgr, 165), (self.shift, 16)] {
      if on {
        v.push(vk);
      }
    }
    v
  }
}

/// What a press sends: text, or virtual keys (down / tap / up).
#[derive(Clone, PartialEq, Debug)]
enum Out {
  Text(String),
  Down(u16),
  Tap(u16),
  Up(u16),
}

/// A key press: what goes out and how the modifiers are afterwards
/// (osk.html `press`).
fn press(mods: &mut Mods, k: &Key) -> Vec<Out> {
  if let Face::Toggle(_, t) = k.face {
    mods.flip(t);
    return Vec::new();
  }
  let any_mod = mods.ctrl || mods.alt || mods.win || mods.altgr;
  let out = match k.face {
    Face::Char(t, s) if !any_mod => vec![Out::Text(mods.char_of(t, s).to_string())],
    _ => {
      // a shortcut or a special key: the held modifiers around its key
      let held = mods.held();
      let mut v: Vec<Out> = held.iter().map(|&m| Out::Down(m)).collect();
      v.push(Out::Tap(k.vk));
      v.extend(held.iter().rev().map(|&m| Out::Up(m)));
      mods.ctrl = false;
      mods.alt = false;
      mods.win = false;
      mods.altgr = false;
      v
    }
  };
  mods.shift = false; // one Shift per key
  out
}

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
  INPUT {
    r#type: INPUT_KEYBOARD,
    Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
  }
}

/// Arrows, Home / End, Insert / Delete, Page keys, Win and AltGr are
/// extended keys.
fn extended(vk: u16) -> bool {
  (0x21..=0x2E).contains(&vk) || vk == 0x5B || vk == 0xA5
}

fn send(out: &[Out]) {
  let mut inputs = Vec::new();
  for o in out {
    match o {
      Out::Text(s) => {
        for u in s.encode_utf16() {
          inputs.push(key_input(0, u, KEYEVENTF_UNICODE));
          inputs.push(key_input(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
      }
      Out::Down(vk) | Out::Tap(vk) | Out::Up(vk) => {
        let ext = if extended(*vk) { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
        if !matches!(o, Out::Up(_)) {
          inputs.push(key_input(*vk, 0, ext));
        }
        if !matches!(o, Out::Down(_)) {
          inputs.push(key_input(*vk, 0, ext | KEYEVENTF_KEYUP));
        }
      }
    }
  }
  if inputs.is_empty() {
    return;
  }
  let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
  if sent as usize != inputs.len() {
    tracing::warn!("Keyboard: {} of {} key events went out", sent, inputs.len());
  }
}

fn paint(p: &mut Painter, t: &Theme, mods: &Mods, pinned: bool, hover: Option<Hit>, down: Option<Hit>) -> anyhow::Result<()> {
  let (pw, ph) = (panel_w(), panel_h());
  let panel = Rect::new(SHADOW, SHADOW, pw, ph);
  // box-shadow: 0 4px 18px rgba(0 0 0 / 45%)
  for k in 1..=6 {
    let g = k as f32 * 2.5;
    p.fill_round(Rect::new(panel.x - g, panel.y + 4.0 - g, panel.w + 2.0 * g, panel.h + 2.0 * g), RADIUS + g, Rgba(0, 0, 0, 0.06))?;
  }
  p.fill_round(panel, RADIUS, t.layer0)?;
  p.stroke_round(panel.inset(0.5, 0.5), RADIUS, t.border, 1.0)?;
  // `.divider { margin: 20px 0 }`
  let dx = panel.x + PAD + SIDE + GAP;
  p.fill(Rect::new(dx, panel.y + 20.0, 1.0, ph - 40.0), t.outline_variant)?;
  for (hit, r) in key_rects() {
    let r = Rect::new(panel.x + r.x, panel.y + r.y, r.w, r.h);
    let pressed = down == Some(hit) && hover == Some(hit);
    match hit {
      Hit::Pin | Hit::Close => {
        let on = hit == Hit::Pin && pinned;
        let (bg, fg) = if on { (t.primary, t.on_primary) } else { (if hover == Some(hit) { t.layer1_hover } else { t.layer1 }, t.on_layer1) };
        // `.side button:active { height: 50px }`
        let r = if pressed { Rect::new(r.x, r.y, r.w, 50.0) } else { r };
        p.fill_round(r, 17.0, bg)?;
        let name = if hit == Hit::Pin { "keep" } else { "keyboard_hide" };
        p.icon(name, r.x + r.w / 2.0, r.y + r.h / 2.0, 19.0, false, fg)?;
      }
      Hit::Key(row, col) => {
        let k = &ROWS[row][col];
        let on = matches!(k.face, Face::Toggle(_, tog) if mods.on(tog));
        let bg = if on { t.primary } else if hover == Some(hit) { t.layer1_hover } else { t.layer1 };
        let fg = if on { t.on_primary } else { t.on_layer1 };
        // `.key:active { transform: scaleY(1.12); border-radius: 17px }`
        let (r, radius) = if pressed {
          let grow = r.h * 0.06;
          (Rect::new(r.x, r.y - grow, r.w, r.h + 2.0 * grow), 17.0)
        } else {
          (r, 12.0)
        };
        p.fill_round(r, radius, bg)?;
        let size = if row == 0 { 13.0 } else { 17.0 };
        let style = TextStyle { size, weight: 450.0 };
        match k.face {
          Face::Icon(name) => p.icon(name, r.x + r.w / 2.0, r.y + r.h / 2.0, 22.0, false, fg)?,
          Face::Char(a, b) => {
            p.text(mods.char_of(a, b), r, style, fg, Align::Center, false)?;
          }
          Face::Text(label) | Face::Toggle(label, _) => {
            p.text(label, r, style, fg, Align::Center, false)?;
          }
        }
      }
    }
  }
  Ok(())
}

pub(super) struct Osk {
  pub hwnd: HWND,
  scale: f32,
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  panel: Layer,
  mods: Mods,
  /// osk.html's pin button (shown as on; nothing closes the keyboard by itself)
  pinned: bool,
  hover: Option<Hit>,
  /// the button the mouse went down on: it acts if the mouse comes up on it
  down: Option<Hit>,
  tracking: bool,
  /// sliding out: destroyed by TIMER_OSK_CLOSE
  closing: bool,
}

impl Osk {
  /// Panel coordinates (DIP) of a client point.
  fn point(&self, lp: LPARAM) -> (f32, f32) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    (x / self.scale - SHADOW, y / self.scale - SHADOW)
  }
}

/// The primary monitor (at 0, 0) and its DPI; else the first one.
fn primary_monitor() -> Option<(RECT, u32)> {
  let all = super::monitor_layout();
  let pick = all.iter().find(|m| m.0 == 0 && m.1 == 0).or(all.first())?;
  Some((RECT { left: pick.0, top: pick.1, right: pick.2, bottom: pick.3 }, pick.4))
}

fn make(gfx: &Gfx) -> anyhow::Result<Osk> {
  let (mon, dpi) = primary_monitor().ok_or_else(|| anyhow::anyhow!("no monitor"))?;
  // the interface scale can make it wider than the monitor: then it shrinks as one
  let scale = crate::native_bar::scale::fit_scale(crate::native_bar::scale::of_dpi(dpi), panel_w() + 2.0 * SHADOW, mon.right - mon.left);
  let w = ((panel_w() + 2.0 * SHADOW) * scale).ceil() as i32;
  let h = ((panel_h() + SHADOW + BOTTOM) * scale).ceil() as i32;
  let x = mon.left + (mon.right - mon.left - w) / 2;
  let y = mon.bottom - h;
  let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
  unsafe {
    let hwnd = CreateWindowExW(ex, CLASS, TITLE, WS_POPUP, x, y, w, h, None, None, GetModuleHandleW(None)?, None)?;
    let made = (|| -> windows::core::Result<Osk> {
      let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let root = gfx.dcomp.CreateVisual()?;
      let panel = Layer::new(gfx, w as u32, h as u32)?;
      root.AddVisual(&panel.visual, false, None)?;
      target.SetRoot(&root)?;
      Ok(Osk {
        hwnd,
        scale,
        _target: target,
        _root: root,
        panel,
        mods: Mods::default(),
        pinned: false,
        hover: None,
        down: None,
        tracking: false,
        closing: false,
      })
    })();
    made.map_err(|err| {
      let _ = DestroyWindow(hwnd);
      err.into()
    })
  }
}

impl Ui {
  /// `ll:osk-toggle`: the bar's keyboard button, the sidebar's tile.
  pub(super) fn osk_toggle(&mut self) {
    match &self.osk {
      Some(o) if !o.closing => self.osk_close(),
      _ => self.osk_open(),
    }
  }

  fn osk_open(&mut self) {
    self.osk_destroy();
    match make(&self.gfx) {
      Ok(o) => self.osk = Some(o),
      Err(err) => {
        tracing::warn!("Keyboard: window: {:?}", err);
        return;
      }
    }
    if let Err(err) = self.osk_paint() {
      tracing::warn!("Keyboard: paint: {:?}", err);
      self.osk_destroy();
      return;
    }
    if let Err(err) = self.osk_slide(true) {
      tracing::debug!("Keyboard: entrance: {:?}", err);
    }
    let Some(o) = &self.osk else { return };
    unsafe {
      let _ = ShowWindow(o.hwnd, SW_SHOWNOACTIVATE);
      let _ = SetWindowPos(o.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
  }

  pub(super) fn osk_paint(&mut self) -> anyhow::Result<()> {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, osk, .. } = self;
    let Some(o) = osk.as_ref() else { return Ok(()) };
    let mut requests = Vec::new();
    gfx::draw_surface(&o.panel.surface, o.scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = paint(&mut p, &theme, &o.mods, o.pinned, o.hover, o.down) {
        tracing::warn!("Keyboard: paint: {:?}", err);
      }
      Ok(())
    })?;
    unsafe { gfx.dcomp.Commit()? };
    Ok(())
  }

  /// osk.html's slide: in from below (320 ms, --emphDecel) with a 200 ms
  /// fade; out (240 ms, --emphAccel). With animations off it is cut.
  fn osk_slide(&self, open: bool) -> windows::core::Result<()> {
    let Some(o) = &self.osk else { return Ok(()) };
    if !self.model.animations {
      return Ok(());
    }
    let dcomp = &self.gfx.dcomp;
    let away = (panel_h() + 20.0 + BOTTOM) * o.scale;
    unsafe {
      let v: IDCompositionVisual3 = o.panel.visual.cast()?;
      if open {
        v.SetOpacity(&anim::build(dcomp, 0.0, 1.0, 200.0, POP_IN)?)?;
        o.panel.visual.SetOffsetY(&anim::build(dcomp, away, 0.0, 320.0, POP_IN)?)?;
      } else {
        v.SetOpacity(&anim::build(dcomp, 1.0, 0.0, 240.0, POP_OUT)?)?;
        o.panel.visual.SetOffsetY(&anim::build(dcomp, 0.0, away, 240.0, POP_OUT)?)?;
      }
      dcomp.Commit()
    }
  }

  pub(super) fn osk_close(&mut self) {
    match self.osk.as_mut() {
      Some(o) if !o.closing => o.closing = true,
      _ => return,
    }
    if !self.model.animations || self.osk_slide(false).is_err() {
      self.osk_destroy();
      return;
    }
    unsafe { SetTimer(self.msg_hwnd, TIMER_OSK_CLOSE, CLOSE_MS, None) };
  }

  pub(super) fn osk_destroy(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_OSK_CLOSE);
    }
    if let Some(o) = self.osk.take() {
      unsafe {
        let _ = DestroyWindow(o.hwnd);
      }
    }
  }

  /// A message for the keyboard's window (None: not it).
  pub(super) fn osk_msg(&mut self, hwnd: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    let o = self.osk.as_mut()?;
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
    let mut repaint = false;
    let mut act = None;
    let result = match msg {
      WM_MOUSEMOVE => {
        if !o.tracking {
          let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
          };
          o.tracking = unsafe { TrackMouseEvent(&mut tme) }.is_ok();
        }
        let (x, y) = o.point(lp);
        let hit = hit_at(x, y);
        if hit != o.hover {
          o.hover = hit;
          repaint = true;
        }
        Some(LRESULT(0))
      }
      0x02A3 /* WM_MOUSELEAVE */ => {
        o.tracking = false;
        if o.hover.is_some() {
          o.hover = None;
          repaint = true;
        }
        Some(LRESULT(0))
      }
      WM_SETCURSOR if o.hover.is_some() && (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
        unsafe {
          if let Ok(hand) = LoadCursorW(None, IDC_HAND) {
            SetCursor(hand);
          }
        }
        Some(LRESULT(1))
      }
      WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
        let (x, y) = o.point(lp);
        o.down = hit_at(x, y);
        if o.down.is_some() {
          unsafe { SetCapture(hwnd) };
          repaint = true;
        }
        Some(LRESULT(0))
      }
      WM_LBUTTONUP => {
        let (x, y) = o.point(lp);
        let up = hit_at(x, y);
        if let Some(d) = o.down.take() {
          unsafe {
            let _ = ReleaseCapture();
          }
          if up == Some(d) {
            act = Some(d);
          }
          repaint = true;
        }
        Some(LRESULT(0))
      }
      WM_CAPTURECHANGED => {
        if o.down.take().is_some() {
          repaint = true;
        }
        Some(LRESULT(0))
      }
      WM_CLOSE => {
        act = Some(Hit::Close);
        Some(LRESULT(0))
      }
      _ => None,
    };
    match act {
      Some(Hit::Close) => {
        self.osk_close();
        return Some(result);
      }
      Some(Hit::Pin) => o.pinned = !o.pinned,
      Some(Hit::Key(r, c)) => {
        let out = press(&mut o.mods, &ROWS[r][c]);
        send(&out);
      }
      None => {}
    }
    if repaint || act.is_some() {
      if let Err(err) = self.osk_paint() {
        tracing::warn!("Keyboard: paint: {:?}", err);
      }
    }
    Some(result)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn find(label: &str) -> &'static Key {
    ROWS
      .iter()
      .flat_map(|r| r.iter())
      .find(|k| match k.face {
        Face::Char(t, _) => t == label,
        Face::Text(l) | Face::Icon(l) | Face::Toggle(l, _) => l == label,
      })
      .unwrap()
  }

  #[test]
  fn the_panel_matches_the_web_keyboard() {
    // the widest rows (Caps … Enter, the bottom row) are 775 px
    assert_eq!(panel_w(), ROWS_X + 775.0 + PAD);
    assert_eq!(panel_h(), PAD + FN_H + 5.0 * KEY + 5.0 * GAP + PAD);
  }

  #[test]
  fn keys_are_hit_where_they_are_drawn() {
    assert_eq!(hit_at(ROWS_X + 1.0, PAD + 1.0), Some(Hit::Key(0, 0)));
    assert_eq!(hit_at(ROWS_X + KEY + GAP / 2.0, PAD + 1.0), None, "the gap between keys");
    assert_eq!(hit_at(PAD + 1.0, PAD + 1.0), Some(Hit::Pin));
    assert_eq!(hit_at(PAD + 1.0, PAD + SIDE + GAP + 1.0), Some(Hit::Close));
    let backspace = ROWS_X + 13.0 * (KEY + GAP) + 1.0;
    assert_eq!(hit_at(backspace, PAD + FN_H + GAP + 1.0), Some(Hit::Key(1, 13)));
  }

  #[test]
  fn shift_types_once_and_caps_only_changes_letters() {
    let mut m = Mods::default();
    press(&mut m, find("Shift"));
    assert_eq!(press(&mut m, find("ı")), vec![Out::Text("I".into())]);
    assert_eq!(press(&mut m, find("ı")), vec![Out::Text("ı".into())], "Shift is gone after one key");
    press(&mut m, find("Caps"));
    assert_eq!(press(&mut m, find("i")), vec![Out::Text("İ".into())]);
    assert_eq!(press(&mut m, find("1")), vec![Out::Text("1".into())], "Caps leaves digits alone");
  }

  #[test]
  fn a_latched_modifier_makes_a_shortcut() {
    let mut m = Mods::default();
    press(&mut m, find("Ctrl"));
    press(&mut m, find("Shift"));
    assert_eq!(press(&mut m, find("c")), vec![Out::Down(17), Out::Down(16), Out::Tap(67), Out::Up(16), Out::Up(17)]);
    assert_eq!(m, Mods::default(), "the modifiers are released after the shortcut");
  }

  #[test]
  fn special_keys_go_out_as_keys() {
    let mut m = Mods::default();
    assert_eq!(press(&mut m, find("backspace")), vec![Out::Tap(8)]);
    assert!(extended(37) && extended(91) && extended(165) && !extended(8));
  }
}
