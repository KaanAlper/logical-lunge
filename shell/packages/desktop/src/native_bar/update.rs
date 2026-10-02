//! The update card (the web edition's update widget): top right of the
//! primary monitor, above the notification cards. It checks for a new
//! version a minute after the start and every six hours, and when the right
//! panel or the settings ask (`ll:update-check`); it downloads and installs
//! through the core (`lunge.exe --update-*`), which does the work and
//! reports progress. The window never takes the keyboard ("Logical Lunge ·
//! update": the core keeps it above workspace slides and out of focus).

use std::{
  sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
  },
  time::{Duration, Instant},
};

use serde_json::Value;
use windows::Win32::{
  Foundation::{HWND, LPARAM, LRESULT, WPARAM},
  UI::{Controls::WM_MOUSELEAVE, WindowsAndMessaging::*},
};

use super::{
  core_api,
  fonts::TextStyle,
  gfx::{Rect, Rgba},
  popup::{self, Motion, PopWin},
  toast,
  view::{Align, Painter, Theme},
  Msg, Ui, TIMER_UPDATE, TIMER_UPDATE_TICK,
};

const TITLE: &str = "Logical Lunge · update";
const CW: f32 = toast::CW;
/// the window around the card leaves room for its shadow
const M: f32 = popup::PAD;
const RADIUS: f32 = 20.0;
const PAD: f32 = 14.0;
const BADGE: f32 = 52.0;
const BODY_MAX: f32 = 150.0;
/// release notes: about four lines, or a scrolling-free longer view when opened
const NOTES_SHORT: f32 = 70.0;
const NOTES_OPEN: f32 = 340.0;
const FIRST_CHECK_MS: u32 = 60_000;
const RECHECK_MS: u32 = 6 * 3600 * 1000;
/// "you are up to date" goes away by itself
const UPTODATE_FOR: Duration = Duration::from_millis(3600);
/// an install that never restarts the desktop (a permission prompt left
/// open) stops being watched; the panel's button checks again
const INSTALL_WATCH: Duration = Duration::from_secs(600);

/// `lunge.exe --update-check`
#[derive(Clone, Default)]
pub struct Info {
  current: String,
  latest: String,
  tag: String,
  available: bool,
  downloaded: bool,
  size: u64,
  notes: String,
}

impl Info {
  /// The core's answer, or its error.
  fn parse(out: Option<String>) -> Result<Info, String> {
    let v: Value = out
      .as_deref()
      .and_then(|s| serde_json::from_str(s).ok())
      .ok_or_else(|| "Yanıt alınamadı".to_string())?;
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let error = s("error");
    if !error.is_empty() {
      return Err(error);
    }
    Ok(Info {
      current: s("current"),
      latest: s("latest"),
      tag: s("tag"),
      available: v["available"].as_bool() == Some(true),
      downloaded: v["downloaded"].as_bool() == Some(true),
      size: v["size"].as_u64().unwrap_or(0),
      notes: s("notes").trim().to_string(),
    })
  }
}

/// From the worker threads (and the panel's button).
pub enum Event {
  /// manual: the user asked (the card shows "checking" and "up to date")
  Check(bool),
  Checked(bool, Result<Info, String>),
  Progress(u64, u64),
  Downloaded(Result<(), String>),
  InstallFailed(String),
  /// the install was watched long enough (the desktop never restarted)
  Idle,
}

#[derive(Clone, PartialEq)]
enum State {
  Checking,
  UpToDate,
  Available,
  Downloading,
  Ready,
  Installing,
  Error(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Act {
  Download,
  Later,
  Install,
  Retry,
  Close,
  Notes,
}

#[derive(Default)]
pub struct UpdateCard {
  state: Option<State>,
  info: Option<Info>,
  /// a check, download or install is running
  busy: bool,
  win: Option<PopWin>,
  /// card height (DIPs, without the shadow room)
  height: f32,
  hits: Vec<(Rect, Act)>,
  hover: Option<Act>,
  notes_open: bool,
  progress: (u64, u64),
  /// since when the card is on screen (None: waiting for a fullscreen app)
  shown: Option<Instant>,
  closing: Option<Instant>,
  timer: bool,
}

/// What a card shows, its texts already translated.
struct View {
  state: State,
  icon: &'static str,
  title: String,
  ver: String,
  body: String,
  notes: String,
  notes_open: bool,
  more: String,
  /// downloaded / total bytes while downloading
  progress: Option<(u64, u64)>,
  /// the moving bar while installing (0..1)
  phase: Option<f32>,
  pct: String,
  sizes: String,
  buttons: Vec<(String, Act)>,
  hover: Option<Act>,
  light: bool,
}

fn mb(n: u64) -> String {
  let v = n as f64 / 1_048_576.0;
  if n >= 10_485_760 {
    format!("{v:.0}")
  } else {
    format!("{v:.1}")
  }
}

fn dismissed_path() -> std::path::PathBuf {
  super::state_dir().join("update-dismissed")
}

fn send(e: Event) {
  super::send(Msg::Update(e));
}

/// Lays the card out from the top; draws it when `draw` (the first pass only
/// measures: the frame needs the height). Returns the height and the buttons.
fn compose(p: &mut Painter, t: &Theme, v: &View, height: f32, draw: bool) -> anyhow::Result<(f32, Vec<(Rect, Act)>)> {
  let r = Rect::new(M, M, CW, height);
  if draw {
    popup::frame_shadow(p, r, RADIUS)?;
    p.fill_round(r, RADIUS, t.layer0)?;
    p.stroke_round(r, RADIUS, t.border, 1.0)?;
    let ok = matches!(v.state, State::UpToDate | State::Ready);
    let (bg, fg) = match &v.state {
      _ if ok && v.light => (Rgba::hex(0xc7f0d0), Rgba::hex(0x0b3d1a)),
      _ if ok => (Rgba::hex(0x2e5d3a), Rgba::hex(0xc7f0d0)),
      State::Error(_) => (t.error_container, t.on_error_container),
      _ => (t.primary_container, t.on_primary_container),
    };
    let b = Rect::new(r.x + PAD, r.y + PAD, BADGE, BADGE);
    p.fill_round(b, if ok { BADGE / 2.0 } else { 16.0 }, bg)?;
    p.icon(v.icon, b.x + BADGE / 2.0, b.y + BADGE / 2.0, 28.0, true, fg)?;
  }
  let tx = r.x + PAD + BADGE + PAD;
  let tw = r.x + CW - 16.0 - tx;
  let mut hits = Vec::new();

  // title and version on one line
  let title = TextStyle { size: 15.0, weight: 600.0 };
  let small = TextStyle { size: 12.0, weight: 450.0 };
  if draw {
    let w = p.measure(&v.title, title)?.min(tw);
    p.text(&v.title, Rect::new(tx, r.y + PAD, w + 1.0, 20.0), title, t.on_layer0, Align::Left, false)?;
    if !v.ver.is_empty() && w + 8.0 < tw {
      p.text(&v.ver, Rect::new(tx + w + 8.0, r.y + PAD + 2.0, tw - w - 8.0, 18.0), small, t.subtext, Align::Left, true)?;
    }
  }
  let mut y = r.y + PAD + 20.0;

  let body = TextStyle { size: 13.0, weight: 450.0 };
  if !v.body.is_empty() {
    y += 3.0;
    y += if draw {
      p.text_wrapped(&v.body, Rect::new(tx, y, tw, BODY_MAX), body, t.on_surface_variant, false)?
    } else {
      p.measure_wrapped(&v.body, body, tw, BODY_MAX, false)?
    };
  }
  if !v.notes.is_empty() {
    y += 6.0;
    let full = p.measure_wrapped(&v.notes, body, tw, 10_000.0, false)?;
    let max = if v.notes_open { NOTES_OPEN } else { NOTES_SHORT };
    y += if draw {
      p.text_wrapped(&v.notes, Rect::new(tx, y, tw, max), body, t.on_surface_variant, false)?
    } else {
      full.min(max)
    };
    // long notes open inside the card
    if full > NOTES_SHORT + 2.0 {
      y += 4.0;
      let w = p.measure(&v.more, small)? + 32.0;
      let b = Rect::new(tx - 4.0, y, w, 22.0);
      if draw {
        if v.hover == Some(Act::Notes) {
          p.fill_round(b, 11.0, t.sec_container)?;
        }
        p.icon(if v.notes_open { "expand_less" } else { "expand_more" }, b.x + 13.0, b.y + 11.0, 18.0, true, t.primary)?;
        p.text(&v.more, Rect::new(b.x + 24.0, b.y + 3.0, w - 26.0, 18.0), small, t.primary, Align::Left, false)?;
      }
      hits.push((b, Act::Notes));
      y += 22.0;
    }
  }

  if v.progress.is_some() || v.phase.is_some() {
    y += 10.0;
    let track = Rect::new(tx, y, tw, 8.0);
    if draw {
      p.fill_round(track, 4.0, t.sec_container)?;
      let fill = match (v.progress, v.phase) {
        (Some((done, total)), _) if total > 0 => {
          Some((tx, tw * (done as f32 / total as f32).clamp(0.0, 1.0)))
        }
        // indeterminate: a 38 % piece sliding across
        (_, Some(k)) => {
          let x0 = tx + (k * 1.38 - 0.38) * tw;
          let (a, b) = (x0.max(tx), (x0 + 0.38 * tw).min(tx + tw));
          (b > a).then_some((a, b - a))
        }
        _ => None,
      };
      if let Some((x, w)) = fill {
        if w > 0.5 {
          p.fill_round(Rect::new(x, y, w.max(8.0).min(tw), 8.0), 4.0, t.primary)?;
        }
      }
    }
    y += 8.0;
    if v.progress.is_some() {
      y += 5.0;
      if draw {
        p.text(&v.pct, Rect::new(tx, y, tw / 2.0, 16.0), small, t.subtext, Align::Left, true)?;
        let w = p.measure_with(&v.sizes, small, true)?.min(tw / 2.0);
        p.text(&v.sizes, Rect::new(tx + tw - w, y, w + 1.0, 16.0), small, t.subtext, Align::Left, true)?;
      }
      y += 16.0;
    }
  }

  if !v.buttons.is_empty() {
    y += 12.0;
    let label = TextStyle { size: 13.0, weight: 450.0 };
    let mut bx = tx;
    for (i, (text, act)) in v.buttons.iter().enumerate() {
      let w = p.measure(text, label)? + 36.0;
      let b = Rect::new(bx, y, w, 32.0);
      if draw {
        let (bg, fg) = if i == 0 { (t.primary, t.on_primary) } else { (t.sec_container, t.on_sec_container) };
        // pills that square up a little under the pointer
        p.fill_round(b, if v.hover == Some(*act) { 12.0 } else { 16.0 }, bg)?;
        p.text(text, Rect::new(bx + 18.0, y + 6.0, w - 36.0, 20.0), label, fg, Align::Left, false)?;
      }
      hits.push((b, *act));
      bx += w + 8.0;
    }
    y += 32.0;
  }
  Ok((f32::max(PAD + BADGE + PAD, y - r.y + PAD), hits))
}

impl Ui {
  /// After the bar starts: the first check in a minute.
  pub(super) fn update_start(&mut self) {
    unsafe { SetTimer(self.msg_hwnd, TIMER_UPDATE, FIRST_CHECK_MS, None) };
  }

  /// TIMER_UPDATE: the periodic check (then every six hours).
  pub(super) fn update_timer(&mut self) {
    unsafe { SetTimer(self.msg_hwnd, TIMER_UPDATE, RECHECK_MS, None) };
    self.update_event(Event::Check(false));
  }

  pub(super) fn update_event(&mut self, e: Event) {
    let u = &mut self.update;
    match e {
      Event::Check(manual) => {
        if u.busy {
          // a download or install is running: the button shows it again
          if manual && u.state.is_none() {
            u.state = Some(if u.progress.1 > 0 { State::Downloading } else { State::Checking });
            self.update_layout();
          }
          return;
        }
        u.busy = true;
        if manual {
          u.notes_open = false;
          u.state = Some(State::Checking);
          self.update_layout();
        }
        std::thread::spawn(move || {
          send(Event::Checked(manual, Info::parse(core_api::run_core_output(&["--update-check"]))));
        });
      }
      Event::Checked(manual, result) => {
        u.busy = false;
        match result {
          Err(err) if manual => self.update_show(State::Error(err)),
          Err(err) => {
            tracing::debug!("Update check: {}", err);
            self.update_close();
          }
          Ok(info) => {
            let available = info.available;
            let downloaded = info.downloaded;
            let tag = info.tag.clone();
            u.info = Some(info);
            if !available {
              if manual {
                self.update_show(State::UpToDate);
              } else {
                self.update_close();
              }
              return;
            }
            // "later" for this version: the background check stays quiet
            if !manual && std::fs::read_to_string(dismissed_path()).is_ok_and(|d| d.trim() == tag) {
              return;
            }
            // downloaded earlier: straight to "install now"
            self.update_show(if downloaded { State::Ready } else { State::Available });
          }
        }
      }
      Event::Progress(done, total) => {
        u.progress = (done, total);
        if u.state == Some(State::Downloading) {
          self.update_layout();
        }
      }
      Event::Downloaded(result) => {
        u.busy = false;
        match result {
          Ok(()) => {
            u.progress = (1, 1);
            self.update_show(State::Ready);
          }
          Err(err) => self.update_show(State::Error(err)),
        }
      }
      Event::InstallFailed(err) => {
        u.busy = false;
        self.update_show(State::Error(err));
      }
      Event::Idle => u.busy = false,
    }
  }

  fn update_show(&mut self, state: State) {
    self.update.state = Some(state);
    self.update_layout();
  }

  fn update_act(&mut self, act: Act) {
    match act {
      Act::Download => {
        let u = &mut self.update;
        u.busy = true;
        u.progress = (0, 0);
        self.update_show(State::Downloading);
        let done = Arc::new(AtomicBool::new(false));
        let polling = done.clone();
        // each question is a process: the next waits for the answer
        std::thread::spawn(move || {
          while !polling.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(500));
            let Some(out) = core_api::run_core_output(&["--update-status"]) else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&out) else { continue };
            if v["state"] == "downloading" && !polling.load(Ordering::Acquire) {
              send(Event::Progress(v["bytes"].as_u64().unwrap_or(0), v["total"].as_u64().unwrap_or(0)));
            }
          }
        });
        std::thread::spawn(move || {
          let out = core_api::run_core_output(&["--update-download"]);
          done.store(true, Ordering::Release);
          let v: Value = out.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
          send(Event::Downloaded(if v["state"] == "ready" {
            Ok(())
          } else {
            Err(v["error"].as_str().filter(|e| !e.is_empty()).unwrap_or("İndirme tamamlanamadı").to_string())
          }));
        });
      }
      Act::Install => {
        self.update.busy = true;
        self.update_show(State::Installing);
        std::thread::spawn(|| {
          let out = core_api::run_core_output(&["--update-install"]);
          let v: Value = out.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
          if v["state"] == "error" {
            return send(Event::InstallFailed(v["error"].as_str().unwrap_or("").to_string()));
          }
          // The install script writes its state; a refused permission or a
          // failed setup shows here. When it works the desktop restarts and
          // this process ends.
          let until = Instant::now() + INSTALL_WATCH;
          while Instant::now() < until {
            std::thread::sleep(Duration::from_secs(1));
            let Some(out) = core_api::run_core_output(&["--update-status"]) else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&out) else { continue };
            if v["state"] == "error" {
              return send(Event::InstallFailed(v["error"].as_str().unwrap_or("").to_string()));
            }
          }
          send(Event::Idle);
        });
      }
      Act::Later => {
        if let Some(tag) = self.update.info.as_ref().map(|i| i.tag.clone()) {
          let _ = std::fs::write(dismissed_path(), tag);
        }
        self.update_close();
      }
      Act::Retry => self.update_event(Event::Check(true)),
      Act::Close => self.update_close(),
      Act::Notes => {
        self.update.notes_open = !self.update.notes_open;
        self.update_layout();
      }
    }
  }

  fn update_view(&self) -> View {
    let u = &self.update;
    let state = u.state.clone().unwrap_or(State::Checking);
    let tr = |s: &str| self.model.tr(s);
    let info = u.info.clone().unwrap_or_default();
    let (icon, title) = match &state {
      State::Checking => ("sync", "Güncellemeler denetleniyor…"),
      State::UpToDate => ("check_circle", "Güncelsin"),
      State::Available => ("system_update_alt", "Yeni sürüm var"),
      State::Downloading => ("downloading", "İndiriliyor…"),
      State::Ready => ("task_alt", "Güncelleme hazır"),
      State::Installing => ("rocket_launch", "Kuruluyor…"),
      State::Error(_) => ("error", "Güncelleme yapılamadı"),
    };
    let ver = if !info.latest.is_empty() && info.latest != info.current {
      format!("v{}", info.latest)
    } else if !info.current.is_empty() {
      format!("v{}", info.current)
    } else {
      String::new()
    };
    let size = if info.size > 0 { format!(" · {} MB", mb(info.size)) } else { String::new() };
    let body = match &state {
      State::UpToDate => tr(&format!("Logical Lunge v{} · en son sürüm", info.current)),
      State::Available => format!("v{} → v{}{}", info.current, info.latest, size),
      State::Ready => tr(&format!("v{} indirildi. Kurulum masaüstünü kısa süre yeniden başlatır.", info.latest)),
      State::Installing => tr("Windows izin isteyebilir. Masaüstü birazdan yeniden başlayacak."),
      State::Error(err) => tr(err),
      State::Checking | State::Downloading => String::new(),
    };
    let notes = if state == State::Available { info.notes.clone() } else { String::new() };
    let buttons: Vec<(&str, Act)> = match &state {
      State::Available => vec![("Güncelle", Act::Download), ("Daha sonra", Act::Later)],
      State::Ready => vec![("Şimdi kur", Act::Install), ("Daha sonra", Act::Later)],
      State::Error(_) if info.available => vec![("Yeniden dene", Act::Retry), ("Tamam", Act::Close)],
      State::Error(_) => vec![("Tamam", Act::Close)],
      _ => Vec::new(),
    };
    let (done, total) = u.progress;
    let progress = (state == State::Downloading).then_some((done, total));
    let pct = if total > 0 { format!("{}%", (done * 100 / total).min(100)) } else { "0%".to_string() };
    let sizes = if total > 0 { format!("{} / {} MB", mb(done), mb(total)) } else { String::new() };
    let phase = (state == State::Installing).then(|| {
      let ms = u.shown.map_or(0, |s| s.elapsed().as_millis() as u64);
      (ms % 1150) as f32 / 1150.0
    });
    View {
      icon,
      title: tr(title),
      ver,
      body,
      notes,
      notes_open: u.notes_open,
      more: tr(if u.notes_open { "Daha az göster" } else { "Tümünü göster" }),
      progress,
      phase,
      pct,
      sizes,
      buttons: buttons.into_iter().map(|(s, a)| (tr(s), a)).collect(),
      hover: u.hover,
      light: self.model.light,
      state,
    }
  }

  /// Measures, draws and places the card; it waits while a fullscreen app
  /// is in front (as the notifications do).
  pub(super) fn update_layout(&mut self) {
    if self.update.state.is_none() {
      return;
    }
    let (mon, scale) = toast::primary();
    let theme = self.theme();
    let view = self.update_view();
    let Ui { gfx, fonts, res, icons, update: u, .. } = self;
    if u.win.as_ref().is_some_and(|w| (w.scale - scale).abs() > 0.001) {
      if let Some(w) = u.win.as_mut() {
        w.hide();
      }
      u.win = None;
      u.shown = None;
    }
    if u.win.is_none() {
      match PopWin::new(gfx, TITLE, scale, Motion::FromRight) {
        Ok(w) => u.win = Some(w),
        Err(err) => {
          tracing::warn!("Update card: {:?}", err);
          return;
        }
      }
    }
    let mut requests = Vec::new();
    {
      let mut p = Painter { dc: &gfx.dc, gfx, fonts, res, icons, requests: &mut requests };
      u.height = compose(&mut p, &theme, &view, 0.0, false).map(|(h, _)| h).unwrap_or(80.0);
    }
    let Some(win) = u.win.as_mut() else { return };
    if win.resize(gfx, CW + 2.0 * M, u.height + 2.0 * M).is_err() {
      return;
    }
    let mut hits = Vec::new();
    let height = u.height;
    let drawn = win.draw(|dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      match compose(&mut p, &theme, &view, height, true) {
        Ok((_, h)) => hits = h,
        Err(err) => tracing::warn!("Update card: paint: {:?}", err),
      }
      Ok(())
    });
    if let Err(err) = drawn {
      tracing::warn!("Update card: draw: {:?}", err);
    }
    u.hits = hits;
    u.closing = None;
    let waiting = u.shown.is_none() && toast::busy();
    if !waiting {
      let x = mon.right - ((CW + toast::STACK_PAD + M) * scale).round() as i32;
      let top = mon.top + ((toast::TOP + toast::STACK_PAD - M) * scale).round() as i32;
      if win.show_at(gfx, x, top).is_ok() && u.shown.is_none() {
        u.shown = Some(Instant::now());
      }
    }
    unsafe {
      let _ = gfx.dcomp.Commit();
    }
    // the notifications go under it (the web card's stack: 10 around)
    let below = if u.shown.is_some() { u.height + 2.0 * toast::STACK_PAD } else { 0.0 };
    self.update_ticking(true);
    self.toast_update_card(below);
  }

  fn update_ticking(&mut self, on: bool) {
    if on != self.update.timer {
      self.update.timer = on;
      unsafe {
        if on {
          SetTimer(self.msg_hwnd, TIMER_UPDATE_TICK, 100, None);
        } else {
          let _ = KillTimer(self.msg_hwnd, TIMER_UPDATE_TICK);
        }
      }
    }
  }

  fn update_close(&mut self) {
    let u = &mut self.update;
    u.state = None;
    u.hover = None;
    u.hits.clear();
    if let Some(w) = u.win.as_mut() {
      if w.shown && u.closing.is_none() {
        let _ = w.close(&self.gfx);
        u.closing = Some(Instant::now());
      }
    }
    u.shown = None;
    self.toast_update_card(0.0);
  }

  /// Every 100 ms while the card is up: "up to date" goes away, the slide
  /// out ends, a card that waited for a game shows, the install bar moves.
  pub(super) fn update_tick(&mut self) {
    let u = &mut self.update;
    if let Some(since) = u.closing {
      let ms = u.win.as_ref().map_or(0, |w| w.close_ms());
      if since.elapsed() >= Duration::from_millis(ms as u64) {
        if let Some(w) = u.win.as_mut() {
          w.hide();
        }
        u.win = None;
        u.closing = None;
      }
    }
    enum Next {
      Stop,
      Close,
      Draw,
      Nothing,
    }
    let next = match &u.state {
      None if u.closing.is_none() => Next::Stop,
      None => Next::Nothing,
      Some(State::UpToDate) if u.shown.is_some_and(|s| s.elapsed() >= UPTODATE_FOR) && u.hover.is_none() => Next::Close,
      Some(State::Installing) => Next::Draw,
      Some(_) if u.shown.is_none() && !toast::busy() => Next::Draw,
      Some(_) => Next::Nothing,
    };
    match next {
      Next::Stop => self.update_ticking(false),
      Next::Close => self.update_close(),
      Next::Draw => self.update_layout(),
      Next::Nothing => {}
    }
  }

  /// Graphics device rebuilt or monitors changed: drawn again.
  pub(super) fn update_reset(&mut self) {
    let u = &mut self.update;
    if let Some(w) = u.win.as_mut() {
      w.hide();
    }
    u.win = None;
    u.shown = None;
    u.closing = None;
    if u.state.is_some() {
      self.update_layout();
    }
  }

  /// The card window's mouse; None when `hwnd` is not the card.
  pub(super) fn update_msg(&mut self, hwnd: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let w = self.update.win.as_ref()?;
    if w.hwnd != hwnd {
      return None;
    }
    let s = w.scale;
    let at = || ((lp.0 & 0xFFFF) as i16 as f32 / s, ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / s);
    match msg {
      WM_PAINT => unsafe {
        let _ = windows::Win32::Graphics::Gdi::ValidateRect(hwnd, None);
      },
      WM_MOUSEMOVE => {
        super::track_leave(hwnd);
        let (x, y) = at();
        let over = self.update.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a);
        if over != self.update.hover {
          self.update.hover = over;
          self.update_layout();
        }
      }
      WM_MOUSELEAVE => {
        if self.update.hover.take().is_some() {
          self.update_layout();
        }
      }
      WM_LBUTTONUP => {
        let (x, y) = at();
        if let Some(act) = self.update.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a) {
          self.update_act(act);
        }
      }
      WM_MOUSEACTIVATE => return Some(LRESULT(MA_NOACTIVATE as isize)),
      _ => return None,
    }
    Some(LRESULT(0))
  }
}

#[cfg(test)]
mod tests {
  use super::{mb, Info};

  #[test]
  fn sizes_read_like_the_web_card() {
    assert_eq!(mb(5 * 1_048_576 + 524_288), "5.5");
    assert_eq!(mb(42 * 1_048_576), "42");
  }

  #[test]
  fn the_core_answer_or_its_error() {
    let info = Info::parse(Some(r#"{"current":"0.2.9","latest":"0.2.10","tag":"v0.2.10-native-ui","available":true,"downloaded":false,"size":1048576,"notes":" a ","error":""}"#.into())).ok().unwrap();
    assert!(info.available && !info.downloaded);
    assert_eq!((info.latest.as_str(), info.notes.as_str(), info.size), ("0.2.10", "a", 1_048_576));
    assert_eq!(Info::parse(Some(r#"{"error":"offline"}"#.into())).err().as_deref(), Some("offline"));
    assert_eq!(Info::parse(None).err().as_deref(), Some("Yanıt alınamadı"));
  }
}
