//! The right panel: the
//! uptime and the buttons on top (issue report, update check, wallpapers,
//! shortcuts, settings, session), the quick settings, the notifications, the
//! calendar / to-do / timer group, and its pages sliding over it (shortcut
//! editor, wallpapers, issue report).
//!
//! One window on the primary monitor's right edge ("Logical Lunge ·
//! sidebar-right": the core keeps it above workspace slides), made when the
//! panel opens and destroyed when it has slid out. It takes the keyboard
//! while open and closes when the focus goes elsewhere, on Esc, on a click
//! outside the shell or on the bar, and when the Super menu opens. Two
//! surfaces: the panel and the page over it (each slides in the
//! compositor); the rest moves by redrawing while something animates.

mod bottom;
mod bug;
pub(super) mod images;
mod input;
mod panel;
mod keys;
mod kit;
mod notifs;
mod quick;
mod store;
mod text;
mod walls;

use std::{
  collections::HashMap,
  sync::atomic::Ordering,
  time::{Duration, Instant},
};

use serde_json::Value;
use windows::{
  core::{Interface, HSTRING},
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      Direct2D::{ID2D1Factory, ID2D1StrokeStyle, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_DASH, D2D1_STROKE_STYLE_PROPERTIES},
      DirectComposition::{IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3},
      Gdi::{ScreenToClient, ValidateRect},
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
      Controls::WM_MOUSELEAVE,
      Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_MENU, VK_SHIFT},
      WindowsAndMessaging::*,
    },
  },
};

use self::{
  panel::{paint_main, paint_page},
  bottom::{BHit, Bottom},
  bug::{Bug, BugEv, BugHit},
  images::{Images, Pixels},
  keys::{KEv, KHit, Keys},
  kit::{st, stw, Cx, Region},
  notifs::{NHit, Notifs},
  quick::{QEv, QHit, Quick},
  store::Store,
  text::{TextField, Typed},
  walls::{WEv, WHit, Walls},
};
use super::{
  anim::{self, POP_IN, POP_OUT},
  core_api,
  gfx::{self, Gfx, Rect},
  menu::MenuFocus,
  toast,
  view::Painter,
  Layer, Ui, CLASS, SIDEBAR_HWND, TIMER_SB_CLOSE, TIMER_SB_FRAME, TIMER_SB_PAGE, TIMER_SB_TICK, TIMER_SB_WHEEL,
};

/// The core finds the panel by this title (above workspace slides, `--raise`).
pub(super) const TITLE: &str = "Logical Lunge · sidebar-right";
/// sidebar.css: a 460 DIP widget, the panel 5 DIP inside it
const WIN_W: f32 = 460.0;
const INSET: f32 = 5.0;
const PANEL_R: f32 = 19.0;
/// focus that leaves this soon after opening is Windows settling
const BLUR_GRACE: Duration = Duration::from_millis(300);
/// Opened again within this time, the panel comes back as it was left: the
/// page that was open, its scroll, what was typed (it closes whenever the
/// focus goes elsewhere, e.g. to look something up for an issue report).
const RESUME: Duration = Duration::from_secs(180);
/// illogical-impulse's sidebars are layers sliding from the right (`layerrule animation slide right`) with
/// Hyprland's layer animations: layersIn 2.7 emphasizedDecel, fadeLayersIn 0.5 menu_decel; layersOut 2.4
/// menu_accel, fadeLayersOut 2.7 stall (one unit = 100 ms)
const OPEN_MS: f32 = 270.0;
const CLOSE_MS: f32 = 240.0;
const FADE_IN_MS: f32 = 50.0;
const FADE_OUT_MS: f32 = 270.0;
const PAGE_IN_MS: f32 = 340.0;
const PAGE_OUT_MS: f32 = 260.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum FieldId {
  KeysSearch,
  /// the shortcut editor's app picker
  KeysApp,
  Todo,
  WifiPw,
  NightFrom,
  NightTo,
  SaverMinutes,
  BugStart,
  BugEnd,
  BugText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ScrollId {
  Notifs,
  Todo,
  Card,
  Card2,
  Page,
  /// the shortcut editor's app picker
  KeysApps,
  /// a horizontal gallery row or chip row of a page
  Row(u8),
}

/// The top row's buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sys {
  Bug,
  Update,
  Walls,
  Keys,
  Settings,
  Session,
}

#[derive(Clone, Debug, PartialEq)]
enum Hit {
  /// the panel itself: takes the click, does nothing
  Panel,
  Sys(Sys),
  Quick(QHit),
  Notif(NHit),
  Bottom(BHit),
  /// a page's back arrow
  Back,
  Keys(KHit),
  Walls(WHit),
  Bug(BugHit),
  Field(FieldId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Page {
  Keys,
  Walls,
  Bug,
}

/// Messages for the panel (from the shell's events and from workers).
pub(super) enum Ev {
  Toggle,
  /// open with a page (`ll:sidebar-open-page`: "keys", "walls", "bug")
  OpenPage(String),
  /// the core's `ll:notifications`: the list changed
  NotifsChanged,
  Notifs(Value),
  Quick(QEv),
  Image(String, Option<Vec<(Pixels, u32)>>),
  Keys(KEv),
  Walls(WEv),
  Bug(BugEv),
}

struct Win {
  hwnd: HWND,
  scale: f32,
  /// DIPs
  h: f32,
  _target: IDCompositionTarget,
  root: IDCompositionVisual2,
  panel: Layer,
  /// a lifted quick settings tile: moved by the compositor, never repainted
  /// while it follows the pointer
  ghost: Layer,
  ghost_scale: windows::Win32::Graphics::DirectComposition::IDCompositionScaleTransform,
  page: Layer,
}

impl Drop for Win {
  fn drop(&mut self) {
    SIDEBAR_HWND.store(0, Ordering::Release);
    unsafe {
      let _ = DestroyWindow(self.hwnd);
    }
  }
}

/// A pointer drag under way.
#[derive(Clone, Debug, PartialEq)]
enum Drag {
  Field(FieldId),
  Slider(Hit),
  Tile,
  Notif,
  Page(Hit),
  /// a sideways row followed the pointer (`moved`) or may (a press on it)
  Row { id: ScrollId, x0: f32, off0: f32, moved: bool },
}

pub(super) struct Sidebar {
  store: Store,
  win: Option<Win>,
  pub open: bool,
  page: Option<Page>,
  page_closing: bool,
  focus: Option<FieldId>,
  fields: HashMap<FieldId, TextField>,
  hover: Option<Hit>,
  pressed: Option<Hit>,
  drag: Option<Drag>,
  mouse: Option<(f32, f32)>,
  tracking: bool,
  hits: Vec<(Rect, Hit)>,
  regions: Vec<Region>,
  scroll: HashMap<ScrollId, f32>,
  frames: bool,
  spin0: Instant,
  quick: Quick,
  notifs: Notifs,
  bottom: Bottom,
  keys: Keys,
  walls: Walls,
  bug: Option<Bug>,
  /// the page open when the panel last closed, and when
  resume: Option<(Page, Instant)>,
  images: Images,
  /// a file dialog of the core is open: losing the focus to it does not close
  modal: u32,
  shown_at: Option<Instant>,
  dash: Option<ID2D1StrokeStyle>,
  /// where the lifted tile's visual is (pixels): its landing starts there
  ghost_at: (f32, f32),
}

impl Default for Sidebar {
  fn default() -> Self {
    let store = Store::load();
    let quick = Quick::new(store.toggles(), &store.qs_cache);
    Sidebar {
      win: None,
      open: false,
      page: None,
      page_closing: false,
      focus: None,
      fields: HashMap::new(),
      hover: None,
      pressed: None,
      drag: None,
      mouse: None,
      tracking: false,
      hits: Vec::new(),
      regions: Vec::new(),
      scroll: HashMap::new(),
      frames: false,
      spin0: Instant::now(),
      quick,
      notifs: Notifs::default(),
      bottom: Bottom::default(),
      keys: Keys::default(),
      walls: Walls::new(&store),
      bug: None,
      resume: None,
      images: Images::default(),
      modal: 0,
      shown_at: None,
      dash: None,
      ghost_at: (0.0, 0.0),
      store,
    }
  }
}

impl Sidebar {
  fn field(&mut self, id: FieldId) -> &mut TextField {
    self.fields.entry(id).or_insert_with(|| {
      let mut f = TextField::new(id == FieldId::BugText);
      f.password = id == FieldId::WifiPw;
      f.max = match id {
        FieldId::NightFrom | FieldId::NightTo => 5,
        FieldId::SaverMinutes => 3,
        FieldId::BugStart | FieldId::BugEnd => 16,
        _ => 2000,
      };
      f
    })
  }

  /// The store is small: written at once.
  fn save_soon(&mut self) {
    self.store.save();
  }

  fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
    self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| h.clone())
  }

  fn region_at(&self, x: f32, y: f32, horizontal: bool) -> Option<Region> {
    self.regions.iter().rev().find(|r| r.rect.contains(x, y) && r.horizontal == horizontal && r.max > 0.0).copied()
  }

  fn page_open(&self) -> Option<Page> {
    self.page.filter(|_| !self.page_closing)
  }
}

/// "1d 4h", "3h 12m", "5m" (sidebar.html formatUptime without the comma).
fn uptime(ms: u64) -> String {
  let m = ms / 60_000;
  let (h, d) = (m / 60, m / 60 / 24);
  if d > 0 {
    format!("{}d {}h", d, h % 24)
  } else if h > 0 {
    format!("{}h {}m", h, m % 60)
  } else {
    format!("{}m", m)
  }
}

fn primary() -> (RECT, f32) {
  toast::primary()
}

fn modifiers() -> (bool, bool) {
  // AltGr comes as Ctrl+Alt: a character, not a shortcut
  let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 && GetKeyState(VK_MENU.0 as i32) >= 0 };
  let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
  (ctrl, shift)
}

/// The panel's window on the monitor: its right edge, from under the bar to
/// the bottom (ii's sidebar keeps out of the bar's zone). Over the bar, its
/// session button lay on the indicators that toggle it, so the click meant
/// to close the panel opened the session screen.
fn window_rect(mon: RECT, scale: f32) -> RECT {
  let w = (WIN_W * scale).round() as i32;
  let bar = (super::view::BAR_H * scale).round() as i32;
  RECT { left: mon.right - w, top: mon.top + bar, right: mon.right, bottom: mon.bottom }
}

fn make_win(gfx: &Gfx) -> anyhow::Result<Win> {
  let (mon, scale) = primary();
  let r = window_rect(mon, scale);
  let (w, h) = (r.right - r.left, r.bottom - r.top);
  unsafe {
    let hwnd = CreateWindowExW(
      WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
      CLASS,
      &HSTRING::from(TITLE),
      WS_POPUP,
      r.left,
      r.top,
      w,
      h,
      None,
      None,
      GetModuleHandleW(None)?,
      None,
    )?;
    let made = (|| -> windows::core::Result<(IDCompositionTarget, IDCompositionVisual2, Layer, Layer, windows::Win32::Graphics::DirectComposition::IDCompositionScaleTransform, Layer)> {
      let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let root = gfx.dcomp.CreateVisual()?;
      let panel = Layer::new(gfx, w as u32, h as u32)?;
      let ghost = Layer::new(gfx, w as u32, (quick::drag::GHOST_H * scale).ceil() as u32)?;
      let ghost_scale = gfx.dcomp.CreateScaleTransform()?;
      ghost.visual.SetTransform(&ghost_scale)?;
      let gv: IDCompositionVisual3 = ghost.visual.cast()?;
      gv.SetOpacity2(0.0)?;
      let page = Layer::new(gfx, w as u32, h as u32)?;
      root.AddVisual(&panel.visual, false, None)?;
      root.AddVisual(&ghost.visual, false, None)?;
      root.AddVisual(&page.visual, false, None)?;
      target.SetRoot(&root)?;
      Ok((target, root, panel, ghost, ghost_scale, page))
    })();
    match made {
      Ok((target, root, panel, ghost, ghost_scale, page)) => {
        SIDEBAR_HWND.store(hwnd.0 as isize, Ordering::Release);
        Ok(Win { hwnd, scale, h: h as f32 / scale, _target: target, root, panel, ghost, ghost_scale, page })
      }
      Err(err) => {
        let _ = DestroyWindow(hwnd);
        Err(err.into())
      }
    }
  }
}

fn dash_style(gfx: &Gfx) -> Option<ID2D1StrokeStyle> {
  unsafe {
    let factory: ID2D1Factory = gfx.dc.GetFactory().ok()?;
    factory
      .CreateStrokeStyle(
        &D2D1_STROKE_STYLE_PROPERTIES { dashStyle: D2D1_DASH_STYLE_DASH, dashCap: D2D1_CAP_STYLE_ROUND, ..Default::default() },
        None,
      )
      .ok()
  }
}

impl Ui {
  // -------------------------------------------------------------- lifecycle

  pub(super) fn sidebar_event(&mut self, e: Ev) {
    match e {
      Ev::Toggle => {
        if self.sidebar.open {
          self.sidebar_close();
        } else {
          self.sidebar_open(None);
        }
      }
      Ev::OpenPage(name) => {
        let name = name.trim_matches('"').to_string();
        let page = match name.as_str() {
          "keys" => Some(Page::Keys),
          "walls" | "screensaver" => Some(Page::Walls),
          "bug" => Some(Page::Bug),
          _ => None,
        };
        if name == "screensaver" {
          // Windows' "Settings" of our video screen saver: its tab
          self.sidebar.walls.tab = walls::TAB_SAVER;
          self.sidebar.walls.sub = 1;
          self.sidebar.store.wall_page_tab = walls::TAB_SAVER;
        }
        if self.sidebar.open {
          if let Some(p) = page {
            self.sb_page_open(p);
          }
        } else {
          self.sidebar_open(page);
        }
      }
      Ev::NotifsChanged => {
        if self.sidebar.open {
          self.sb_notifs_load();
        } else {
          self.sidebar.notifs.loaded = false;
        }
      }
      Ev::Notifs(v) => {
        self.sidebar.notifs.set(&v);
        self.sb_render();
      }
      Ev::Quick(q) => self.sb_quick_event(q),
      Ev::Image(key, frames) => {
        self.sidebar.images.arrived(&self.gfx, key, frames);
        if self.sidebar.open {
          self.sb_render();
        }
      }
      Ev::Keys(k) => self.sb_keys_event(k),
      Ev::Walls(w) => self.sb_walls_event(w),
      Ev::Bug(b) => self.sb_bug_event(b),
    }
  }

  fn sidebar_open(&mut self, page: Option<Page>) {
    if self.sidebar.open {
      return;
    }
    // what the bar opened goes (one panel at a time)
    self.tray_close();
    self.mixer_close();
    if self.overview.as_ref().is_some_and(|o| o.shown) {
      self.overview_hide();
    }
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SB_CLOSE);
    }
    if self.sidebar.win.is_none() {
      match make_win(&self.gfx) {
        Ok(w) => self.sidebar.win = Some(w),
        Err(err) => {
          tracing::warn!("Sidebar: window: {:?}", err);
          return;
        }
      }
    }
    if self.sidebar.dash.is_none() {
      self.sidebar.dash = dash_style(&self.gfx);
    }
    let resumed = match page {
      None => self.sidebar.resume.take().filter(|(_, at)| at.elapsed() < RESUME).map(|(p, _)| p),
      Some(_) => None,
    };
    let page = page.or(resumed);
    let sb = &mut self.sidebar;
    sb.open = true;
    sb.page = page;
    sb.page_closing = false;
    sb.shown_at = None;
    sb.hover = None;
    sb.pressed = None;
    sb.drag = None;
    sb.focus = None;
    sb.quick.set_menu(None, 0.0);
    sb.quick.edit = false;
    sb.bottom.close_picker();
    sb.scroll.remove(&ScrollId::Notifs);
    self.sb_qs_refresh(false);
    self.sb_notifs_load();
    if let Some(p) = page {
      self.sb_page_started(p, resumed.is_some());
    }
    self.sb_render();
    if let Err(err) = self.sb_slide(true) {
      tracing::debug!("Sidebar: entrance: {:?}", err);
    }
    let Some(w) = &self.sidebar.win else { return };
    let hwnd = w.hwnd;
    unsafe {
      let _ = ShowWindow(hwnd, SW_SHOW);
      let _ = SetForegroundWindow(hwnd);
      SetTimer(self.msg_hwnd, TIMER_SB_TICK, 1000, None);
      if GetForegroundWindow() != hwnd {
        // opened by the core (a gesture): it lends the focus
        std::thread::spawn(|| core_api::run_core(&["--raise", TITLE]));
      }
    }
    self.sidebar.shown_at = Some(Instant::now());
    if page.is_some() {
      let _ = self.sb_page_slide(true);
    }
  }

  pub(super) fn sidebar_close(&mut self) {
    if !self.sidebar.open {
      return;
    }
    self.menu_close();
    // the issue report's text survives a restart too
    if let Some(f) = self.sidebar.fields.get(&FieldId::BugText) {
      let text = f.text();
      if self.sidebar.store.bug_draft != text && self.sidebar.bug.as_ref().is_some_and(|b| !b.sent) {
        self.sidebar.store.bug_draft = text;
        self.sidebar.save_soon();
      }
    }
    let sb = &mut self.sidebar;
    sb.resume = sb.page.filter(|_| !sb.page_closing).map(|p| (p, Instant::now()));
    sb.open = false;
    sb.focus = None;
    sb.drag = None;
    if self.quick_drop_cancel() {
      self.sidebar.save_soon();
    }
    unsafe {
      let _ = ReleaseCapture();
      let _ = KillTimer(self.msg_hwnd, TIMER_SB_TICK);
    }
    if !self.model.animations || self.sb_slide(false).is_err() {
      self.sidebar_destroy();
      return;
    }
    unsafe { SetTimer(self.msg_hwnd, TIMER_SB_CLOSE, (CLOSE_MS.max(FADE_OUT_MS) + 10.0) as u32, None) };
  }

  fn quick_drop_cancel(&mut self) -> bool {
    let tr = |s: &str| self.model.tr(s);
    self.sidebar.quick.release(&self.model, &tr, false);
    self.sidebar.quick.landing = None;
    false
  }

  /// The slide is over: the window goes (nothing stays on screen or in the
  /// compositor between uses); the page's state goes with it.
  pub(super) fn sidebar_destroy(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SB_CLOSE);
      let _ = KillTimer(self.msg_hwnd, TIMER_SB_FRAME);
      let _ = KillTimer(self.msg_hwnd, TIMER_SB_PAGE);
    }
    let sb = &mut self.sidebar;
    if sb.open {
      return;
    }
    sb.frames = false;
    sb.win = None;
    sb.page = None;
    sb.page_closing = false;
    sb.hits.clear();
    sb.regions.clear();
  }

  /// The device was rebuilt: windows and pictures belonged to the old one.
  pub(super) fn sidebar_reset(&mut self) {
    self.sidebar.open = false;
    self.sidebar_destroy();
    self.sidebar.images.clear();
    self.sidebar.dash = None;
  }

  /// Slides the panel in from the right (270 ms, emphasized decelerate, a
  /// 50 ms fade) or out (240 ms, menu_accel, a 270 ms stalling fade) as
  /// illogical-impulse's sidebar layers.
  fn sb_slide(&self, open: bool) -> windows::core::Result<()> {
    let Some(w) = &self.sidebar.win else { return Ok(()) };
    let dcomp = &self.gfx.dcomp;
    let full = WIN_W * 1.1 * w.scale;
    unsafe {
      let v: IDCompositionVisual3 = w.root.cast()?;
      if !self.model.animations {
        v.SetOpacity2(if open { 1.0 } else { 0.0 })?;
        w.root.SetOffsetX2(if open { 0.0 } else { full })?;
        return dcomp.Commit();
      }
      let (from, to, ms, curve) = if open { (full, 0.0, OPEN_MS, POP_IN) } else { (0.0, full, CLOSE_MS, anim::MENU_ACCEL) };
      w.root.SetOffsetX(&anim::build(dcomp, from, to, ms, curve)?)?;
      let fade = if open { anim::build(dcomp, 0.0, 1.0, FADE_IN_MS, anim::MENU_DECEL)? } else { anim::build(dcomp, 1.0, 0.0, FADE_OUT_MS, anim::STALL)? };
      v.SetOpacity(&fade)?;
      dcomp.Commit()
    }
  }

  /// A page slides over the panel from the right (340 ms) or back (260 ms).
  fn sb_page_slide(&self, open: bool) -> windows::core::Result<()> {
    let Some(w) = &self.sidebar.win else { return Ok(()) };
    let dcomp = &self.gfx.dcomp;
    let bug = self.sidebar.page == Some(Page::Bug);
    let full = (WIN_W - INSET) * 1.04 * w.scale;
    unsafe {
      let v: IDCompositionVisual3 = w.page.visual.cast()?;
      if !self.model.animations {
        v.SetOpacity2(if open { 1.0 } else { 0.0 })?;
        w.page.visual.SetOffsetX2(0.0)?;
        return dcomp.Commit();
      }
      if bug {
        // the issue report fades in over its dimmed panel
        w.page.visual.SetOffsetX2(0.0)?;
        v.SetOpacity(&anim::build(dcomp, if open { 0.0 } else { 1.0 }, if open { 1.0 } else { 0.0 }, if open { 220.0 } else { 160.0 }, POP_IN)?)?;
      } else if open {
        w.page.visual.SetOffsetX(&anim::build(dcomp, full, 0.0, PAGE_IN_MS, POP_IN)?)?;
        v.SetOpacity(&anim::build(dcomp, 0.6, 1.0, 240.0, POP_IN)?)?;
      } else {
        w.page.visual.SetOffsetX(&anim::build(dcomp, 0.0, full, PAGE_OUT_MS, POP_OUT)?)?;
        v.SetOpacity(&anim::build(dcomp, 1.0, 0.6, PAGE_OUT_MS, POP_OUT)?)?;
      }
      dcomp.Commit()
    }
  }

  fn sb_page_started(&mut self, p: Page, resumed: bool) {
    if !resumed {
      self.sidebar.scroll.remove(&ScrollId::Page);
    }
    match p {
      Page::Keys => self.sb_keys_open(),
      Page::Walls => self.sb_walls_open(),
      Page::Bug => self.sb_bug_open(),
    }
  }

  fn sb_page_open(&mut self, p: Page) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SB_PAGE);
    }
    self.sidebar.quick.set_menu(None, 0.0);
    self.sidebar.page = Some(p);
    self.sidebar.page_closing = false;
    self.sidebar.focus = None;
    self.sb_page_started(p, false);
    self.sb_render();
    let _ = self.sb_page_slide(true);
  }

  pub(super) fn sb_page_close(&mut self) {
    if self.sidebar.page.is_none() || self.sidebar.page_closing {
      return;
    }
    self.menu_close();
    self.sidebar.page_closing = true;
    self.sidebar.focus = None;
    if !self.model.animations || self.sb_page_slide(false).is_err() {
      self.sb_page_gone();
      return;
    }
    let ms = if self.sidebar.page == Some(Page::Bug) { 170 } else { PAGE_OUT_MS as u32 + 10 };
    unsafe { SetTimer(self.msg_hwnd, TIMER_SB_PAGE, ms, None) };
  }

  pub(super) fn sb_page_gone(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_SB_PAGE);
    }
    if self.sidebar.page == Some(Page::Bug) {
      // left with Back: the text stays a draft unless it was sent
      if self.sidebar.bug.as_ref().is_some_and(|b| !b.sent) {
        if let Some(f) = self.sidebar.fields.get(&FieldId::BugText) {
          self.sidebar.store.bug_draft = f.text();
          self.sidebar.save_soon();
        }
      }
      self.sidebar.bug = None;
    }
    self.sidebar.page = None;
    self.sidebar.page_closing = false;
    self.sb_render();
  }

  /// Timers of the panel (the bar's message window gets them).
  pub(super) fn sidebar_timer(&mut self, id: usize) {
    match id {
      TIMER_SB_CLOSE => self.sidebar_destroy(),
      TIMER_SB_PAGE => self.sb_page_gone(),
      TIMER_SB_FRAME => {
        self.sb_notifs_frame();
        self.sb_tiles_frame();
        self.sb_render();
      }
      TIMER_SB_TICK => {
        self.sb_quick_tick();
        self.sb_walls_tick();
        self.sb_render();
      }
      TIMER_SB_WHEEL => {
        unsafe {
          let _ = KillTimer(self.msg_hwnd, TIMER_SB_WHEEL);
        }
        let app = self.sidebar.notifs.wheel_end();
        self.sb_notif_settled(app);
      }
      _ => {}
    }
  }

  /// Something moves: frames until it stops.
  pub(super) fn sb_frames(&mut self) {
    if !self.sidebar.frames && self.sidebar.win.is_some() {
      self.sidebar.frames = true;
      unsafe { SetTimer(self.msg_hwnd, TIMER_SB_FRAME, 16, None) };
    }
  }

  // -------------------------------------------------------------- painting

  pub(super) fn sb_render(&mut self) {
    if self.sidebar.win.is_none() {
      return;
    }
    match self.sb_render_inner() {
      Ok(busy) => {
        let busy = busy || self.sidebar.notifs.busy() || self.sidebar.quick.pressing() || self.sidebar.quick.landing.is_some();
        if busy {
          self.sb_frames();
        } else if self.sidebar.frames {
          self.sidebar.frames = false;
          unsafe {
            let _ = KillTimer(self.msg_hwnd, TIMER_SB_FRAME);
          }
        }
      }
      Err(err) => tracing::warn!("Sidebar: paint: {:?}", err),
    }
  }

  fn sb_render_inner(&mut self) -> anyhow::Result<bool> {
    let theme = self.theme();
    let animations = self.model.animations;
    let Ui { gfx, fonts, res, icons, model, sidebar, .. } = self;
    let sb = sidebar;
    let Some(w) = &sb.win else { return Ok(false) };
    let (scale, h) = (w.scale, w.h);
    let (panel_s, page_s) = (w.panel.surface.clone(), w.page.surface.clone());
    let tr = |s: &str| model.tr(s);
    let mut hits = Vec::new();
    let mut regions = Vec::new();
    let mut requests = Vec::new();
    let mut busy = false;
    let spin = (sb.spin0.elapsed().as_millis() % 1000) as f32 * 0.36;
    let (hover, pressed, focus, mouse) = (sb.hover.clone(), sb.pressed.clone(), sb.focus, sb.mouse);
    let page = sb.page;
    let page_only = page.is_some() && !sb.page_closing && sb.page != Some(Page::Bug) && sb.shown_at.is_some_and(|t| t.elapsed() > Duration::from_millis(400));
    if !page_only {
      gfx::draw_surface(&panel_s, scale, |dc| {
        let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
        let mut cx = Cx::new(&mut p, theme, &mut hits, &mut regions, hover.clone(), pressed.clone(), focus, &tr, spin);
        cx.mouse = mouse;
        if let Err(err) = paint_main(&mut cx, sb, model, h, animations) {
          tracing::warn!("Sidebar: panel: {:?}", err);
        }
        busy |= cx.busy;
        Ok(())
      })?;
    } else {
      // the page covers the panel: its clicks stay out
      hits.push((Rect::new(INSET, INSET, WIN_W - 2.0 * INSET, h - 2.0 * INSET), Hit::Panel));
    }
    gfx::draw_surface(&page_s, scale, |dc| {
      let Some(page) = page else { return Ok(()) };
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      let mut cx = Cx::new(&mut p, theme, &mut hits, &mut regions, hover.clone(), pressed.clone(), focus, &tr, spin);
      cx.mouse = mouse;
      if let Err(err) = paint_page(&mut cx, sb, model, page, h) {
        tracing::warn!("Sidebar: page: {:?}", err);
      }
      busy |= cx.busy;
      Ok(())
    })?;
    unsafe { gfx.dcomp.Commit()? };
    sb.hits = hits;
    sb.regions = regions;
    Ok(busy)
  }

  // -------------------------------------------------------------- input

  // pages own no drags or sliders of their own yet: the hooks keep them in one place

  /// A screen point of a panel point (DIPs), for menus.
  pub(super) fn sb_screen(&self, x: f32, y: f32) -> POINT {
    let Some(w) = &self.sidebar.win else { return POINT::default() };
    let mut r = RECT::default();
    unsafe {
      let _ = GetWindowRect(w.hwnd, &mut r);
    }
    POINT { x: r.left + (x * w.scale) as i32, y: r.top + (y * w.scale) as i32 }
  }

  /// Opens one of our context menus over the panel (it keeps the keyboard).
  pub(super) fn sb_menu(&mut self, x: f32, y: f32, items: Vec<super::menu::Item>, pick: impl FnOnce(&mut Ui, &str) + 'static) {
    let at = self.sb_screen(x, y);
    self.menu_open(at, MenuFocus::Keep, items, pick);
  }

  /// A core file dialog is open: the panel stays open meanwhile.
  pub(super) fn sb_modal(&mut self, on: bool) {
    if on {
      self.sidebar.modal += 1;
    } else {
      self.sidebar.modal = self.sidebar.modal.saturating_sub(1);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn uptime_reads_like_the_web_panel() {
    assert_eq!(uptime(5 * 60_000), "5m");
    assert_eq!(uptime((3 * 60 + 12) * 60_000), "3h 12m");
    assert_eq!(uptime((26 * 60 + 5) * 60_000), "1d 2h");
  }

  #[test]
  fn the_open_panel_leaves_the_bar_uncovered() {
    // the bar's indicators toggle the panel; a panel over the bar put its
    // session button on them, so the click meant to close it opened the
    // session screen
    let mon = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    for scale in [1.0, 1.25, 1.5, 2.0] {
      let r = window_rect(mon, scale);
      let bar_bottom = mon.top + (crate::native_bar::view::BAR_H * scale).round() as i32;
      assert!(r.top >= bar_bottom, "scale {}: the panel starts at {}, the bar ends at {}", scale, r.top, bar_bottom);
      assert_eq!((r.right, r.bottom), (mon.right, mon.bottom), "scale {}: right edge, down to the bottom", scale);
      assert_eq!(r.right - r.left, (WIN_W * scale).round() as i32, "scale {}: the width", scale);
    }
  }
}
