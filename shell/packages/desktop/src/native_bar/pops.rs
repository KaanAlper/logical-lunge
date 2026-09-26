//! What the popups do: hover popups (resources, media) with ii's 200 ms
//! close delay, the tray panel (click-away closes it, dragging does not),
//! drag to pin / unpin tray icons, and tooltips.

use std::{
  collections::{HashMap, HashSet},
  os::windows::process::CommandExt,
  path::PathBuf,
  sync::atomic::Ordering,
  time::Instant,
};

use windows::Win32::{
  Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
  Graphics::Direct2D::ID2D1Bitmap1,
  System::Threading::GetCurrentProcessId,
  UI::{
    Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
    Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
    WindowsAndMessaging::*,
  },
};

use super::{
  gfx::Rect,
  model::{pin_key, Model},
  popup::{self, Hit, MediaView, Motion, PopHit, PopKind, PopWin, Temps, TrayView, GAP, PAD},
  send,
  view::{self, HitKind, Painter},
  Msg, Ui, TIMER_POP_CLOSE, TIMER_POP_HIDE, TIMER_POP_TICK, TIMER_TIP, TIMER_TRAY_HIDE, WAKE, WM_APP_TRAY_CLOSE,
};
use crate::providers::{MediaControlArgs, MediaFunction, ProviderFunction, SystrayFunction, SystrayIconArgs};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, PartialEq)]
enum TipAt {
  Bar(usize, HitKind),
  Panel(String),
}

struct Drag {
  id: String,
  key: String,
  start: POINT,
  started: bool,
}

#[derive(Default)]
pub struct PopState {
  hover: Option<PopWin>,
  kind: Option<PopKind>,
  bar: usize,
  anchor: Rect,
  hover_hit: Option<PopHit>,
  close_pending: bool,

  tray: Option<PopWin>,
  tray_bar: usize,
  tray_anchor: Rect,
  tray_hover: Option<String>,
  tray_drop: bool,
  mouse_hook: Option<HHOOK>,
  fg_hook: Option<HWINEVENTHOOK>,

  tip: Option<PopWin>,
  tip_at: Option<TipAt>,

  ghost: Option<PopWin>,
  drag: Option<Drag>,

  temps: Option<Temps>,
  temps_busy: bool,
  art: HashMap<String, Option<ID2D1Bitmap1>>,
  art_asked: HashSet<String>,
  /// (title, scale x 100, blurred background)
  art_bg: Option<(String, u32, ID2D1Bitmap1)>,
  /// (title|position|playing, position seconds, when it was read)
  clock: (String, f64, Option<Instant>),
}

fn tools_dir() -> PathBuf {
  std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("tools"))).unwrap_or_default()
}

fn window_rect(hwnd: HWND) -> RECT {
  let mut r = RECT::default();
  unsafe {
    let _ = GetWindowRect(hwnd, &mut r);
  }
  r
}

fn cursor() -> POINT {
  let mut p = POINT::default();
  unsafe {
    let _ = GetCursorPos(&mut p);
  }
  p
}

// Tray panel click-away: a click outside this process, or another window
// coming to the front, closes it. Installed only while the panel is open.
unsafe extern "system" fn mouse_ll(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
  if code >= 0 && matches!(wp.0 as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
    let m = &*(lp.0 as *const MSLLHOOKSTRUCT);
    let under = WindowFromPoint(m.pt);
    let mut pid = 0u32;
    if !under.is_invalid() {
      GetWindowThreadProcessId(GetAncestor(under, GA_ROOT), Some(&mut pid));
    }
    if pid != GetCurrentProcessId() {
      post_close();
    }
  }
  CallNextHookEx(None, code, wp, lp)
}

unsafe extern "system" fn foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
  let mut pid = 0u32;
  GetWindowThreadProcessId(hwnd, Some(&mut pid));
  if pid != GetCurrentProcessId() {
    post_close();
  }
}

fn post_close() {
  let h = WAKE.load(Ordering::Acquire);
  if h != 0 {
    unsafe {
      let _ = PostMessageW(HWND(h as _), WM_APP_TRAY_CLOSE, WPARAM(0), LPARAM(0));
    }
  }
}

impl Ui {
  pub(super) fn pop_windows(&self) -> [Option<HWND>; 4] {
    let p = &self.pops;
    [p.hover.as_ref().map(|w| w.hwnd), p.tray.as_ref().map(|w| w.hwnd), p.tip.as_ref().map(|w| w.hwnd), p.ghost.as_ref().map(|w| w.hwnd)]
  }

  /// Graphics device rebuilt / monitors changed: popups start over.
  pub(super) fn pops_reset(&mut self) {
    self.tray_hooks(false);
    if self.pops.drag.is_some() {
      self.pops.drag = None;
      unsafe {
        let _ = ReleaseCapture();
      }
    }
    let p = &mut self.pops;
    p.hover = None;
    p.tray = None;
    p.tip = None;
    p.ghost = None;
    p.kind = None;
    p.art_bg = None;
    p.art.clear();
    p.art_asked.clear();
    self.model.tray_open = false;
  }

  fn measure_painter<R>(&mut self, f: impl FnOnce(&mut Painter, &Model) -> R) -> R {
    let mut requests = Vec::new();
    let Ui { gfx, fonts, res, icons, model, .. } = self;
    let mut p = Painter { dc: &gfx.dc, gfx, fonts, res, icons, requests: &mut requests };
    f(&mut p, model)
  }

  /// A popup window for bar `i`'s DPI (recreated when the DPI differs).
  fn ensure(win: &mut Option<PopWin>, gfx: &super::Gfx, title: &str, scale: f32, motion: Motion) -> bool {
    if win.as_ref().is_some_and(|w| w.scale != scale) {
      *win = None;
    }
    if win.is_none() {
      match PopWin::new(gfx, title, scale, motion) {
        Ok(w) => *win = Some(w),
        Err(err) => {
          tracing::warn!("Native bar popup: {:?}", err);
          return false;
        }
      }
    }
    true
  }

  /// Screen position of a box `w` DIPs wide centred under `anchor` (bar
  /// DIPs), kept 8 DIPs inside the bar; the window starts `PAD` earlier.
  fn under(&self, i: usize, anchor: Rect, w: f32) -> (i32, i32) {
    let bar = &self.bars[i];
    let s = bar.scale;
    let left = (anchor.x + anchor.w / 2.0 - w / 2.0).min(bar.width - w - 8.0).max(8.0);
    let r = window_rect(bar.hwnd);
    (r.left + ((left - PAD) * s).round() as i32, r.top + ((view::BAR_H + GAP - PAD) * s).round() as i32)
  }

  // ------------------------------------------------------------ hover popups

  /// Hover moved over the bar: open / keep / schedule closing.
  pub(super) fn pop_follow_hover(&mut self, i: usize, hover: Option<&HitKind>) {
    let want = match hover {
      Some(HitKind::Resources) => Some(PopKind::Res),
      Some(HitKind::Media) => Some(PopKind::Media),
      _ => None,
    };
    match want {
      Some(kind) => {
        let anchor = self.bars[i].frame.hits.iter().find(|h| Some(&h.kind) == hover).map(|h| h.rect).unwrap_or_default();
        self.pop_open(i, kind, anchor);
      }
      None => self.pop_schedule_close(),
    }
  }

  fn pop_open(&mut self, i: usize, kind: PopKind, anchor: Rect) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_POP_CLOSE);
      let _ = KillTimer(self.msg_hwnd, TIMER_POP_HIDE);
    }
    self.pops.close_pending = false;
    let open = self.pops.hover.as_ref().is_some_and(|w| w.shown && !w.closing);
    if open && self.pops.kind == Some(kind) && self.pops.bar == i {
      return;
    }
    self.pops.kind = Some(kind);
    self.pops.bar = i;
    self.pops.anchor = anchor;
    self.pops.hover_hit = None;
    match kind {
      PopKind::Res => {
        self.read_temps();
        unsafe { SetTimer(self.msg_hwnd, TIMER_POP_TICK, 2000, None) };
      }
      PopKind::Media => {
        self.ask_art();
        unsafe { SetTimer(self.msg_hwnd, TIMER_POP_TICK, 1000, None) };
      }
    }
    self.pop_render();
  }

  /// ii: popups close 200 ms after the pointer leaves (moving onto the popup keeps it).
  pub(super) fn pop_schedule_close(&mut self) {
    if self.pops.kind.is_none() || self.pops.close_pending {
      return;
    }
    self.pops.close_pending = true;
    unsafe { SetTimer(self.msg_hwnd, TIMER_POP_CLOSE, 200, None) };
  }

  pub(super) fn pop_close_now(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_POP_CLOSE);
    }
    self.pops.close_pending = false;
    if let Some(w) = &mut self.pops.hover {
      if w.close(&self.gfx).is_ok() {
        unsafe { SetTimer(self.msg_hwnd, TIMER_POP_HIDE, w.close_ms(), None) };
      }
    }
  }

  pub(super) fn pop_hidden(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_POP_HIDE);
      let _ = KillTimer(self.msg_hwnd, TIMER_POP_TICK);
    }
    if let Some(w) = &mut self.pops.hover {
      if w.closing {
        w.hide();
        self.pops.kind = None;
      }
    }
  }

  /// New media state: the open media popup shows it.
  pub(super) fn pop_render_media(&mut self) {
    if self.pops.kind == Some(PopKind::Media) && self.pops.hover.as_ref().is_some_and(|w| w.shown && !w.closing) {
      self.pop_render();
    }
  }

  pub(super) fn pop_tick(&mut self) {
    match self.pops.kind {
      Some(PopKind::Res) => self.read_temps(),
      Some(PopKind::Media) => self.pop_render(),
      None => unsafe {
        let _ = KillTimer(self.msg_hwnd, TIMER_POP_TICK);
      },
    }
  }

  /// Draws the hover popup and places it (animating in when it was closed).
  pub(super) fn pop_render(&mut self) {
    let Some(kind) = self.pops.kind else { return };
    let i = self.pops.bar;
    if i >= self.bars.len() {
      return;
    }
    let scale = self.bars[i].scale;
    let theme = self.theme();
    let temps = self.pops.temps;
    let size = match kind {
      PopKind::Res => match self.measure_painter(|p, m| popup::res_size(p, m, temps.as_ref())) {
        Ok(s) => s,
        Err(err) => {
          tracing::warn!("Native bar popup size: {:?}", err);
          return;
        }
      },
      PopKind::Media => (popup::MEDIA_W, popup::MEDIA_H),
    };
    if kind == PopKind::Media {
      self.ensure_art_bg(scale);
    }
    if !Self::ensure(&mut self.pops.hover, &self.gfx, "Logical Lunge · popup", scale, Motion::Slide) {
      return;
    }
    let (x, y) = self.under(i, self.pops.anchor, size.0);
    let (pos, end, playing) = self.media_now();
    let Ui { gfx, fonts, res, icons, model, pops, .. } = self;
    let Some(win) = pops.hover.as_mut() else { return };
    if let Err(err) = win.resize(gfx, size.0 + 2.0 * PAD, size.1 + 2.0 * PAD) {
      tracing::warn!("Native bar popup: {:?}", err);
      return;
    }
    let title = model.media.as_ref().and_then(|m| m.current_session.as_ref()).and_then(|s| s.title.clone()).unwrap_or_default();
    let art = pops.art.get(&title).cloned().flatten();
    let bg = pops.art_bg.as_ref().filter(|(t, sc, _)| *t == title && *sc == (scale * 100.0) as u32).map(|(_, _, b)| b.clone());
    let hover = pops.hover_hit.clone();
    let mut requests = Vec::new();
    let mut hits: Vec<Hit> = Vec::new();
    let drawn = win.draw(|dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      let r = match kind {
        PopKind::Res => popup::paint_res(&mut p, model, theme, temps.as_ref(), size).map(|_| Vec::new()),
        PopKind::Media => popup::paint_media(
          &mut p,
          model,
          theme,
          &MediaView { art: art.as_ref(), bg: bg.as_ref(), pos, end, playing, hover: hover.as_ref() },
        ),
      };
      match r {
        Ok(h) => hits = h,
        Err(err) => tracing::warn!("Native bar popup paint: {:?}", err),
      }
      Ok(())
    });
    if let Err(err) = drawn {
      tracing::warn!("Native bar popup draw: {:?}", err);
      return;
    }
    win.hits = hits;
    if let Err(err) = win.show_at(gfx, x, y) {
      tracing::warn!("Native bar popup show: {:?}", err);
    }
  }

  /// Pointer over the hover popup.
  pub(super) fn pop_mouse(&mut self, x: i32, y: i32) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_POP_CLOSE);
    }
    self.pops.close_pending = false;
    let Some(closing) = self.pops.hover.as_ref().map(|w| w.closing) else { return };
    if closing {
      // back onto a closing popup: open it again (`keepPop`)
      let (i, kind, anchor) = (self.pops.bar, self.pops.kind, self.pops.anchor);
      if let Some(kind) = kind {
        self.pops.kind = None;
        self.pop_open(i, kind, anchor);
      }
      return;
    }
    let Some(w) = &self.pops.hover else { return };
    let s = w.scale;
    let hit = popup::hit_at(&w.hits, x as f32 / s, y as f32 / s).map(|h| h.kind.clone());
    if hit != self.pops.hover_hit {
      self.pops.hover_hit = hit;
      self.pop_render();
    }
  }

  pub(super) fn pop_leave(&mut self) {
    if self.pops.hover_hit.take().is_some() {
      self.pop_render();
    }
    self.pop_schedule_close();
  }

  pub(super) fn pop_click(&mut self, x: i32, y: i32) {
    let Some(w) = &self.pops.hover else { return };
    let s = w.scale;
    let (dx, dy) = (x as f32 / s, y as f32 / s);
    let Some(kind) = popup::hit_at(&w.hits, dx, dy).map(|h| h.kind.clone()) else { return };
    let session_id = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()).map(|s| s.session_id.clone());
    let media = |f: fn(MediaControlArgs) -> MediaFunction| ProviderFunction::Media(f(MediaControlArgs { session_id: session_id.clone() }));
    match kind {
      PopHit::Prev => self.provider("media", media(MediaFunction::Previous)),
      PopHit::Next => self.provider("media", media(MediaFunction::Next)),
      PopHit::Play => self.provider("media", media(MediaFunction::TogglePlayPause)),
      PopHit::Seek(bar) => {
        let (_, end, _) = self.media_now();
        if end > 0.0 && bar.w > 0.0 {
          let sec = (((dx - bar.x) / bar.w).clamp(0.0, 1.0) as f64) * end;
          // the clock jumps at once; the provider catches up
          self.pops.clock.1 = sec;
          self.pops.clock.2 = Some(Instant::now());
          let exe = tools_dir().join("lunge-media.exe");
          std::thread::spawn(move || {
            let _ = std::process::Command::new(exe).args(["--seek", &format!("{:.1}", sec)]).creation_flags(CREATE_NO_WINDOW).status();
          });
          self.pop_render();
        }
      }
      PopHit::TrayIcon(_) => {}
    }
  }

  // ------------------------------------------------------------ data

  fn read_temps(&mut self) {
    if self.pops.temps_busy {
      return;
    }
    self.pops.temps_busy = true;
    let exe = tools_dir().join("temps").join("lunge-temps.exe");
    std::thread::spawn(move || {
      let out = std::process::Command::new(exe).arg("--read").creation_flags(CREATE_NO_WINDOW).output();
      let t = out.ok().and_then(|o| Temps::parse(&String::from_utf8_lossy(&o.stdout)));
      send(Msg::Temps(t));
    });
  }

  pub(super) fn got_temps(&mut self, t: Option<Temps>) {
    self.pops.temps_busy = false;
    if t.is_some() {
      self.pops.temps = t;
    }
    if self.pops.kind == Some(PopKind::Res) {
      self.pop_render();
    }
  }

  /// The current song's cover (`lunge-media.exe`), asked as soon as the
  /// song changes so the popup opens with it.
  pub(super) fn ask_art(&mut self) {
    let Some(title) = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()).and_then(|s| s.title.clone()).filter(|t| !t.is_empty()) else {
      return;
    };
    if self.pops.art.contains_key(&title) || !self.pops.art_asked.insert(title.clone()) {
      return;
    }
    let exe = tools_dir().join("lunge-media.exe");
    std::thread::spawn(move || {
      let out = std::process::Command::new(exe).creation_flags(CREATE_NO_WINDOW).output();
      let bytes = out.ok().and_then(|o| super::icons::data_url_bytes(String::from_utf8_lossy(&o.stdout).trim()));
      send(Msg::Art(title, bytes));
    });
  }

  pub(super) fn got_art(&mut self, title: String, bytes: Option<Vec<u8>>) {
    self.pops.art_asked.remove(&title);
    let bmp = bytes.and_then(|b| self.gfx.bitmap(&b).ok());
    if self.pops.art.len() >= 16 {
      self.pops.art.clear();
    }
    self.pops.art.insert(title, bmp);
    if self.pops.kind == Some(PopKind::Media) {
      self.pop_render();
    }
  }

  fn ensure_art_bg(&mut self, scale: f32) {
    let Some(title) = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()).and_then(|s| s.title.clone()) else { return };
    let key = (scale * 100.0) as u32;
    if self.pops.art_bg.as_ref().is_some_and(|(t, k, _)| *t == title && *k == key) {
      return;
    }
    let Some(Some(art)) = self.pops.art.get(&title) else { return };
    match popup::blur_art(&self.gfx, art, scale) {
      Ok(bg) => self.pops.art_bg = Some((title, key, bg)),
      Err(err) => tracing::warn!("Native bar media background: {:?}", err),
    }
  }

  /// Keeps the media clock: position as reported plus the time since, while
  /// playing (the provider reports only on changes).
  pub(super) fn media_seen(&mut self) {
    let Some(s) = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()) else { return };
    let key = format!("{:?}|{}|{}", s.title, s.position, s.is_playing);
    if key != self.pops.clock.0 {
      self.pops.clock = (key, s.position as f64, Some(Instant::now()));
    }
  }

  /// (position, end, playing) now.
  fn media_now(&self) -> (f64, f64, bool) {
    let Some(s) = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()) else { return (0.0, 0.0, false) };
    let end = s.end_time as f64;
    let since = self.pops.clock.2.map_or(0.0, |t| t.elapsed().as_secs_f64());
    let mut pos = self.pops.clock.1 + if s.is_playing { since } else { 0.0 };
    if end > 0.0 {
      pos = pos.min(end);
    }
    (pos, end, s.is_playing)
  }

  // ------------------------------------------------------------ tray panel

  pub(super) fn tray_toggle(&mut self, i: usize) {
    if self.pops.tray.as_ref().is_some_and(|w| w.shown && !w.closing) {
      self.tray_close();
    } else {
      self.tray_open(i);
    }
  }

  fn tray_open(&mut self, i: usize) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_TRAY_HIDE);
    }
    self.pops.tray_bar = i;
    self.pops.tray_anchor = self.bars[i].frame.hits.iter().find(|h| h.kind == HitKind::TrayMore).map(|h| h.rect).unwrap_or_default();
    self.model.tray_open = true;
    self.tray_hooks(true);
    self.tray_render();
    self.redraw_all();
  }

  pub(super) fn tray_close(&mut self) {
    if self.pops.drag.as_ref().is_some_and(|d| d.started) {
      return; // dragging: the panel is a drop target
    }
    self.tray_hooks(false);
    self.pops.tray_hover = None;
    if self.model.tray_open {
      self.model.tray_open = false;
      self.redraw_all();
    }
    if let Some(w) = &mut self.pops.tray {
      if w.close(&self.gfx).is_ok() {
        unsafe { SetTimer(self.msg_hwnd, TIMER_TRAY_HIDE, w.close_ms(), None) };
      }
    }
  }

  pub(super) fn tray_hidden(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_TRAY_HIDE);
    }
    if let Some(w) = &mut self.pops.tray {
      if w.closing {
        w.hide();
      }
    }
  }

  fn tray_hooks(&mut self, on: bool) {
    unsafe {
      if on {
        if self.pops.mouse_hook.is_none() {
          if let Ok(m) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) {
            let hinst: windows::Win32::Foundation::HINSTANCE = m.into();
            self.pops.mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_ll), hinst, 0).ok();
          }
        }
        if self.pops.fg_hook.is_none() {
          let h = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
          if !h.is_invalid() {
            self.pops.fg_hook = Some(h);
          }
        }
      } else {
        if let Some(h) = self.pops.mouse_hook.take() {
          let _ = UnhookWindowsHookEx(h);
        }
        if let Some(h) = self.pops.fg_hook.take() {
          let _ = UnhookWinEvent(h);
        }
      }
    }
  }

  pub(super) fn tray_render(&mut self) {
    let i = self.pops.tray_bar;
    if i >= self.bars.len() {
      return;
    }
    let scale = self.bars[i].scale;
    let theme = self.theme();
    let n = self.model.unpinned_icons().len();
    let size = popup::tray_size(n);
    if !Self::ensure(&mut self.pops.tray, &self.gfx, "Logical Lunge · tray", scale, Motion::Slide) {
      return;
    }
    let (x, y) = self.under(i, self.pops.tray_anchor, size.0);
    let dragging = self.pops.drag.as_ref().filter(|d| d.started).map(|d| d.key.clone());
    let Ui { gfx, fonts, res, icons, model, pops, .. } = self;
    let hover = pops.tray_hover.clone();
    let drop_zone = pops.tray_drop;
    let Some(win) = pops.tray.as_mut() else { return };
    if win.resize(gfx, size.0 + 2.0 * PAD, size.1 + 2.0 * PAD).is_err() {
      return;
    }
    let mut requests = Vec::new();
    let mut hits = Vec::new();
    let drawn = win.draw(|dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      let v = TrayView { hover: hover.as_deref(), drop_zone, dragging: dragging.as_deref() };
      match popup::paint_tray(&mut p, model, theme, &v) {
        Ok(h) => hits = h,
        Err(err) => tracing::warn!("Native bar tray paint: {:?}", err),
      }
      Ok(())
    });
    if drawn.is_err() {
      return;
    }
    win.hits = hits;
    let _ = win.show_at(gfx, x, y);
  }

  fn tray_hit(&self, x: i32, y: i32) -> Option<String> {
    let w = self.pops.tray.as_ref()?;
    let s = w.scale;
    match popup::hit_at(&w.hits, x as f32 / s, y as f32 / s).map(|h| &h.kind) {
      Some(PopHit::TrayIcon(id)) => Some(id.clone()),
      _ => None,
    }
  }

  pub(super) fn tray_mouse(&mut self, x: i32, y: i32) {
    if self.pops.drag.is_some() {
      self.drag_move();
      return;
    }
    let hit = self.tray_hit(x, y);
    if hit != self.pops.tray_hover {
      self.pops.tray_hover = hit.clone();
      self.tray_render();
      self.tip_candidate(hit.map(TipAt::Panel));
    }
  }

  pub(super) fn tray_leave(&mut self) {
    if self.pops.drag.is_some() {
      return;
    }
    if self.pops.tray_hover.take().is_some() {
      self.tray_render();
    }
    self.tip_candidate(None);
  }

  pub(super) fn tray_button_down(&mut self, x: i32, y: i32) {
    if let Some(id) = self.tray_hit(x, y) {
      let hwnd = self.pops.tray.as_ref().map(|w| w.hwnd).unwrap_or_default();
      self.drag_begin(id, hwnd);
    }
  }

  /// button: 0 left, 1 right, 2 middle, 3 left double
  pub(super) fn tray_button_up(&mut self, x: i32, y: i32, button: u8) {
    if button == 0 && self.drag_end() {
      return;
    }
    if let Some(id) = self.tray_hit(x, y) {
      self.tip_candidate(None);
      self.tray_action(id, button);
    }
  }

  pub(super) fn tray_action(&self, id: String, button: u8) {
    let args = SystrayIconArgs { icon_id: id };
    let f = match button {
      0 => SystrayFunction::IconLeftClick(args),
      1 => SystrayFunction::IconRightClick(args),
      2 => SystrayFunction::IconMiddleClick(args),
      _ => SystrayFunction::IconLeftDoubleClick(args),
    };
    self.provider("systray", ProviderFunction::Systray(f));
  }

  // ------------------------------------------------------------ drag to pin

  pub(super) fn drag_begin(&mut self, id: String, source: HWND) {
    let Some(key) = self.model.tray_icon(&id).map(pin_key) else { return };
    self.pops.drag = Some(Drag { id, key, start: cursor(), started: false });
    unsafe {
      SetCapture(source);
    }
  }

  pub(super) fn dragging(&self) -> bool {
    self.pops.drag.is_some()
  }

  /// Pointer moved while a tray icon is held.
  pub(super) fn drag_move(&mut self) {
    let pt = cursor();
    let Some(d) = self.pops.drag.as_mut() else { return };
    if !d.started {
      if ((pt.x - d.start.x).pow(2) + (pt.y - d.start.y).pow(2)) < 25 {
        return;
      }
      d.started = true;
      self.tip_candidate(None);
      // ii: dragging opens the panel (it is where icons are put away)
      if !self.pops.tray.as_ref().is_some_and(|w| w.shown && !w.closing) {
        let i = self.bar_at(pt).unwrap_or(0);
        self.tray_open(i);
      } else {
        self.tray_render();
      }
    }
    self.ghost_at(pt);
    let over = self.pops.tray.as_ref().is_some_and(|w| w.contains_screen(pt.x, pt.y));
    if over != self.pops.tray_drop {
      self.pops.tray_drop = over;
      self.tray_render();
    }
  }

  fn bar_at(&self, pt: POINT) -> Option<usize> {
    self.bars.iter().position(|b| {
      let r = window_rect(b.hwnd);
      pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
    })
  }

  fn ghost_at(&mut self, pt: POINT) {
    let i = self.bar_at(pt).unwrap_or(self.pops.tray_bar.min(self.bars.len().saturating_sub(1)));
    if self.bars.is_empty() {
      return;
    }
    let scale = self.bars[i].scale;
    let first = self.pops.ghost.as_ref().map_or(true, |g| !g.shown || g.scale != scale);
    if first {
      if !Self::ensure(&mut self.pops.ghost, &self.gfx, "Logical Lunge · drag", scale, Motion::None) {
        return;
      }
      let icon = self.pops.drag.as_ref().and_then(|d| self.model.tray_icon(&d.id)).map(|ic| (ic.icon_hash.clone(), ic.icon_bytes.clone()));
      let Ui { gfx, fonts, res, icons, pops, .. } = self;
      let Some(win) = pops.ghost.as_mut() else { return };
      if win.resize(gfx, popup::GHOST + 2.0 * PAD, popup::GHOST + 2.0 * PAD).is_err() {
        return;
      }
      let bmp = icon.and_then(|(h, b)| icons.tray(gfx, &h, &b));
      let mut requests = Vec::new();
      let _ = win.draw(|dc| {
        let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
        let _ = popup::paint_ghost(&mut p, bmp.as_ref());
        Ok(())
      });
      let _ = unsafe { gfx.dcomp.Commit() };
    }
    let Some(win) = self.pops.ghost.as_mut() else { return };
    let s = win.scale;
    // `.tray-ghost`: centred on the pointer
    let off = ((PAD + popup::GHOST / 2.0) * s).round() as i32;
    let _ = win.show_at(&self.gfx, pt.x - off, pt.y - off);
  }

  /// Button released: drops the dragged icon. False when nothing was
  /// dragged (the release is a click).
  pub(super) fn drag_end(&mut self) -> bool {
    let Some(d) = self.pops.drag.take() else { return false };
    unsafe {
      let _ = ReleaseCapture();
    }
    if !d.started {
      return false;
    }
    if let Some(g) = &mut self.pops.ghost {
      g.hide();
    }
    self.pops.tray_drop = false;
    let pt = cursor();
    let mut pins: Vec<String> = self.model.pins.clone().unwrap_or_default();
    pins.retain(|k| *k != d.key);
    let changed = if self.pops.tray.as_ref().is_some_and(|w| w.contains_screen(pt.x, pt.y)) {
      true // into the panel: put away
    } else if let Some(i) = self.bar_at(pt) {
      let r = window_rect(self.bars[i].hwnd);
      let s = self.bars[i].scale;
      let (x, y) = ((pt.x - r.left) as f32 / s, (pt.y - r.top) as f32 / s);
      let frame = &self.bars[i].frame;
      match frame.hit(x, y).map(|h| &h.kind) {
        // in front of a pinned icon
        Some(HitKind::TrayIcon(other)) => {
          let other_key = self.model.tray_icon(other).map(pin_key);
          match other_key.and_then(|k| pins.iter().position(|p| *p == k)) {
            Some(at) => pins.insert(at, d.key.clone()),
            None => pins.push(d.key.clone()),
          }
          true
        }
        _ if frame.tray_zone.contains(x, y) => {
          pins.push(d.key.clone());
          true
        }
        _ => false,
      }
    } else {
      false
    };
    if changed {
      self.save_pins(pins);
    }
    self.tray_render();
    self.redraw_all();
    true
  }

  /// Capture taken away mid-drag (another window, Esc): cancel.
  pub(super) fn drag_cancel(&mut self) {
    if self.pops.drag.take().is_some() {
      if let Some(g) = &mut self.pops.ghost {
        g.hide();
      }
      self.pops.tray_drop = false;
      self.tray_render();
    }
  }

  // ------------------------------------------------------------ tooltips

  fn tip_text(&self, at: &TipAt) -> Option<String> {
    let m = &self.model;
    match at {
      TipAt::Panel(id) => m.tray_icon(id).map(|ic| ic.tooltip.clone()).filter(|t| !t.trim().is_empty()),
      TipAt::Bar(_, kind) => match kind {
        HitKind::Search => Some(m.tr("Arama / Overview (Super)")),
        HitKind::Workspace(n) => {
          let name = n.to_string();
          let big = m.wm.all_workspaces().find(|w| w.name == name).and_then(|w| w.biggest.as_ref());
          Some(match big {
            Some(b) if !b.title.is_empty() => format!("{}: {}", n, b.title),
            _ => name,
          })
        }
        HitKind::Snip => Some(m.tr("Bölge ekran görüntüsü")),
        HitKind::Osk => Some(m.tr("Ekran klavyesi")),
        HitKind::Theme => Some(m.tr("Karanlık / aydınlık")),
        HitKind::Battery(p) => Some(format!("{}%", p)),
        HitKind::TrayIcon(id) => m.tray_icon(id).map(|ic| ic.tooltip.clone()).filter(|t| !t.trim().is_empty()),
        HitKind::TrayMore => Some(m.tr("Diğer simgeler (sürükleyerek taşı)")),
        _ => None,
      },
    }
  }

  /// Hover moved: the tooltip waits for the pointer to rest (600 ms).
  pub(super) fn tip_bar_hover(&mut self, i: usize, hover: Option<&HitKind>) {
    self.tip_candidate(hover.map(|k| TipAt::Bar(i, k.clone())));
  }

  fn tip_candidate(&mut self, at: Option<TipAt>) {
    if at == self.pops.tip_at {
      return;
    }
    if let Some(t) = &mut self.pops.tip {
      if t.shown {
        t.hide();
      }
    }
    self.pops.tip_at = at.filter(|a| self.tip_text(a).is_some());
    unsafe {
      if self.pops.tip_at.is_some() && self.pops.drag.is_none() {
        SetTimer(self.msg_hwnd, TIMER_TIP, 600, None);
      } else {
        let _ = KillTimer(self.msg_hwnd, TIMER_TIP);
      }
    }
  }

  pub(super) fn tip_hide(&mut self) {
    self.tip_candidate(None);
  }

  pub(super) fn tip_show(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_TIP);
    }
    let Some(at) = self.pops.tip_at.clone() else { return };
    let Some(text) = self.tip_text(&at) else { return };
    // anchor: under the bar item, or under the panel cell
    let (scale, cx, top) = match &at {
      TipAt::Bar(i, kind) => {
        let Some(bar) = self.bars.get(*i) else { return };
        let Some(hit) = bar.frame.hits.iter().find(|h| &h.kind == kind) else { return };
        let r = window_rect(bar.hwnd);
        let s = bar.scale;
        (s, r.left + ((hit.rect.x + hit.rect.w / 2.0) * s).round() as i32, r.top + ((view::BAR_H + GAP) * s).round() as i32)
      }
      TipAt::Panel(id) => {
        let Some(w) = &self.pops.tray else { return };
        let Some(h) = w.hits.iter().find(|h| h.kind == PopHit::TrayIcon(id.clone())) else { return };
        let s = w.scale;
        (s, w.origin.0 + ((h.rect.x + h.rect.w / 2.0) * s).round() as i32, w.origin.1 + ((h.rect.bottom() + 4.0) * s).round() as i32)
      }
    };
    let size = match self.measure_painter(|p, _| popup::tip_size(p, &text)) {
      Ok(s) => s,
      Err(_) => return,
    };
    if !Self::ensure(&mut self.pops.tip, &self.gfx, "Logical Lunge · tooltip", scale, Motion::Fade) {
      return;
    }
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, pops, .. } = self;
    let Some(win) = pops.tip.as_mut() else { return };
    if win.resize(gfx, size.0 + 2.0 * PAD, size.1 + 2.0 * PAD).is_err() {
      return;
    }
    let mut requests = Vec::new();
    let _ = win.draw(|dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      let _ = popup::paint_tip(&mut p, theme, &text, size);
      Ok(())
    });
    let x = cx - ((size.0 / 2.0 + PAD) * scale).round() as i32;
    let y = top - (PAD * scale).round() as i32;
    let _ = win.show_at(gfx, x, y);
  }
}
