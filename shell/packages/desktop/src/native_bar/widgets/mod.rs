//! Desktop widgets: clock, media, system, weather, agenda and note cards
//! that live on the desktop. They belong to no workspace (the window
//! manager leaves the shell's windows alone), sit right above the desktop
//! icons' window (under every other window; icons stay clickable around
//! them) and are added from the desktop's right-click menu ("Widget ekle").
//! A press moves a widget, its corner resizes it (both on an 8 DIP grid),
//! its right-click menu has its settings and "Kaldır". The layout is saved
//! per monitor in `state\desktop-widgets.json` and kept on screen when the
//! monitors change.
//!
//! One window per widget, drawn like the bar (Direct2D on a
//! DirectComposition surface). A widget is drawn again only when what it
//! shows changes; clocks tick once a second only when they show seconds;
//! nothing is drawn while the session is locked or on a monitor a
//! fullscreen app covers.

mod desktop;
mod edit;
mod input;
pub(super) mod layout;
mod location;
mod picker;
mod shape;
mod paint;
mod policy;
mod weather;

use std::{
  collections::{hash_map::DefaultHasher, HashMap, HashSet},
  hash::{Hash, Hasher},
  os::windows::process::CommandExt,
  path::PathBuf,
  time::{Instant, SystemTime},
};

use windows::{
  core::{w, Interface, HSTRING, PCWSTR},
  Win32::{
    Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      DirectComposition::{IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3},
      DirectWrite::{IDWriteTextLayout, DWRITE_HIT_TEST_METRICS},
      Gdi::{EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, ValidateRect, HDC, HMONITOR, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST},
    },
    System::{
      LibraryLoader::GetModuleHandleW,
      StationsAndDesktops::{CloseDesktop, OpenInputDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS},
      SystemInformation::GetLocalTime,
    },
    UI::{
      Controls::WM_MOUSELEAVE,
      HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
      Input::KeyboardAndMouse::{
        GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE,
        VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
      },
      WindowsAndMessaging::*,
    },
  },
};

use self::{
  desktop::*,
  layout::{ClockStyle, Kind, Spec, Store},
  paint::{Data, Editing, Hit},
  weather::Report,
};
use super::{
  anim::{self, POP_IN},
  core_api,
  gfx::{self, Rect},
  menu::{Item as MenuItem, MenuFocus},
  overview::{clipboard_text, set_clipboard, Edit},
  popup::Temps,
  view::Painter,
  Layer, Msg, Ui, CLASS, TIMER_WIDGETS_SAVE, TIMER_WIDGETS_TICK, TIMER_WIDGETS_WEATHER,
};
use crate::providers::{MediaControlArgs, MediaFunction, ProviderFunction};

const TITLE: PCWSTR = w!("Logical Lunge · widget");
/// the title while a note takes the keyboard (the core raises it by name)
const EDIT_TITLE: &str = "Logical Lunge · widget-edit";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// weather: read again after this long (Open-Meteo updates every 15 min)
const WEATHER_EVERY_S: u64 = 30 * 60;
/// a failed weather read is tried again sooner
const WEATHER_RETRY_S: u64 = 5 * 60;
const TEMPS_FILE: &str = r"C:\Users\Public\lunge-temps.json";

/// Results from worker threads.
pub(super) enum Ev {
  Weather(u64, u64, String, Result<Report, String>),
  PickerOpened(u64, u64, isize),
  PickerClosed(u64, u64),
  LocationSave(u64, u64, layout::Location, std::sync::mpsc::Sender<Result<(), String>>),
  Temps(Option<Temps>),
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
  Note,
}

struct Editor {
  id: u64,
  target: Target,
  edit: Edit,
  /// a high surrogate waiting for its pair (WM_CHAR)
  high: Option<u16>,
}

struct Drag {
  resize: bool,
  /// the cursor where it started (screen pixels)
  from: POINT,
  /// the widget's rectangle then (DIP, from its monitor's work area)
  start: (f32, f32, f32, f32),
  moved: bool,
}

struct Win {
  id: u64,
  hwnd: HWND,
  scale: f32,
  /// the monitor it is on and its work area (pixels)
  device: String,
  work: RECT,
  /// the surface's size (pixels)
  px: (u32, u32),
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  layer: Layer,
  hover: bool,
  tracking: bool,
  hot: Option<Hit>,
  hits: Vec<(Rect, Hit)>,
  drag: Option<Drag>,
  /// what it showed last (skip draws that would look the same)
  repaint: policy::Repaint,
  note_layout: Option<IDWriteTextLayout>,
}

#[derive(Default)]
pub(super) struct Widgets {
  store: Store,
  started: bool,
  wins: Vec<Win>,
  weather: HashMap<u64, (Result<Report, String>, Instant)>,
  weather_busy: HashMap<u64, (u64, String)>,
  weather_token: u64,
  weather_refresh: HashSet<u64>,
  pickers: HashMap<u64, (u64, Option<isize>)>,
  picker_token: u64,
  temps: Option<Temps>,
  temps_busy: bool,
  temps_woken: Option<Instant>,
  todos: Vec<String>,
  todos_seen: Option<SystemTime>,
  editor: Option<Editor>,
  /// the tick's period now (ms; 0: stopped)
  period: u32,
  taskbar_created: u32,
}

fn path() -> PathBuf {
  super::state_dir().join("desktop-widgets.json")
}

fn load() -> Store {
  let p = path();
  match std::fs::read_to_string(&p) {
    Ok(text) => Store::parse(&text).unwrap_or_else(|| {
      // keep the unreadable file for the user instead of writing over it
      let _ = std::fs::rename(&p, p.with_extension("json.bad"));
      tracing::warn!("Desktop widgets: {} unreadable, kept as .json.bad", p.display());
      Store::default()
    }),
    Err(_) => Store::default(),
  }
}

fn save(store: &Store) {
  if let Err(error) = try_save(store) { tracing::warn!("Desktop widgets: save: {error}"); }
}

fn try_save(store: &Store) -> Result<(), String> {
  std::fs::create_dir_all(super::state_dir()).map_err(|e| e.to_string())?;
  let p = path();
  let tmp = p.with_extension("json.tmp");
  std::fs::write(&tmp, store.to_json()).map_err(|e| e.to_string())?;
  std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

fn persist_location(store: &mut Store, id: u64, location: layout::Location, write: impl FnOnce(&Store) -> Result<(), String>) -> Result<(), String> {
  if !location.valid() { return Err("Geçersiz konum".into()); }
  let index = store.widgets.iter().position(|s| s.id == id && s.kind == Kind::Weather).ok_or("Widget kaldırıldı")?;
  let before = store.widgets[index].clone();
  store.widgets[index].save_location(location);
  if let Err(error) = write(store) { store.widgets[index] = before; return Err(error); }
  Ok(())
}

impl Ui {
  // ------------------------------------------------------------ lifecycle

  /// The bars were (re)built: the first time the saved widgets come back,
  /// later they are made again for the new monitors or device.
  pub(super) fn widgets_after_bars(&mut self) {
    if self.widgets.started {
      self.widgets_rebuild();
    } else {
      self.widgets_start();
    }
  }

  /// The saved widgets come back.
  fn widgets_start(&mut self) {
    if self.widgets.started {
      return;
    }
    self.widgets.started = true;
    self.widgets.taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    self.widgets.store = load();
    self.widgets_build();
  }

  /// The monitors changed (or the graphics device came back): every window
  /// is made again on its monitor, kept on screen.
  fn widgets_rebuild(&mut self) {
    if self.widgets.started {
      self.widgets_build();
    }
  }

  fn widgets_build(&mut self) {
    self.widgets_end_edit(true);
    for w in self.widgets.wins.drain(..) {
      unsafe {
        let _ = DestroyWindow(w.hwnd);
      }
    }
    let ids: Vec<u64> = self.widgets.store.widgets.iter().map(|s| s.id).collect();
    for id in ids {
      self.widget_window(id, false);
    }
    self.widgets_schedule();
    self.widgets_weather_due();
  }

  /// Makes the window of widget `id` (on its monitor, clamped) and draws it.
  fn widget_window(&mut self, id: u64, enter: bool) {
    let mons = monitors();
    let Some(spec) = self.widgets.store.widgets.iter_mut().find(|s| s.id == id) else { return };
    let Some(m) = mon_for(&mons, &spec.monitor) else { return };
    let (aw, ah) = area_dip(&m);
    let (x, y, w, h) = shape::clamp(spec.kind, &spec.shape, spec.rect(), aw, ah);
    // on its own monitor the clamped place is kept; shown on the primary one
    // while its monitor is away, it goes back there when it returns
    let own = spec.monitor.is_empty() || spec.monitor.eq_ignore_ascii_case(&m.device);
    if own {
      (spec.x, spec.y, spec.w, spec.h) = (x, y, w, h);
    }
    let scale = crate::native_bar::scale::of_dpi(m.dpi);
    let (px, py) = (m.work.left + (x * scale).round() as i32, m.work.top + (y * scale).round() as i32);
    let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
    let (hwnd, target, root, layer) = match make_window(&self.gfx, px, py, pw, ph) {
      Ok(made) => made,
      Err(err) => {
        tracing::warn!("Desktop widget: window: {:?}", err);
        return;
      }
    };
    shape_region(hwnd, &shape::plan(&spec.shape,w,h), scale);
    self.widgets.wins.push(Win {
      id,
      hwnd,
      scale,
      device: m.device.clone(),
      work: m.work,
      px: (pw, ph),
      _target: target,
      _root: root,
      layer,
      hover: false,
      tracking: false,
      hot: None,
      hits: Vec::new(),
      drag: None,
      repaint: policy::Repaint::default(),
      note_layout: None,
    });
    self.widget_paint(id, true);
    if enter && self.model.animations {
      if let Some(win) = self.widgets.wins.iter().find(|w| w.id == id) {
        let fade = (|| -> windows::core::Result<()> {
          let v: IDCompositionVisual3 = win.layer.visual.cast()?;
          unsafe {
            v.SetOpacity(&anim::build(&self.gfx.dcomp, 0.0, 1.0, 220.0, POP_IN)?)?;
            self.gfx.dcomp.Commit()
          }
        })();
        if let Err(err) = fade {
          tracing::debug!("Desktop widget: entrance: {:?}", err);
        }
      }
    }
    unsafe {
      let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    place_above_desktop(hwnd);
  }

  fn widgets_save_soon(&mut self) {
    unsafe { SetTimer(self.msg_hwnd, TIMER_WIDGETS_SAVE, 400, None) };
  }

  /// The desktop menu's "Widget ekle" list.
  pub(super) fn widgets_add_menu(&self) -> Vec<MenuItem> {
    layout::KINDS.iter().map(|k| MenuItem::new(&format!("widget:{}", k.id()), Some(k.icon()), self.model.tr(k.label()))).collect()
  }

  /// A desktop menu choice ("widget:clock" ...) at a screen point: the new
  /// widget goes on that monitor, in its first free spot.
  pub(super) fn widgets_pick(&mut self, id: &str, at: POINT) -> bool {
    let Some(kind) = id.strip_prefix("widget:").and_then(Kind::from_id) else { return false };
    self.widgets_start();
    let mons = monitors();
    let device = unsafe {
      let hm = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
      let mut info = MONITORINFOEXW::default();
      info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
      if GetMonitorInfoW(hm, &mut info as *mut _ as *mut _).as_bool() {
        let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        String::from_utf16_lossy(&info.szDevice[..end])
      } else {
        String::new()
      }
    };
    let Some(m) = mon_for(&mons, &device) else { return true };
    let (aw, ah) = area_dip(&m);
    let id = self.widgets.store.next_id();
    let mut spec = Spec::new(id, kind, &m.device);
    let taken: Vec<_> = self.widgets.store.widgets.iter().filter(|s| s.monitor.eq_ignore_ascii_case(&m.device)).map(|s| s.rect()).collect();
    (spec.x, spec.y) = layout::free_spot((spec.w, spec.h), &taken, aw, ah);
    self.widgets.store.widgets.push(spec);
    save(&self.widgets.store);
    self.widget_window(id, true);
    self.widgets_schedule();
    if kind == Kind::Weather {
      self.widget_weather(id);
    }
    if kind == Kind::Note {
      self.widgets_begin_edit(id, Target::Note);
    }
    true
  }

  fn widget_remove(&mut self, id: u64) {
    self.widget_close_picker(id);
    if self.widgets.editor.as_ref().is_some_and(|e| e.id == id) {
      self.widgets.editor = None;
    }
    if let Some(i) = self.widgets.wins.iter().position(|w| w.id == id) {
      let w = self.widgets.wins.remove(i);
      unsafe {
        let _ = DestroyWindow(w.hwnd);
      }
    }
    self.widgets.store.widgets.retain(|s| s.id != id);
    self.widgets.weather.remove(&id);
    self.widgets.weather_busy.remove(&id);
    self.widgets.weather_refresh.remove(&id);
    save(&self.widgets.store);
    self.widgets_schedule();
  }

  // ------------------------------------------------------------ data and drawing

  fn spec(&self, id: u64) -> Option<&Spec> {
    self.widgets.store.widgets.iter().find(|s| s.id == id)
  }

  /// After new data (providers, art, the theme): draws the widgets whose
  /// look changed.
  pub(super) fn widgets_refresh(&mut self) {
    if self.widgets.wins.is_empty() || locked() {
      return;
    }
    let ours: Vec<HWND> = self.widgets.wins.iter().map(|w| w.hwnd).collect();
    let editing = self.widgets.editor.as_ref().and_then(|e| self.widgets.wins.iter().find(|w| w.id == e.id)).map(|w| w.hwnd);
    if !layer_ok(&ours, editing) {
      tracing::debug!(?editing, widgets = ours.len(), "Desktop widgets: repairing desktop layer");
      for h in ours.into_iter().filter(|h| Some(*h) != editing) {
        place_above_desktop(h);
      }
    }
    let covered = covered_monitor();
    let ids: Vec<u64> = self
      .widgets
      .wins
      .iter()
      .filter(|w| covered.map_or(true, |c| !(w.work.left >= c.left && w.work.top >= c.top && w.work.right <= c.right && w.work.bottom <= c.bottom)))
      .map(|w| w.id)
      .collect();
    for id in ids {
      self.widget_paint(id, false);
    }
    // media started or stopped playing: the tick's pace follows
    if (self.widgets_fast()) != (self.widgets.period == 1000) {
      self.widgets_schedule();
    }
  }

  fn widgets_fast(&self) -> bool {
    let playing = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()).is_some_and(|s| s.is_playing);
    self.widgets.store.widgets.iter().any(|s| s.per_second() || s.kind == Kind::System || (s.kind == Kind::Media && playing))
  }

  fn clock_now(&self, s: &Spec) -> paint::Clock {
    let t = unsafe { GetLocalTime() };
    let pattern = match (self.model.hour12, s.seconds && s.clock != ClockStyle::Analog) {
      (true, true) => "h:mm:ss tt",
      (true, false) => "h:mm tt",
      (false, true) => "HH:mm:ss",
      (false, false) => "HH:mm",
    };
    paint::Clock {
      h: t.wHour as u32,
      m: t.wMinute as u32,
      s: t.wSecond as u32,
      time: super::model::format_time(pattern),
      date: self.model.format_day(t.wYear as i32, t.wMonth as u32, t.wDay as u32, "dddd, d MMMM"),
    }
  }

  fn system_now(&self) -> paint::System {
    let t = self.widgets.temps.as_ref();
    paint::System {
      cpu: self.model.cpu.as_ref().map(|c| c.usage),
      ram: self.model.memory.as_ref().map(|m| m.usage),
      gpu: t.and_then(|t| t.gpu_load),
      cpu_temp: t.and_then(|t| t.cpu),
      gpu_temp: t.and_then(|t| t.gpu.or(t.gpu_hot)),
    }
  }

  /// A hash of what widget `s` would show now.
  fn widget_key(&self, s: &Spec, win: &Win) -> u64 {
    let mut h = DefaultHasher::new();
    s.id.hash(&mut h);
    format!("{:?}", s).hash(&mut h);
    win.hover.hash(&mut h);
    win.px.hash(&mut h);
    let t = self.theme();
    format!("{:?}{:?}{:?}", t.primary, t.layer0, t.on_layer0).hash(&mut h);
    match s.kind {
      Kind::Clock => {
        let c = self.clock_now(s);
        (c.time, c.date, if s.clock == ClockStyle::Analog { c.s } else { 0 }).hash(&mut h);
      }
      Kind::Media => {
        if let Some(m) = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()) {
          (&m.title, &m.artist, m.is_playing).hash(&mut h);
          let (pos, end, _) = self.media_now();
          if end > 0.0 {
            ((pos / end * 200.0) as i64).hash(&mut h);
          }
          self.media_art().is_some().hash(&mut h);
        }
      }
      Kind::System => {
        let sys = self.system_now();
        let r = |v: Option<f32>| v.map(|x| x.round() as i32);
        (r(sys.cpu), r(sys.ram), r(sys.gpu), r(sys.cpu_temp), r(sys.gpu_temp)).hash(&mut h);
      }
      Kind::Weather => format!("{:?}", self.widgets.weather.get(&s.id).map(|w| &w.0)).hash(&mut h),
      Kind::Agenda => {
        let t = unsafe { GetLocalTime() };
        (t.wYear, t.wMonth, t.wDay, &self.widgets.todos).hash(&mut h);
      }
      Kind::Note => {
        if let Some(e) = self.widgets.editor.as_ref().filter(|e| e.id == s.id) {
          (e.edit.text(), e.edit.caret, e.edit.anchor).hash(&mut h);
        }
      }
    }
    h.finish()
  }

  fn widget_paint(&mut self, id: u64, force: bool) {
    let Some(mut spec) = self.spec(id).cloned() else { return };
    let Some(wi) = self.widgets.wins.iter().position(|w| w.id == id) else { return };
    // A missing monitor may display the saved widget in a smaller work
    // area. Painting uses that actual surface without changing its home.
    spec.w = self.widgets.wins[wi].px.0 as f32 / self.widgets.wins[wi].scale;
    spec.h = self.widgets.wins[wi].px.1 as f32 / self.widgets.wins[wi].scale;
    let key = self.widget_key(&spec, &self.widgets.wins[wi]);
    if !self.widgets.wins[wi].repaint.should_draw(key, force) {
      return;
    }
    let theme = self.theme();
    let clock = self.clock_now(&spec);
    // (title, artist, playing, progress, cover)
    let media = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()).map(|m| {
      let (pos, end, playing) = self.media_now();
      (
        m.title.clone().unwrap_or_default(),
        m.artist.clone().unwrap_or_default(),
        playing,
        (end > 0.0).then(|| (pos / end) as f32),
        self.media_art().cloned(),
      )
    });
    let system = self.system_now();
    let today = unsafe { GetLocalTime() };
    let day_big = today.wDay.to_string();
    let day_line = self.model.format_day(today.wYear as i32, today.wMonth as u32, today.wDay as u32, "dddd\nMMMM yyyy");
    let model = &self.model;
    let tr = |s: &str| model.tr(s);
    let weather = self.widgets.weather.get(&id).map(|w| w.0.clone());
    let ed_text;
    let editing = match self.widgets.editor.as_ref().filter(|e| e.id == id && e.target == Target::Note) {
      Some(e) => {
        ed_text = e.edit.text();
        Some(Editing { text: &ed_text, caret: e.edit.caret, selection: e.edit.selection() })
      }
      None => None,
    };
    let todos = self.widgets.todos.clone();
    let Ui { gfx, fonts, res, icons, widgets, .. } = self;
    let win = &mut widgets.wins[wi];
    let data = Data {
      clock,
      media: media.as_ref().map(|m| paint::Media { title: m.0.clone(), artist: m.1.clone(), playing: m.2, progress: m.3, art: m.4.as_ref() }),
      system,
      weather: weather.as_ref(),
      day_big,
      day_line,
      todos: &todos,
      tr: &tr,
    };
    let mut requests = Vec::new();
    let mut hits = Vec::new();
    let mut note_layout = None;
    let drawn = gfx::draw_surface(&win.layer.surface, win.scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      hits = paint::paint(&mut p, &theme, &spec, &data, win.hover || win.drag.is_some(), editing.as_ref(), &mut note_layout)
        .map_err(|err| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), err.to_string()))?;
      Ok(())
    });
    match drawn.and_then(|_| unsafe { gfx.dcomp.Commit() }) {
      Ok(()) => {
        win.repaint.presented(key);
        win.hits = hits;
        win.note_layout = note_layout;
      },
      Err(err) => tracing::warn!("Desktop widget: draw: {:?}", err),
    }
  }

  // ------------------------------------------------------------ timers and workers

  /// The tick: once a second while something shows seconds or moves
  /// (a seconds clock, playing media, the system gauges), else at the next
  /// minute; none without widgets.
  pub(super) fn widgets_schedule(&mut self) {
    let fast = self.widgets_fast();
    let specs = &self.widgets.store.widgets;
    // game mode: no tick (no temperatures, no redraws) until it ends
    let period = if specs.is_empty() || crate::common::game_mode::on() { 0 } else if fast { 1000 } else { super::ms_to_next_minute() };
    self.widgets.period = period;
    unsafe {
      if period == 0 {
        let _ = KillTimer(self.msg_hwnd, TIMER_WIDGETS_TICK);
        let _ = KillTimer(self.msg_hwnd, TIMER_WIDGETS_WEATHER);
      } else {
        SetTimer(self.msg_hwnd, TIMER_WIDGETS_TICK, period, None);
        if specs.iter().any(|s| s.kind == Kind::Weather) {
          SetTimer(self.msg_hwnd, TIMER_WIDGETS_WEATHER, 60_000, None);
        }
      }
    }
  }

  pub(super) fn widgets_timer(&mut self, id: usize) {
    match id {
      TIMER_WIDGETS_SAVE => {
        unsafe {
          let _ = KillTimer(self.msg_hwnd, TIMER_WIDGETS_SAVE);
        }
        save(&self.widgets.store);
      }
      TIMER_WIDGETS_WEATHER => self.widgets_weather_due(),
      TIMER_WIDGETS_TICK => {
        if !locked() {
          if self.widgets.store.widgets.iter().any(|s| s.kind == Kind::System) {
            self.widgets_read_temps();
          }
          if self.widgets.store.widgets.iter().any(|s| s.kind == Kind::Agenda) {
            self.widgets_read_todos();
          }
          self.widgets_refresh();
        }
        self.widgets_schedule();
      }
      _ => {}
    }
  }

  pub(super) fn widgets_event(&mut self, ev: Ev) {
    match ev {
      Ev::Weather(id, token, query, r) => {
        if !self.widgets.weather_busy.get(&id).is_some_and(|busy| busy.0 == token && busy.1 == query) { return; }
        self.widgets.weather_busy.remove(&id);
        let Some(spec) = self.spec(id) else { return };
        if weather::query(spec, self.model.locale()) != query {
          self.widget_weather(id);
          return;
        }
        if self.widgets.weather_refresh.contains(&id) { self.widget_weather(id); return; }
        if let Err(e) = &r {
          tracing::info!("Desktop widget: weather: {}", e);
        }
        self.widgets.weather.insert(id, (r, Instant::now()));
        self.widget_paint(id, false);
      }
      Ev::PickerOpened(id, token, hwnd) => {
        if let Some(picker) = self.widgets.pickers.get_mut(&id).filter(|p| p.0 == token) {
          picker.1 = Some(hwnd);
        } else { unsafe { let _ = PostMessageW(HWND(hwnd as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0)); } }
      }
      Ev::PickerClosed(id, token) => {
        if self.widgets.pickers.get(&id).is_some_and(|p| p.0 == token) { self.widgets.pickers.remove(&id); }
      }
      Ev::LocationSave(id, token, location, reply) => {
        if !self.widgets.pickers.get(&id).is_some_and(|p| p.0 == token) || !location.valid() {
          let _ = reply.send(Err("Konum seçimi artık etkin değil".into())); return;
        }
        if let Err(error) = persist_location(&mut self.widgets.store, id, location, try_save) {
          let _ = reply.send(Err(error)); return;
        }
        let _ = reply.send(Ok(()));
        self.widgets.weather.remove(&id);
        self.widget_weather(id);
        self.widget_paint(id, true);
      }
      Ev::Temps(t) => {
        self.widgets.temps_busy = false;
        if t.is_some() {
          self.widgets.temps = t;
        }
      }
    }
  }

  fn widgets_weather_due(&mut self) {
    if locked() {
      return;
    }
    let due: Vec<u64> = self
      .widgets
      .store
      .widgets
      .iter()
      .filter(|s| s.kind == Kind::Weather)
      .filter(|s| match self.widgets.weather.get(&s.id) {
        None => true,
        Some((Ok(_), at)) => at.elapsed().as_secs() >= WEATHER_EVERY_S,
        Some((Err(_), at)) => at.elapsed().as_secs() >= WEATHER_RETRY_S,
      })
      .map(|s| s.id)
      .collect();
    for id in due {
      self.widget_weather(id);
    }
  }

  fn widget_weather(&mut self, id: u64) {
    let Some(spec) = self.spec(id) else { return };
    let query = weather::query(spec, self.model.locale());
    if self.widgets.weather_busy.contains_key(&id) { return; }
    self.widgets.weather_token += 1;
    let token = self.widgets.weather_token;
    self.widgets.weather_busy.insert(id, (token, query.clone()));
    let request = weather::request_uri(&query, self.widgets.weather_refresh.remove(&id));
    std::thread::spawn(move || {
      let r = weather::fetch_query(&request);
      super::send(Msg::Widgets(Ev::Weather(id, token, query, r)));
    });
  }

  fn widget_weather_force(&mut self, id: u64) {
    self.widgets.weather_refresh.insert(id);
    self.widget_weather(id);
  }

  fn widget_close_picker(&mut self, id: u64) {
    if let Some((_, Some(hwnd))) = self.widgets.pickers.remove(&id) {
      unsafe { let _ = PostMessageW(HWND(hwnd as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0)); }
    }
  }

  fn widget_location_picker(&mut self, id: u64) {
    if let Some((_, hwnd)) = self.widgets.pickers.get(&id) {
      if let Some(hwnd) = hwnd { unsafe { let _ = SetForegroundWindow(HWND(*hwnd as *mut _)); } }
      return;
    }
    let Some(spec) = self.spec(id).cloned() else { return };
    let Some(win) = self.widgets.wins.iter().find(|w| w.id == id) else { return };
    let owner = win.hwnd.0 as isize;
    let labels = ["Konumu değiştir", "Ülke", "Şehir", "İlçe (isteğe bağlı)", "Sonuçlar · tıklayın veya Enter ile seçin",
      "Tekrar dene", "Son konumlar", "İptal", "Kaydet", "Konumlar yüklenemedi", "Aranıyor…",
      "Konum seçildi · Kaydet ile uygulayın", "Sonuç yok · ülke ve şehir seçin", "Bir sonuç seçin", "Konum kaydedilemedi",
      "Ülke, şehir ve isteğe bağlı ilçe", "Kapat"]
      .iter().map(|label| self.model.tr(label)).collect();
    self.widgets.picker_token += 1;
    let token = self.widgets.picker_token;
    self.widgets.pickers.insert(id, (token, None));
    picker::open(spec, token, owner, self.model.locale().to_string(), labels, self.theme());
  }

  /// The temperature service's file (fresh: written every 2 s); when it is
  /// stale the tool runs once to wake the service, at most every minute.
  fn widgets_read_temps(&mut self) {
    super::pops::want_temps();
    let fresh = std::fs::metadata(TEMPS_FILE).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() < 15);
    if fresh {
      if let Some(t) = std::fs::read_to_string(TEMPS_FILE).ok().and_then(|s| Temps::parse(&s)) {
        self.widgets.temps = Some(t);
      }
      return;
    }
    if self.widgets.temps_busy || self.widgets.temps_woken.is_some_and(|t| t.elapsed().as_secs() < 60) {
      return;
    }
    self.widgets.temps_busy = true;
    self.widgets.temps_woken = Some(Instant::now());
    let exe = tools_dir().join("temps").join("lunge-temps.exe");
    std::thread::spawn(move || {
      let out = std::process::Command::new(exe).arg("--read").creation_flags(CREATE_NO_WINDOW).output();
      let t = out.ok().and_then(|o| Temps::parse(&String::from_utf8_lossy(&o.stdout)));
      super::send(Msg::Widgets(Ev::Temps(t)));
    });
  }

  /// The right panel's to-do list (`state\sidebar.json`), read when the
  /// file changed: the unfinished items.
  fn widgets_read_todos(&mut self) {
    let file = super::state_dir().join("sidebar.json");
    let modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
    if modified.is_some() && modified == self.widgets.todos_seen {
      return;
    }
    self.widgets.todos_seen = modified;
    let v: serde_json::Value = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    self.widgets.todos = v["todo"]
      .as_array()
      .map(|list| {
        list
          .iter()
          .filter(|t| t["done"].as_bool() != Some(true))
          .filter_map(|t| t["content"].as_str().map(str::to_string))
          .filter(|t| !t.trim().is_empty())
          .take(20)
          .collect()
      })
      .unwrap_or_default();
  }
}

#[cfg(test)]
mod persistence_tests {
  use super::*;
  #[test]
  fn failed_location_write_preserves_city_location_and_recents() {
    let mut store = Store::default(); let mut spec = Spec::new(1, Kind::Weather, "");
    spec.city = "Old legacy city".into(); store.widgets.push(spec);
    let before = store.clone();
    let location = layout::Location { country_code: "TR".into(), country: "Türkiye".into(), city: "İzmir".into(), district: String::new(),
      latitude: 38.42, longitude: 27.14, city_latitude: 38.42, city_longitude: 27.14 };
    let error = persist_location(&mut store, 1, location.clone(), |pending| {
      assert_eq!(pending.widgets[0].city, "İzmir"); Err("disk full".into())
    }).unwrap_err();
    assert_eq!(error, "disk full"); assert_eq!(store, before);
    persist_location(&mut store, 1, location.clone(), |_| Ok(())).unwrap();
    assert_eq!(store.widgets[0].location, Some(location)); assert_eq!(store.widgets[0].recent_locations.len(), 1);
  }
}
