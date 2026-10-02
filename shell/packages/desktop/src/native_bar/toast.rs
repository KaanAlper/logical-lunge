//! Notification cards (ii notificationPopup, the web edition's toast
//! widget): top right of the primary monitor under the bar, newest on top.
//! A card slides in from the right and back out, waits while the pointer is
//! on it, and opens its sender or an action's link when clicked. Each card is
//! a topmost window that never takes the keyboard ("Logical Lunge · toast":
//! the core keeps such windows above workspace slides and out of focus).
//!
//! Cards come from the core's event stream (Windows notifications, the
//! core's own messages) and from the widgets' `ll:toast` events.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use windows::{
  core::{Interface, HSTRING},
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      Direct2D::{ID2D1Bitmap1, ID2D1Image},
      DirectComposition::{IDCompositionScaleTransform, IDCompositionVisual2},
      Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY},
    },
    System::Com::Urlmon::URLDownloadToCacheFileW,
    UI::{
      Controls::WM_MOUSELEAVE,
      HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
      Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
      },
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  anim::{Animated, LINEAR},
  core_api,
  fonts::TextStyle,
  gfx::{self, Rect, Rgba},
  icons::data_url_bytes,
  model,
  popup::{self, Motion, PopWin},
  view::{Align, Painter, Theme},
  Msg, Ui, TIMER_TOASTS,
};

const TITLE: &str = "Logical Lunge · toast";
/// a card's width (the web stack: 420 with 10 around)
const CW: f32 = 400.0;
const STACK_PAD: f32 = 10.0;
const GAP: f32 = 8.0;
/// under the bar
const TOP: f32 = 40.0;
const RADIUS: f32 = 17.0;
const BODY_MAX: f32 = 150.0;
/// the window around a card leaves room for its shadow
const M: f32 = popup::PAD;
/// cards on screen at once; an older one beyond makes room
const MAX_CARDS: usize = 6;
/// after the pointer leaves a card
const LINGER: Duration = Duration::from_millis(2000);

#[derive(Clone, Copy, PartialEq)]
enum Kind {
  Info,
  Ok,
  Warn,
  Error,
}

/// The remaining-time bar at the bottom of a card: a pixel of colour the
/// compositor stretches, clipped to the card's rounded shape.
struct Countdown {
  /// clipped to the card (kept with the card; the tree holds it too)
  _holder: IDCompositionVisual2,
  bar: IDCompositionVisual2,
  clip: windows::Win32::Graphics::DirectComposition::IDCompositionRectangleClip,
  scale: IDCompositionScaleTransform,
  /// the bar's width in pixels
  width: Animated,
}

pub struct Card {
  id: u64,
  kind: Kind,
  title: String,
  body: String,
  /// "<sender> · <time>"
  time: String,
  icon: String,
  mono: bool,
  /// (label, link)
  actions: Vec<(String, String)>,
  image: Option<(ID2D1Bitmap1, f32, f32)>,
  /// the core's id of a Windows notification: a click opens its app
  notification: i64,
  timeout: Duration,
  win: Option<PopWin>,
  countdown: Option<Countdown>,
  /// card height (DIPs, without the shadow room)
  height: f32,
  /// shown since (None: waiting, e.g. for a fullscreen game to end)
  shown: Option<Instant>,
  deadline: Instant,
  /// time left, frozen while the pointer is on it
  paused: Option<Duration>,
  /// hidden once the slide out is over
  closing: Option<Instant>,
  hover_action: Option<usize>,
  /// action buttons (card DIPs from the window's top-left)
  hits: Vec<(Rect, usize)>,
}

#[derive(Default)]
pub struct Toasts {
  cards: Vec<Card>,
  next_id: u64,
  /// the update card's height above the stack (DIPs)
  update_h: f32,
  timer: bool,
}

/// The primary monitor (cards go there, as the web widget did) and its scale.
fn primary() -> (RECT, f32) {
  unsafe {
    let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(mon, &mut info);
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    (info.rcMonitor, dx as f32 / 96.0)
  }
}

/// A fullscreen app, a Direct3D game or a presentation is running: new
/// cards wait until it is over (as the core held the web widget, and as
/// Windows holds its own notifications).
fn busy() -> bool {
  matches!(
    unsafe { SHQueryUserNotificationState() },
    Ok(QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
  )
}

/// An image the card shows instead of its icon: a data URL, a local file
/// or a web address (downloaded through the Windows cache). SVG is skipped.
fn load_image(src: &str) -> Option<Vec<u8>> {
  if src.starts_with("data:") {
    return if src.starts_with("data:image/svg") { None } else { data_url_bytes(src) };
  }
  if src.starts_with("http://") || src.starts_with("https://") {
    let mut path = [0u16; 1024];
    unsafe { URLDownloadToCacheFileW(None, &HSTRING::from(src), &mut path, 0, None).ok()? };
    let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
    return std::fs::read(String::from_utf16_lossy(&path[..len])).ok();
  }
  let path = src.strip_prefix("file:///").unwrap_or(src).replace('/', "\\");
  std::fs::read(path).ok()
}

/// The card's height for its texts (DIPs, without the shadow room).
fn measure(p: &mut Painter, c: &Card) -> anyhow::Result<f32> {
  let badge = if c.image.is_some() { 52.0 } else { 42.0 };
  let tw = CW - (12.0 + badge + 12.0) - 14.0;
  let mut text = 20.0;
  if !c.body.is_empty() {
    text += 3.0 + p.measure_wrapped(&c.body, body_style(c.mono), tw, BODY_MAX, c.mono)?;
  }
  if !c.actions.is_empty() {
    text += 6.0 + 30.0;
  }
  Ok(12.0 + f32::max(badge, text) + 12.0)
}

fn body_style(mono: bool) -> TextStyle {
  TextStyle { size: if mono { 12.0 } else { 13.0 }, weight: 450.0 }
}

/// Draws a card into its window's surface; returns the action buttons.
fn paint(p: &mut Painter, t: &Theme, c: &Card) -> anyhow::Result<Vec<(Rect, usize)>> {
  let r = Rect::new(M, M, CW, c.height);
  popup::frame_shadow(p, r, RADIUS)?;
  p.fill_round(r, RADIUS, t.layer0)?;
  p.stroke_round(r, RADIUS, t.border, 1.0)?;

  let (bx, by) = (r.x + 12.0, r.y + 12.0);
  let badge = match &c.image {
    Some((bmp, w, h)) => {
      let image: ID2D1Image = bmp.cast()?;
      p.image_round(&image, *w, *h, Rect::new(bx, by, 52.0, 52.0), 12.0, 1.0)?;
      52.0
    }
    None => {
      let (bg, fg) = match c.kind {
        Kind::Ok => (t.primary_container, t.on_primary_container),
        Kind::Error => (t.error_container, t.on_error_container),
        Kind::Info | Kind::Warn => (t.sec_container, t.on_sec_container),
      };
      p.fill_round(Rect::new(bx, by, 42.0, 42.0), 21.0, bg)?;
      p.icon(&c.icon, bx + 21.0, by + 21.0, 22.0, true, fg)?;
      42.0
    }
  };

  let tx = bx + badge + 12.0;
  let tw = r.right() - 14.0 - tx;
  let small = TextStyle { size: 12.0, weight: 450.0 };
  let time_w = p.measure(&c.time, small)?.min(tw * 0.45);
  p.text(&c.time, Rect::new(tx + tw - time_w, r.y + 12.0, time_w, 20.0), small, t.subtext, Align::Left, false)?;
  let title = TextStyle { size: 15.0, weight: 550.0 };
  p.text(&c.title, Rect::new(tx, r.y + 12.0, (tw - time_w - 8.0).max(0.0), 20.0), title, t.on_layer0, Align::Left, false)?;

  let mut y = r.y + 12.0 + 20.0;
  if !c.body.is_empty() {
    y += 3.0;
    y += p.text_wrapped(&c.body, Rect::new(tx, y, tw, BODY_MAX), body_style(c.mono), t.on_surface_variant, c.mono)?;
  }
  let mut hits = Vec::new();
  if !c.actions.is_empty() {
    y += 6.0;
    let label = TextStyle { size: 13.0, weight: 450.0 };
    let mut ax = tx;
    for (i, (text, _)) in c.actions.iter().enumerate() {
      let w = p.measure(text, label)? + 28.0;
      let b = Rect::new(ax, y, w, 30.0);
      let (bg, fg) = if i == 0 { (t.primary, t.on_primary) } else { (t.sec_container, t.on_sec_container) };
      // pills that square up a little under the pointer
      p.fill_round(b, if c.hover_action == Some(i) { 10.0 } else { 15.0 }, bg)?;
      p.text(text, Rect::new(ax + 14.0, y + 5.0, w - 28.0, 20.0), label, fg, Align::Left, false)?;
      hits.push((b, i));
      ax += w + 6.0;
    }
  }
  Ok(hits)
}

fn str_of(v: &Value, key: &str) -> String {
  v[key].as_str().unwrap_or("").to_string()
}

impl Ui {
  /// A card from the core or a widget (`ll:toast`); none while "do not
  /// disturb" is on.
  pub(super) fn toast_add(&mut self, v: Value) {
    if self.model.dnd {
      return;
    }
    let kind = match v["kind"].as_str() {
      Some("ok") => Kind::Ok,
      Some("warn") => Kind::Warn,
      Some("error") => Kind::Error,
      _ => Kind::Info,
    };
    let tr = |s: &str| self.model.tr(s);
    let title = tr(&str_of(&v, "title"));
    let body = tr(&str_of(&v, "body"));
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let clock = model::clock_at(now, self.model.hour12);
    let app = str_of(&v, "app");
    let time = if app.is_empty() { clock } else { format!("{app} · {clock}") };
    let icon = v["icon"].as_str().filter(|s| !s.is_empty()).unwrap_or("info").to_string();
    let actions = v["actions"]
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(|a| Some((tr(a["label"].as_str()?), a["url"].as_str().unwrap_or("").to_string())))
      .collect();
    // a card's own time (song recognised: long, copied: short), else the settings'
    let secs = if matches!(kind, Kind::Error | Kind::Warn) { self.model.toast_error } else { self.model.toast_info };
    let timeout = match v["timeout"].as_u64() {
      Some(ms) if ms > 0 => Duration::from_millis(ms.min(120_000)),
      _ => Duration::from_secs(secs as u64),
    };
    let id = self.toasts.next_id;
    self.toasts.next_id += 1;
    let image = str_of(&v, "image");
    if !image.is_empty() {
      std::thread::spawn(move || super::send(Msg::ToastImage(id, load_image(&image))));
    }
    self.toasts.cards.insert(
      0,
      Card {
        id,
        kind,
        title,
        body,
        time,
        icon,
        mono: v["mono"].as_bool() == Some(true),
        actions,
        image: None,
        notification: v["notification"].as_i64().unwrap_or(0),
        timeout,
        win: None,
        countdown: None,
        height: 0.0,
        shown: None,
        deadline: Instant::now() + timeout,
        paused: None,
        closing: None,
        hover_action: None,
        hits: Vec::new(),
      },
    );
    // the oldest beyond the screen's share make room
    let now = Instant::now();
    for c in self.toasts.cards.iter_mut().skip(MAX_CARDS) {
      if c.closing.is_none() {
        c.closing = Some(now);
        if let Some(w) = c.win.as_mut() {
          let _ = w.close(&self.gfx);
        }
      }
    }
    self.toasts_layout();
  }

  /// A card's image arrived (or could not be read: the icon stays).
  pub(super) fn toast_image(&mut self, id: u64, bytes: Option<Vec<u8>>) {
    let Some(bytes) = bytes else { return };
    let Ok(bmp) = self.gfx.bitmap(&bytes) else { return };
    let size = unsafe { bmp.GetSize() };
    let Some(c) = self.toasts.cards.iter_mut().find(|c| c.id == id) else { return };
    c.image = Some((bmp, size.width, size.height));
    self.toasts_layout();
  }

  /// The update card's height (it sits above the notifications).
  pub(super) fn toast_update_card(&mut self, height: f32) {
    self.toasts.update_h = height.max(0.0);
    self.toasts_layout();
  }

  /// Measures, draws and places every card; shows the new ones unless a
  /// fullscreen app is in front.
  pub(super) fn toasts_layout(&mut self) {
    let (mon, scale) = primary();
    let blocked = busy();
    let theme = self.theme();
    let mut y = TOP + STACK_PAD + if self.toasts.update_h > 0.0 { self.toasts.update_h - 12.0 } else { 0.0 };
    let Ui { gfx, fonts, res, icons, toasts, .. } = self;
    for c in toasts.cards.iter_mut() {
      if c.closing.is_some() {
        continue;
      }
      if c.win.as_ref().is_some_and(|w| (w.scale - scale).abs() > 0.001) {
        c.win = None;
        c.countdown = None;
      }
      if c.win.is_none() {
        match PopWin::new(gfx, TITLE, scale, Motion::FromRight) {
          Ok(w) => c.win = Some(w),
          Err(err) => {
            tracing::warn!("Notification card: {:?}", err);
            continue;
          }
        }
      }
      let mut requests = Vec::new();
      {
        let mut p = Painter { dc: &gfx.dc, gfx, fonts, res, icons, requests: &mut requests };
        c.height = measure(&mut p, c).unwrap_or(64.0);
      }
      if c.win.as_mut().is_some_and(|w| w.resize(gfx, CW + 2.0 * M, c.height + 2.0 * M).is_err()) {
        continue;
      }
      let mut hits = Vec::new();
      if let Some(win) = c.win.as_ref() {
        let card = &*c;
        let drawn = win.draw(|dc| {
          let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
          match paint(&mut p, &theme, card) {
            Ok(h) => hits = h,
            Err(err) => tracing::warn!("Notification card: paint: {:?}", err),
          }
          Ok(())
        });
        if let Err(err) = drawn {
          tracing::warn!("Notification card: draw: {:?}", err);
        }
      }
      c.hits = hits;
      let Some(win) = c.win.as_mut() else { continue };
      let x = mon.right - ((CW + STACK_PAD + M) * scale).round() as i32;
      let top = mon.top + ((y - M) * scale).round() as i32;
      y += c.height + GAP;
      if c.shown.is_none() && blocked {
        continue;
      }
      let first = c.shown.is_none();
      if win.show_at(gfx, x, top).is_err() {
        continue;
      }
      if first {
        let now = Instant::now();
        c.shown = Some(now);
        c.deadline = now + c.timeout;
      }
      let color = if c.kind == Kind::Error { theme.error } else { theme.primary };
      if let Err(err) = Self::countdown(gfx, c, scale, color.alpha(0.6)) {
        tracing::debug!("Notification card: countdown: {:?}", err);
      }
    }
    unsafe {
      let _ = gfx.dcomp.Commit();
    }
    if !toasts.timer && !toasts.cards.is_empty() {
      toasts.timer = true;
      unsafe { SetTimer(self.msg_hwnd, TIMER_TOASTS, 100, None) };
    }
  }

  /// Places the remaining-time bar on the card (made once) and runs it.
  fn countdown(gfx: &gfx::Gfx, c: &mut Card, s: f32, color: Rgba) -> windows::core::Result<()> {
    let Some(win) = c.win.as_ref() else { return Ok(()) };
    let (x, y, w, h) = (M * s, M * s, CW * s, c.height * s);
    unsafe {
      if c.countdown.is_none() {
        let holder = gfx.dcomp.CreateVisual()?;
        let clip = gfx.dcomp.CreateRectangleClip()?;
        holder.SetClip(&clip)?;
        let bar = gfx.dcomp.CreateVisual()?;
        let pixel = gfx.surface(1, 1)?;
        gfx::draw_surface(&pixel, 1.0, |dc| {
          dc.Clear(Some(&color.into()));
          Ok(())
        })?;
        bar.SetContent(&pixel)?;
        let scale = gfx.dcomp.CreateScaleTransform()?;
        scale.SetScaleY2(3.0 * s)?;
        bar.SetTransform(&scale)?;
        holder.AddVisual(&bar, false, None)?;
        win.add_above(&holder)?;
        let mut width = Animated::new(w);
        width.set(w);
        scale.SetScaleX2(w)?;
        c.countdown = Some(Countdown { _holder: holder, bar, clip, scale, width });
      }
      let Some(cd) = c.countdown.as_mut() else { return Ok(()) };
      // the card's rounded shape (it may have grown: an image arrived)
      let r = RADIUS * s;
      cd.clip.SetLeft2(x)?;
      cd.clip.SetTop2(y)?;
      cd.clip.SetRight2(x + w)?;
      cd.clip.SetBottom2(y + h)?;
      cd.clip.SetTopLeftRadiusX2(r)?;
      cd.clip.SetTopLeftRadiusY2(r)?;
      cd.clip.SetTopRightRadiusX2(r)?;
      cd.clip.SetTopRightRadiusY2(r)?;
      cd.clip.SetBottomLeftRadiusX2(r)?;
      cd.clip.SetBottomLeftRadiusY2(r)?;
      cd.clip.SetBottomRightRadiusX2(r)?;
      cd.clip.SetBottomRightRadiusY2(r)?;
      cd.bar.SetOffsetX2(x)?;
      cd.bar.SetOffsetY2(y + h - 3.0 * s)?;
      // running down to nothing by the deadline (frozen while paused)
      if c.paused.is_none() {
        if let Some(shown) = c.shown {
          let left = c.deadline.saturating_duration_since(Instant::now());
          let total = c.deadline.saturating_duration_since(shown).max(Duration::from_millis(1));
          if cd.width.target() > 0.5 {
            cd.width.set(w * left.as_secs_f32() / total.as_secs_f32());
            if let Some(a) = cd.width.to(&gfx.dcomp, 0.0, left.as_secs_f32() * 1000.0, LINEAR)? {
              cd.scale.SetScaleX(&a)?;
            }
          }
        }
      }
    }
    Ok(())
  }

  /// Every 100 ms while there are cards: deadlines, the end of slides out,
  /// cards that waited for a fullscreen app.
  pub(super) fn toasts_tick(&mut self) {
    let now = Instant::now();
    let mut changed = false;
    let gfx = &self.gfx;
    for c in self.toasts.cards.iter_mut() {
      match c.closing {
        None => {
          if c.shown.is_some() && c.paused.is_none() && now >= c.deadline {
            c.closing = Some(now);
            if let Some(w) = c.win.as_mut() {
              let _ = w.close(gfx);
            }
          }
        }
        Some(since) => {
          let ms = c.win.as_ref().map_or(0, |w| w.close_ms());
          if now.duration_since(since) >= Duration::from_millis(ms as u64) {
            if let Some(w) = c.win.as_mut() {
              w.hide();
            }
            c.win = None;
            c.countdown = None;
            changed = true;
          }
        }
      }
    }
    let before = self.toasts.cards.len();
    self.toasts.cards.retain(|c| c.win.is_some() || c.closing.is_none());
    // a card waiting for a fullscreen app is drawn once it is gone
    let waiting = self.toasts.cards.iter().any(|c| c.shown.is_none() && c.closing.is_none());
    let free = waiting && !busy();
    if changed || before != self.toasts.cards.len() || free {
      self.toasts_layout();
    }
    if self.toasts.cards.is_empty() && self.toasts.timer {
      self.toasts.timer = false;
      unsafe {
        let _ = KillTimer(self.msg_hwnd, TIMER_TOASTS);
      }
    }
  }

  /// Graphics device rebuilt or monitors changed: the cards are drawn again.
  pub(super) fn toasts_reset(&mut self) {
    for c in self.toasts.cards.iter_mut() {
      if let Some(w) = c.win.as_mut() {
        w.hide();
      }
      c.win = None;
      c.countdown = None;
      c.shown = None;
    }
    self.toasts.cards.retain(|c| c.closing.is_none());
    if !self.toasts.cards.is_empty() {
      self.toasts_layout();
    }
  }

  /// A card window's mouse: hover pauses it, a click runs an action or opens
  /// the sender. None when `hwnd` is no card.
  pub(super) fn toast_msg(&mut self, hwnd: HWND, msg: u32, _wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let i = self.toasts.cards.iter().position(|c| c.win.as_ref().is_some_and(|w| w.hwnd == hwnd))?;
    match msg {
      WM_PAINT => unsafe {
        let _ = windows::Win32::Graphics::Gdi::ValidateRect(hwnd, None);
      },
      WM_MOUSEMOVE => {
        super::track_leave(hwnd);
        let (x, y) = self.toast_point(i, lp);
        let gfx = &self.gfx;
        let c = &mut self.toasts.cards[i];
        // waits while the pointer is on it (the bar stops too)
        if c.closing.is_none() && c.shown.is_some() && c.paused.is_none() {
          c.paused = Some(c.deadline.saturating_duration_since(Instant::now()));
          if let Some(cd) = c.countdown.as_mut() {
            let now = cd.width.current();
            cd.width.set(now);
            unsafe {
              let _ = cd.scale.SetScaleX2(now);
              let _ = gfx.dcomp.Commit();
            }
          }
        }
        let over = c.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a);
        let moved = over != c.hover_action;
        c.hover_action = over;
        if moved {
          self.toasts_layout();
        }
      }
      WM_MOUSELEAVE => {
        let gfx = &self.gfx;
        let c = &mut self.toasts.cards[i];
        if c.closing.is_none() && c.paused.take().is_some() {
          // two more seconds, as the web cards did
          c.deadline = Instant::now() + LINGER;
          if let Some(cd) = c.countdown.as_mut() {
            if let Ok(Some(a)) = cd.width.to(&gfx.dcomp, 0.0, LINGER.as_secs_f32() * 1000.0, LINEAR) {
              unsafe {
                let _ = cd.scale.SetScaleX(&a);
                let _ = gfx.dcomp.Commit();
              }
            }
          }
        }
        let hovered = c.hover_action.take().is_some();
        if hovered {
          self.toasts_layout();
        }
      }
      WM_LBUTTONUP => {
        let (x, y) = self.toast_point(i, lp);
        let c = &self.toasts.cards[i];
        let action = c.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a);
        let url = action.and_then(|a| c.actions.get(a)).map(|(_, url)| url.clone()).filter(|u| !u.is_empty());
        let notification = c.notification;
        match (action, url) {
          (Some(_), Some(url)) => {
            std::thread::spawn(move || core_api::run_core(&["--open", &url]));
          }
          (Some(_), None) => {}
          (None, _) if notification != 0 => {
            std::thread::spawn(move || {
              let _ = core_api::post(&format!("/notification-open?id={notification}"));
            });
          }
          (None, _) => {}
        }
        let gfx = &self.gfx;
        let c = &mut self.toasts.cards[i];
        if c.closing.is_none() {
          c.closing = Some(Instant::now());
          if let Some(w) = c.win.as_mut() {
            let _ = w.close(gfx);
          }
        }
      }
      WM_MOUSEACTIVATE => return Some(LRESULT(MA_NOACTIVATE as isize)),
      _ => return None,
    }
    Some(LRESULT(0))
  }

  /// The mouse position in card DIPs.
  fn toast_point(&self, i: usize, lp: LPARAM) -> (f32, f32) {
    let s = self.toasts.cards[i].win.as_ref().map_or(1.0, |w| w.scale);
    let x = (lp.0 & 0xFFFF) as i16 as f32 / s;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / s;
    (x, y)
  }
}
