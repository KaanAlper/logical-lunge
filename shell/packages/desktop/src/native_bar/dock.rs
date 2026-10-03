//! The app Dock (Super+Alt; the web edition's dock.html): the Super menu's
//! button, the pinned apps and the running ones, a fisheye magnification
//! under the pointer, a right-click menu (the shared one) to keep an app
//! in the Dock.
//!
//! One window over the primary monitor, made when the Dock opens and
//! destroyed when it closes. A press outside the Dock, Esc, the bar or
//! another window taking the focus closes it. Pins live in the core
//! (`state\dock-pins.json`, `/dock-pins`, `/dock-pin`); the Super menu's
//! right-click menu writes there too and the core says `ll:dock-pins`.

use std::time::Instant;

use windows::{
  core::{w, Interface},
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      DirectComposition::{IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3},
      Gdi::{ClientToScreen, ValidateRect},
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
      Controls::WM_MOUSELEAVE,
      Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE},
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  anim::{self, Curve, POP_OUT},
  core_api,
  fonts::TextStyle,
  gfx::{self, Gfx, Rect, Rgba},
  menu::{Item as MenuItem, MenuFocus},
  view::{Align, Painter, Theme},
  Layer, Msg, Ui, CLASS, TIMER_DOCK_CLOSE, TIMER_DOCK_TICK,
};


/// dock.html: resting tile, gap, largest magnification, how far it spreads
/// (in tiles)
const TILE: f32 = 50.0;
const GAP: f32 = 6.0;
const MAGNIFY: f32 = 1.55;
const SPREAD: f32 = 1.1;
/// `.dock { height: 66px; padding: 0 10px 8px; border: 1px }`
const DOCK_H: f32 = 66.0;
const PAD_X: f32 = 10.0;
const PAD_B: f32 = 8.0;
const BORDER: f32 = 1.0;
const RADIUS: f32 = 22.0;
/// `.dock-screen { padding-bottom: 18px }`
const BOTTOM: f32 = 18.0;
/// `.items { padding: 0 2px }`, the divider's `margin: 0 3px`
const ITEMS_PAD: f32 = 2.0;
const DIVIDER_M: f32 = 3.0;
/// the drawn area: the Dock with room above it (magnified icons, the name,
/// the menu) and around it (shadow)
const SURFACE_H: f32 = 260.0;
const MAX_W: f32 = 1400.0;
const SHADOW: f32 = 48.0;
/// the window opened a moment ago: losing the focus now is Windows settling
const BLUR_GRACE_MS: u128 = 300;
/// `transition: width 130ms ease-out`: the magnification follows the pointer
/// with this time constant (about 130 ms to settle)
const EASE_MS: f32 = 40.0;
/// `transform 230ms cubic-bezier(.2, .8, .2, 1)`
const RISE: Curve = Curve(0.2, 0.8, 0.2, 1.0);

/// Exe name as the core keeps pins: lower case, without `.exe`.
fn key(s: &str) -> String {
  let s = s.to_lowercase();
  s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
}

#[derive(Clone, Debug, PartialEq)]
struct Win {
  id: String,
  workspace: String,
  focused: bool,
  handle: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct Item {
  id: String,
  name: String,
  /// index in the app list (pinned or matched by exe)
  app: Option<usize>,
  path: Option<String>,
  /// the running windows' process name and windows
  process: Option<String>,
  windows: Vec<Win>,
}

/// Pins first, then the running apps in the window manager's order; an
/// entry neither running nor in the app list is left out.
fn items(pins: &[String], running: &[(String, String, Vec<Win>)], catalog: &[(String, usize, String, String)]) -> Vec<Item> {
  let mut ids: Vec<String> = Vec::new();
  for id in pins.iter().cloned().chain(running.iter().map(|r| r.0.clone())) {
    if !ids.contains(&id) {
      ids.push(id);
    }
  }
  ids
    .into_iter()
    .filter_map(|id| {
      let run = running.iter().find(|r| r.0 == id);
      let app = catalog.iter().find(|c| c.0 == id);
      if run.is_none() && app.is_none() {
        return None;
      }
      Some(Item {
        name: app.map(|a| a.2.clone()).or_else(|| run.map(|r| r.1.clone())).unwrap_or_else(|| id.clone()),
        app: app.map(|a| a.1),
        path: app.map(|a| a.3.clone()),
        process: run.map(|r| r.1.clone()),
        windows: run.map(|r| r.2.clone()).unwrap_or_default(),
        id,
      })
    })
    .collect()
}

/// The resting size: with many apps the icons shrink to fit (36..50).
fn base_size(screen_w: f32, n: usize) -> f32 {
  let room = (screen_w * 0.92).min(MAX_W) - 120.0;
  (room / n.max(1) as f32 - GAP).clamp(36.0, TILE)
}

/// Magnification of tile `i` for a pointer at `pointer` (items box
/// coordinates), measured on the resting layout so growing icons do not
/// move the target.
fn scale_at(pointer: Option<f32>, i: usize, base: f32) -> f32 {
  let Some(x) = pointer else { return 1.0 };
  let d = (x - (i as f32 * (base + GAP) + base / 2.0)) / (base + GAP);
  1.0 + (MAGNIFY - 1.0) * (-(d * d) / (2.0 * SPREAD * SPREAD)).exp()
}

/// Where everything is, in the Dock's coordinates (its border box's top
/// left at 0, 0).
#[derive(Debug, PartialEq)]
struct Geo {
  w: f32,
  launcher: Rect,
  divider: Option<Rect>,
  /// the items box's left edge (with its padding)
  items_x: f32,
  tiles: Vec<Rect>,
}

fn geo(base: f32, scales: &[f32]) -> Geo {
  let bottom = DOCK_H - BORDER - PAD_B;
  let mut x = BORDER + PAD_X;
  let launcher = Rect::new(x, bottom - TILE, TILE, TILE);
  x += TILE + GAP;
  let divider = (!scales.is_empty()).then(|| {
    let content_mid = BORDER + (DOCK_H - 2.0 * BORDER - PAD_B) / 2.0;
    let r = Rect::new(x + DIVIDER_M, content_mid - 19.0, 1.0, 38.0);
    x += DIVIDER_M + 1.0 + DIVIDER_M + GAP;
    r
  });
  let items_x = x;
  x += ITEMS_PAD;
  let mut tiles = Vec::with_capacity(scales.len());
  for (i, s) in scales.iter().enumerate() {
    if i > 0 {
      x += GAP;
    }
    let size = base * s;
    tiles.push(Rect::new(x, bottom - size, size, size));
    x += size;
  }
  x += ITEMS_PAD + PAD_X + BORDER;
  Geo { w: x, launcher, divider, items_x, tiles }
}

/// What a point of the Dock window is over.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Spot {
  Launcher,
  Tile(usize),
  /// the Dock but nothing that acts
  Dock,
  /// outside the Dock: a press closes it
  Backdrop,
}

pub(super) struct Dock {
  hwnd: HWND,
  /// DIP to pixels
  scale: f32,
  /// the monitor's width in DIP
  screen_w: f32,
  /// the surface's left and top on the window (DIP)
  surface_x: f32,
  surface_y: f32,
  surface_w: f32,
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  layer: Layer,
  items: Vec<Item>,
  scales: Vec<f32>,
  pointer: Option<f32>,
  hot: Option<Spot>,
  /// its right-click menu is open (the shared menu takes the focus: the
  /// Dock stays open meanwhile)
  menu: bool,
  last_tick: Instant,
  ticking: bool,
  tracking: bool,
  active_at: Option<Instant>,
  closing: bool,
}

/// What the Dock keeps while it is closed: the pins (read again on open and
/// on `ll:dock-pins`).
#[derive(Default)]
pub(super) struct DockState {
  pins: Vec<String>,
  open: Option<Dock>,
}

impl Dock {
  /// The Dock's rectangle on the surface (DIP).
  fn rect(&self, g: &Geo) -> Rect {
    Rect::new((self.surface_w - g.w) / 2.0, SURFACE_H - BOTTOM - DOCK_H, g.w, DOCK_H)
  }

  fn base(&self) -> f32 {
    base_size(self.screen_w, self.items.len())
  }

  /// A client point (pixels) in surface DIP.
  fn point(&self, lp: LPARAM) -> (f32, f32) {
    let x = (lp.0 & 0xFFFF) as i16 as f32 / self.scale;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / self.scale;
    (x - self.surface_x, y - self.surface_y)
  }

  fn spot(&self, x: f32, y: f32) -> Spot {
    let g = geo(self.base(), &self.scales);
    let dock = self.rect(&g);
    let (dx, dy) = (x - dock.x, y - dock.y);
    if g.launcher.contains(dx, dy) {
      return Spot::Launcher;
    }
    if let Some(i) = g.tiles.iter().position(|t| t.contains(dx, dy)) {
      return Spot::Tile(i);
    }
    if dock.contains(x, y) {
      Spot::Dock
    } else {
      Spot::Backdrop
    }
  }

  /// The pointer in the items box's coordinates while it is over the Dock
  /// (or a magnified icon rising above it), else None.
  fn pointer_at(&self, x: f32, y: f32) -> Option<f32> {
    let g = geo(self.base(), &self.scales);
    let dock = self.rect(&g);
    let top = g.tiles.iter().map(|t| t.y).fold(0.0f32, f32::min);
    let over = x >= dock.x && x <= dock.right() && y >= dock.y + top.min(0.0) && y <= dock.bottom();
    over.then_some(x - dock.x - g.items_x)
  }
}

fn make_window(gfx: &Gfx, rect: RECT, dpi: u32) -> anyhow::Result<(HWND, IDCompositionTarget, IDCompositionVisual2, Layer, f32, f32, f32, f32, f32)> {
  let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
  let scale = crate::native_bar::scale::of_dpi(dpi);
  let screen_w = w as f32 / scale;
  let surface_w = screen_w.min(MAX_W + 2.0 * SHADOW);
  let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
  unsafe {
    // the core keeps "Logical Lunge ·" windows out of tiling and focus rules
    let hwnd = CreateWindowExW(ex, CLASS, w!("Logical Lunge · dock"), WS_POPUP, rect.left, rect.top, w, h, None, None, GetModuleHandleW(None)?, None)?;
    let made = (|| -> windows::core::Result<_> {
      let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let root = gfx.dcomp.CreateVisual()?;
      let layer = Layer::new(gfx, (surface_w * scale).ceil() as u32, (SURFACE_H * scale).ceil() as u32)?;
      let surface_x = ((screen_w - surface_w) / 2.0 * scale).round() / scale;
      let surface_y = (h as f32 / scale) - SURFACE_H;
      layer.visual.SetOffsetX2((surface_x * scale).round())?;
      layer.visual.SetOffsetY2((surface_y * scale).round())?;
      root.AddVisual(&layer.visual, false, None)?;
      target.SetRoot(&root)?;
      Ok((target, root, layer, surface_x, surface_y))
    })();
    match made {
      Ok((target, root, layer, sx, sy)) => Ok((hwnd, target, root, layer, scale, screen_w, sx, sy, surface_w)),
      Err(err) => {
        let _ = DestroyWindow(hwnd);
        Err(err.into())
      }
    }
  }
}

/// The query part of a URL: exe names are letters, digits and a few signs.
fn encode(s: &str) -> String {
  s.bytes()
    .map(|b| match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
      _ => format!("%{:02X}", b),
    })
    .collect()
}

fn mix(a: Rgba, b: Rgba, k: f32) -> Rgba {
  let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
  Rgba(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2), a.3 + (b.3 - a.3) * k)
}

impl Ui {
  /// `ll:dock-toggle` (Super+Alt)
  pub(super) fn dock_toggle(&mut self) {
    match &self.dock.open {
      Some(d) if !d.closing => self.dock_close(),
      _ => self.dock_open(),
    }
  }

  fn dock_open(&mut self) {
    self.dock_destroy();
    let monitors = super::monitor_layout();
    let Some(&(left, top, right, bottom, dpi)) = monitors.iter().find(|m| m.0 == 0 && m.1 == 0).or(monitors.first()) else {
      return;
    };
    let rect = RECT { left, top, right, bottom };
    let (hwnd, target, root, layer, scale, screen_w, surface_x, surface_y, surface_w) = match make_window(&self.gfx, rect, dpi) {
      Ok(w) => w,
      Err(err) => {
        tracing::warn!("Dock: window: {:?}", err);
        return;
      }
    };
    self.dock.open = Some(Dock {
      hwnd,
      scale,
      screen_w,
      surface_x,
      surface_y,
      surface_w,
      _target: target,
      _root: root,
      layer,
      items: Vec::new(),
      scales: Vec::new(),
      pointer: None,
      hot: None,
      menu: false,
      last_tick: Instant::now(),
      ticking: false,
      tracking: false,
      active_at: None,
      closing: false,
    });
    self.dock_items();
    if let Err(err) = self.dock_paint() {
      tracing::warn!("Dock: paint: {:?}", err);
      self.dock_destroy();
      return;
    }
    if let Err(err) = self.dock_enter() {
      tracing::debug!("Dock: entrance: {:?}", err);
    }
    unsafe {
      let _ = ShowWindow(hwnd, SW_SHOW);
      let _ = SetForegroundWindow(hwnd);
    }
    // the shell may not be allowed to take the foreground: the core raises
    // it (Esc needs the keyboard)
    std::thread::spawn(|| core_api::run_core(&["--raise", "Logical Lunge · dock"]));
    Self::dock_read_pins();
  }

  /// The pins from the core, off the UI thread.
  fn dock_read_pins() {
    std::thread::spawn(|| {
      let pins = core_api::post("/dock-pins")
        .filter(|(status, _)| *status == 200)
        .and_then(|(_, body)| serde_json::from_slice::<Vec<serde_json::Value>>(&body).ok())
        .map(|list| list.into_iter().filter_map(|v| v.as_str().map(str::to_string)).take(24).collect());
      super::send(Msg::DockPins(pins));
    });
  }

  /// A read of the pins answered (None: the core did not; read again
  /// after a failed write, kept otherwise).
  pub(super) fn dock_pins(&mut self, pins: Option<Vec<String>>) {
    let Some(p) = pins else { return };
    self.dock.pins = p;
    self.dock_refresh();
  }

  /// `ll:dock-pins` from the core, or a pin the core did not save.
  pub(super) fn dock_pins_changed(&mut self) {
    Self::dock_read_pins();
  }

  /// The window manager, the app list or an icon changed while it is open.
  pub(super) fn dock_refresh(&mut self) {
    if self.dock.open.as_ref().is_some_and(|d| !d.closing) {
      self.dock_items();
      if let Err(err) = self.dock_paint() {
        tracing::warn!("Dock: paint: {:?}", err);
      }
    }
  }

  fn dock_items(&mut self) {
    let mut running: Vec<(String, String, Vec<Win>)> = Vec::new();
    for m in &self.model.wm.monitors {
      for ws in &m.workspaces {
        for w in &ws.windows {
          let id = key(&w.process);
          if id.is_empty() {
            continue;
          }
          let win = Win { id: w.id.clone(), workspace: ws.name.clone(), focused: w.has_focus, handle: w.handle };
          match running.iter_mut().find(|r| r.0 == id) {
            Some(r) => r.2.push(win),
            None => running.push((id, w.process.clone(), vec![win])),
          }
        }
      }
    }
    let catalog: Vec<(String, usize, String, String)> = self
      .icons
      .apps()
      .iter()
      .enumerate()
      .filter_map(|(i, a)| {
        let exe = a.exe.as_deref().filter(|e| !e.is_empty())?;
        (!a.path.is_empty()).then(|| (key(exe), i, a.name.clone(), a.path.clone()))
      })
      .collect();
    let list = items(&self.dock.pins, &running, &catalog);
    let Some(d) = self.dock.open.as_mut() else { return };
    if d.items.iter().map(|i| &i.id).ne(list.iter().map(|i| &i.id)) {
      d.scales = vec![1.0; list.len()];
    }
    d.items = list;
  }

  fn dock_paint(&mut self) -> anyhow::Result<()> {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, dock, .. } = self;
    let Some(d) = dock.open.as_ref() else { return Ok(()) };
    let mut requests = Vec::new();
    gfx::draw_surface(&d.layer.surface, d.scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = paint(&mut p, &theme, d) {
        tracing::warn!("Dock: paint: {:?}", err);
      }
      Ok(())
    })?;
    unsafe { gfx.dcomp.Commit()? };
    self.ask_win_icons(requests);
    Ok(())
  }

  /// dock.html's entrance: fades in (180 ms) and rises 24 px from 96 %
  /// (230 ms); with animations off it is simply there.
  fn dock_enter(&self) -> windows::core::Result<()> {
    let Some(d) = &self.dock.open else { return Ok(()) };
    if !self.model.animations {
      return Ok(());
    }
    let dcomp = &self.gfx.dcomp;
    unsafe {
      let v: IDCompositionVisual3 = d.layer.visual.cast()?;
      v.SetOpacity(&anim::build(dcomp, 0.0, 1.0, 180.0, anim::LINEAR)?)?;
      let s = d.scale;
      let grow = dcomp.CreateScaleTransform()?;
      grow.SetCenterX2(d.surface_w * s / 2.0)?;
      grow.SetCenterY2((SURFACE_H - BOTTOM) * s)?;
      let size = anim::build(dcomp, 0.96, 1.0, 230.0, RISE)?;
      grow.SetScaleX(&size)?;
      grow.SetScaleY(&size)?;
      let rise = dcomp.CreateTranslateTransform()?;
      rise.SetOffsetY(&anim::build(dcomp, 24.0 * s, 0.0, 230.0, RISE)?)?;
      let moves = dcomp.CreateTransformGroup(&[Some(grow.cast()?), Some(rise.cast()?)])?;
      d.layer.visual.SetTransform(&moves)?;
      dcomp.Commit()
    }
  }

  /// Fades out (or, with animations off, goes at once).
  pub(super) fn dock_close(&mut self) {
    let menu = match self.dock.open.as_mut() {
      Some(d) if !d.closing => {
        d.closing = true;
        std::mem::take(&mut d.menu)
      }
      _ => return,
    };
    if menu && self.menu_is_open() {
      self.menu_close();
    }
    if !self.model.animations || self.dock_fade_out().is_err() {
      self.dock_destroy();
      return;
    }
    unsafe { SetTimer(self.msg_hwnd, TIMER_DOCK_CLOSE, 220, None) };
  }

  fn dock_fade_out(&self) -> windows::core::Result<()> {
    let Some(d) = &self.dock.open else { return Ok(()) };
    let dcomp = &self.gfx.dcomp;
    unsafe {
      let v: IDCompositionVisual3 = d.layer.visual.cast()?;
      v.SetOpacity(&anim::build(dcomp, 1.0, 0.0, 180.0, POP_OUT)?)?;
      dcomp.Commit()
    }
  }

  pub(super) fn dock_destroy(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_DOCK_CLOSE);
      let _ = KillTimer(self.msg_hwnd, TIMER_DOCK_TICK);
    }
    if let Some(d) = self.dock.open.take() {
      unsafe {
        let _ = DestroyWindow(d.hwnd);
      }
    }
  }

  /// One step of the magnification toward the pointer (TIMER_DOCK_TICK).
  pub(super) fn dock_tick(&mut self) {
    let Some(d) = self.dock.open.as_mut() else {
      unsafe {
        let _ = KillTimer(self.msg_hwnd, TIMER_DOCK_TICK);
      }
      return;
    };
    let now = Instant::now();
    let dt = now.duration_since(d.last_tick).as_secs_f32() * 1000.0;
    d.last_tick = now;
    let k = 1.0 - (-dt / EASE_MS).exp();
    let base = d.base();
    let mut moving = false;
    for i in 0..d.scales.len() {
      let target = scale_at(d.pointer, i, base);
      let s = &mut d.scales[i];
      *s += (target - *s) * k;
      if (target - *s).abs() < 0.003 {
        *s = target;
      } else {
        moving = true;
      }
    }
    if !moving {
      d.ticking = false;
      unsafe {
        let _ = KillTimer(self.msg_hwnd, TIMER_DOCK_TICK);
      }
    }
    if let Err(err) = self.dock_paint() {
      tracing::warn!("Dock: paint: {:?}", err);
    }
  }

  fn dock_animate(&mut self) {
    let Some(d) = self.dock.open.as_mut() else { return };
    if !self.model.animations {
      let base = d.base();
      for i in 0..d.scales.len() {
        d.scales[i] = scale_at(d.pointer, i, base);
      }
      return;
    }
    if !d.ticking {
      d.ticking = true;
      d.last_tick = Instant::now();
      unsafe { SetTimer(self.msg_hwnd, TIMER_DOCK_TICK, 16, None) };
    }
  }

  fn dock_activate(&mut self, item: Item) {
    self.dock_close();
    if !item.windows.is_empty() {
      let target = item.windows.iter().find(|w| w.focused).unwrap_or(&item.windows[0]).clone();
      let current = self.model.wm.focused_workspace().map(|w| w.name.clone());
      if current.as_deref() != Some(target.workspace.as_str()) {
        self.wm_command(format!("command focus --workspace {}", target.workspace));
      }
      self.wm_command(format!("command focus --container-id {}", target.id));
      return;
    }
    if let Some(path) = item.path {
      super::launch::open(path, "", super::launch::Verb::Open);
    }
  }

  fn dock_pin(&mut self, id: String) {
    let on = !self.dock.pins.contains(&id);
    if on {
      self.dock.pins.push(id.clone());
    } else {
      self.dock.pins.retain(|p| p != &id);
    }
    self.dock_refresh();
    std::thread::spawn(move || {
      let path = format!("/dock-pin?id={}&on={}", encode(&id), if on { 1 } else { 0 });
      if !matches!(core_api::post(&path), Some((204, _))) {
        tracing::warn!("Dock: pin {} not saved", id);
        // the core's list wins
        Ui::dock_read_pins();
      }
    });
  }

  /// A message for the Dock's window (None: not it).
  pub(super) fn dock_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    let menu_open = self.menu_is_open();
    let d = self.dock.open.as_mut().filter(|d| d.hwnd == hwnd)?;
    if msg == WM_PAINT {
      unsafe {
        let _ = ValidateRect(hwnd, None);
      }
      return Some(Some(LRESULT(0)));
    }
    if d.closing {
      return Some(None);
    }
    let (result, then) = match msg {
      WM_MOUSEMOVE => {
        if !d.tracking {
          let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
          unsafe {
            let _ = TrackMouseEvent(&mut tme);
          }
          d.tracking = true;
        }
        let (x, y) = d.point(lp);
        let pointer = d.pointer_at(x, y);
        let spot = d.spot(x, y);
        let hot_changed = d.hot != Some(spot);
        d.hot = Some(spot);
        let then = if pointer != d.pointer {
          d.pointer = pointer;
          Then::Magnify
        } else if hot_changed {
          Then::Paint
        } else {
          Then::Nothing
        };
        (Some(LRESULT(0)), then)
      }
      WM_MOUSELEAVE => {
        d.tracking = false;
        d.hot = None;
        let then = if d.pointer.take().is_some() { Then::Magnify } else { Then::Paint };
        (Some(LRESULT(0)), then)
      }
      WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
        let hand = matches!(d.hot, Some(Spot::Launcher | Spot::Tile(_)));
        unsafe {
          if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
            SetCursor(c);
          }
        }
        (Some(LRESULT(1)), Then::Nothing)
      }
      // any button outside the Dock closes it (the web backdrop's mousedown)
      WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
        let (x, y) = d.point(lp);
        let then = match (d.spot(x, y), msg) {
          (Spot::Backdrop, _) => Then::Close,
          (Spot::Launcher, WM_LBUTTONDOWN) => Then::Search,
          (Spot::Tile(i), WM_LBUTTONDOWN) => Then::Open(d.items[i].clone()),
          _ => Then::Nothing,
        };
        (Some(LRESULT(0)), then)
      }
      WM_RBUTTONUP => {
        let (x, y) = d.point(lp);
        let then = match d.spot(x, y) {
          Spot::Tile(i) => {
            let mut at = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
            unsafe {
              let _ = ClientToScreen(hwnd, &mut at);
            }
            Then::Menu(d.items[i].clone(), at)
          }
          _ => Then::Nothing,
        };
        (Some(LRESULT(0)), then)
      }
      WM_KEYDOWN | WM_SYSKEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => (Some(LRESULT(0)), Then::Close),
      WM_ACTIVATE => {
        let mut then = Then::Nothing;
        if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
          // another window took the focus: close (the bar, Alt+Tab, Super);
          // its own right-click menu takes it too, and gives it back
          if !(d.menu && menu_open) && d.active_at.is_some_and(|t| t.elapsed().as_millis() > BLUR_GRACE_MS) {
            then = Then::Close;
          }
        } else {
          d.active_at = Some(Instant::now());
          // back from its menu (it gives the focus back when it closes)
          if !menu_open {
            d.menu = false;
          }
        }
        (None, then)
      }
      WM_CLOSE => (Some(LRESULT(0)), Then::Close),
      _ => (None, Then::Nothing),
    };
    match then {
      Then::Nothing => {}
      Then::Paint => self.dock_after_input(),
      Then::Magnify => {
        self.dock_animate();
        self.dock_after_input();
      }
      Then::Close => self.dock_close(),
      Then::Search => {
        self.dock_close();
        self.toggle_overview_from_bar();
      }
      Then::Open(item) => self.dock_activate(item),
      Then::Menu(item, at) => self.dock_menu(item, at),
    }
    Some(result)
  }

  /// "Keep in Dock" / "Remove from Dock" for an app; keeping needs an app
  /// to start, a running app without one can only be removed.
  fn dock_menu(&mut self, item: Item, at: POINT) {
    let pinned = self.dock.pins.contains(&item.id);
    let label = self.model.tr(if pinned { "Dock’tan kaldır" } else { "Dock’ta tut" });
    let icon = if pinned { "keep_off" } else { "keep" };
    let entry = MenuItem::new("pin", Some(icon), label).enabled(pinned || item.app.is_some());
    if let Some(d) = self.dock.open.as_mut() {
      d.menu = true;
    }
    let id = item.id;
    self.menu_open(at, MenuFocus::Take, vec![entry], move |ui: &mut Ui, choice: &str| {
      if choice == "pin" {
        ui.dock_pin(id);
      }
    });
    if !self.menu_is_open() {
      if let Some(d) = self.dock.open.as_mut() {
        d.menu = false;
      }
    }
  }

  /// Repaints after a pointer move unless the magnification timer will.
  fn dock_after_input(&mut self) {
    if self.dock.open.as_ref().is_some_and(|d| d.ticking) {
      return;
    }
    if let Err(err) = self.dock_paint() {
      tracing::warn!("Dock: paint: {:?}", err);
    }
  }
}

/// The Dock on its surface (DIP; the Dock's bottom 18 above the surface's).
fn paint(p: &mut Painter, t: &Theme, d: &Dock) -> anyhow::Result<()> {
  let base = d.base();
  let g = geo(base, &d.scales);
  let dock = d.rect(&g);
  // box-shadow: 0 16px 48px rgb(0 0 0 / 38%)
  for k in 1..=10 {
    let s = k as f32 * 4.0;
    let r = Rect::new(dock.x - s, dock.y + 12.0 - s, dock.w + 2.0 * s, dock.h + 2.0 * s);
    p.fill_round(r, RADIUS + s, Rgba(0, 0, 0, 0.035))?;
  }
  p.fill_round(dock, RADIUS, t.layer0.alpha(0.78))?;
  p.stroke_round(dock, RADIUS, t.border.alpha(0.85), 1.0)?;
  // inset 0 1px rgb(255 255 255 / 6%)
  p.fill(Rect::new(dock.x + RADIUS, dock.y + 1.0, dock.w - 2.0 * RADIUS, 1.0), Rgba(255, 255, 255, 0.06))?;
  // the Super menu's button
  let l = Rect::new(dock.x + g.launcher.x, dock.y + g.launcher.y, g.launcher.w, g.launcher.h);
  let hot = d.hot == Some(Spot::Launcher);
  let bg = if hot { mix(t.primary_container, t.primary, 0.2) } else { t.primary_container };
  p.fill_round(l, 16.0, bg)?;
  p.icon("apps", l.x + l.w / 2.0, l.y + l.h / 2.0, TILE * 0.56, true, t.on_primary_container)?;
  if let Some(div) = g.divider {
    p.fill(Rect::new(dock.x + div.x, dock.y + div.y, div.w, div.h), t.on_surface_variant.alpha(0.28))?;
  }
  let under = d.pointer.map(|x| (x / (base + GAP)).floor()).filter(|i| *i >= 0.0).map(|i| i as usize);
  for (i, item) in d.items.iter().enumerate() {
    let tile = g.tiles[i];
    let r = Rect::new(dock.x + tile.x, dock.y + tile.y, tile.w, tile.h);
    let proc = item.process.clone().or_else(|| item.app.and_then(|a| p.icons.apps().get(a)).and_then(|a| a.exe.clone()));
    let mut bmp = proc.as_deref().and_then(|pr| p.icons.for_process(p.gfx, pr).1);
    if bmp.is_none() {
      bmp = item.app.and_then(|a| p.icons.app(p.gfx, a));
    }
    if bmp.is_none() {
      if let (Some(pr), Some(w)) = (proc.as_deref(), item.windows.first()) {
        let (b, ask) = p.icons.for_window(p.gfx, pr, w.handle);
        if let Some(h) = ask {
          p.requests.push(h);
        }
        bmp = b;
      }
    }
    match &bmp {
      // `.tile img { width: 82% }`
      Some(b) => {
        let s = r.w * 0.82;
        p.image(b, Rect::new(r.x + (r.w - s) / 2.0, r.y + (r.h - s) / 2.0, s, s));
      }
      None => p.icon("web_asset", r.x + r.w / 2.0, r.y + r.h / 2.0, r.w * 0.56, true, t.on_layer0)?,
    }
    if !item.windows.is_empty() {
      p.fill_circle(r.x + r.w / 2.0, r.bottom() + 4.0, 2.0, t.primary)?;
    }
    if under == Some(i) {
      let style = TextStyle { size: 12.0, weight: 450.0 };
      let tw = p.measure(&item.name, style)?;
      let (lw, lh) = (tw + 20.0, 23.0);
      let label = Rect::new(r.x + r.w / 2.0 - lw / 2.0, r.y - 8.0 - lh, lw, lh);
      for k in 1..=4 {
        let s = k as f32 * 3.0;
        p.fill_round(Rect::new(label.x - s, label.y + 4.0 - s, label.w + 2.0 * s, label.h + 2.0 * s), lh / 2.0 + s, Rgba(0, 0, 0, 0.05))?;
      }
      p.fill_round(label, lh / 2.0, t.surface_container_high.alpha(0.92))?;
      p.text(&item.name, Rect::new(label.x + 10.0, label.y, tw + 1.0, lh), style, t.on_layer0, Align::Left, false)?;
    }
  }
  Ok(())
}

/// What a Dock message leads to once its state is updated.
enum Then {
  Nothing,
  Paint,
  /// the pointer moved: the icons grow toward it
  Magnify,
  Close,
  /// the Super menu's button
  Search,
  Open(Item),
  /// right click on an app: the shared menu at this screen point
  Menu(Item, POINT),
}

#[cfg(test)]
mod tests {
  use super::*;

  fn win(ws: &str) -> Win {
    Win { id: "1".into(), workspace: ws.into(), focused: false, handle: 0 }
  }

  #[test]
  fn pins_come_first_then_running_apps_once() {
    let running = vec![("code".to_string(), "Code".to_string(), vec![win("1")]), ("zen".to_string(), "zen".to_string(), vec![win("2")])];
    let catalog = vec![("zen".to_string(), 0, "Zen".to_string(), "shell:AppsFolder\\zen".to_string()), ("mpv".to_string(), 1, "mpv".to_string(), "C:\\mpv.exe".to_string())];
    let list = items(&["zen".into(), "mpv".into(), "gone".into()], &running, &catalog);
    let ids: Vec<&str> = list.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["zen", "mpv", "code"], "a pin neither running nor installed is left out");
    assert_eq!(list[0].name, "Zen", "the app list's name wins");
    assert_eq!(list[2].name, "Code", "a running app without an entry shows its process");
    assert!(list[1].windows.is_empty());
  }

  #[test]
  fn pin_ids_are_escaped_for_the_query() {
    assert_eq!(encode("code"), "code");
    assert_eq!(encode("my app&on=1"), "my%20app%26on%3D1");
  }

  #[test]
  fn keys_are_lower_case_exe_names() {
    assert_eq!(key("Code.EXE"), "code");
    assert_eq!(key("WindowsTerminal"), "windowsterminal");
  }

  #[test]
  fn icons_shrink_to_fit_between_36_and_50() {
    assert_eq!(base_size(1920.0, 3), TILE);
    assert_eq!(base_size(1920.0, 200), 36.0);
    let b = base_size(1920.0, 30);
    assert!(b > 36.0 && b < TILE);
  }

  #[test]
  fn magnification_peaks_under_the_pointer() {
    let base = TILE;
    let at = 2.0 * (base + GAP) + base / 2.0;
    assert!((scale_at(Some(at), 2, base) - MAGNIFY).abs() < 1e-4);
    assert!(scale_at(Some(at), 1, base) < MAGNIFY && scale_at(Some(at), 1, base) > 1.0);
    assert!(scale_at(Some(at), 6, base) < 1.01);
    assert_eq!(scale_at(None, 2, base), 1.0);
  }

  #[test]
  fn layout_follows_the_web_dock() {
    let g = geo(TILE, &[1.0, 1.0]);
    assert_eq!(g.launcher, Rect::new(11.0, 7.0, 50.0, 50.0));
    assert_eq!(g.items_x, 80.0);
    assert_eq!(g.tiles[0].x, 82.0);
    assert_eq!(g.tiles[1].x, 82.0 + 56.0);
    assert_eq!(g.w, 82.0 + 106.0 + 2.0 + 10.0 + 1.0);
    let empty = geo(TILE, &[]);
    assert!(empty.divider.is_none());
    // a magnified tile rises above the Dock's top, its bottom stays
    let big = geo(TILE, &[MAGNIFY]);
    assert_eq!(big.tiles[0].bottom(), 57.0);
    assert!(big.tiles[0].y < 7.0);
  }
}
