//! The notification list: Windows' notifications read by the core
//! (`/notifications`, sent again on `ll:notifications`), grouped by app, the
//! newest two of a group, all of them when expanded. A group goes with its
//! ×, a middle click, a drag or a two-finger swipe to the right; one
//! notification with its own × once expanded; "clear all" sends them out one
//! after another from the bottom. Do not disturb and the count below.

use std::{
  collections::{HashMap, HashSet},
  time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use super::{
  super::{
    anim::{Curve, POP_OUT, SPRING_IN},
    core_api,
    gfx::Rect,
    send, Msg, Ui,
  },
  images::{Img, Images},
  kit::{st, stw, Cx},
  Ev, Hit, ScrollId,
};

/// a group dragged this far (of its width) goes
const SWIPE_DISMISS: f32 = 0.33;
const GROUP_GAP: f32 = 5.0;
const LINE: f32 = 18.0;
/// a slight overshoot
const MOVE: Curve = SPRING_IN;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum NHit {
  /// the group's body (drag, right click, middle click)
  Group(String),
  Expand(String),
  Close(String),
  CloseItem(String),
  Dnd,
  ClearAll,
}

#[derive(Clone, Debug)]
pub(super) struct Item {
  pub id: String,
  pub time: i64,
  pub app: String,
  pub aumid: String,
  pub title: String,
  pub body: String,
}

struct Swipe {
  app: String,
  x0: f32,
  y0: f32,
  active: bool,
}

#[derive(Default)]
pub(super) struct Notifs {
  list: Vec<Item>,
  icons: HashMap<String, String>,
  expanded: HashSet<String>,
  /// groups / items on their way out, since
  gone_groups: HashMap<String, Instant>,
  gone_items: HashMap<String, Instant>,
  clearing: Option<(Instant, Vec<String>)>,
  swipe: Option<Swipe>,
  /// how far each group is pulled right, and groups springing back
  dx: HashMap<String, f32>,
  back: HashMap<String, (f32, Instant)>,
  wheel: Option<(String, f32)>,
  /// group widths of the last paint (the swipe threshold)
  width: f32,
  pub loaded: bool,
}

/// "Şimdi", "5dk", "3sa", else the day ("2 Ekim").
fn friendly(ms: i64, now_ms: i64, day: &dyn Fn(i64) -> String, tr: &dyn Fn(&str) -> String) -> String {
  let diff = (now_ms - ms) / 1000;
  if diff < 60 {
    tr("Şimdi")
  } else if diff < 3600 {
    format!("{}{}", diff / 60, tr("dk"))
  } else if diff < 86400 {
    format!("{}{}", diff / 3600, tr("sa"))
  } else {
    day(ms)
  }
}

pub(super) fn now_ms() -> i64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn id_of(v: &Value) -> String {
  match v {
    Value::String(s) => s.clone(),
    Value::Number(n) => n.to_string(),
    _ => String::new(),
  }
}

impl Notifs {
  pub fn set(&mut self, v: &Value) {
    let Some(items) = v["items"].as_array() else { return };
    self.list = items
      .iter()
      .map(|n| Item {
        id: id_of(&n["id"]),
        time: n["time"].as_i64().unwrap_or(0),
        app: n["app"].as_str().unwrap_or("").to_string(),
        aumid: n["aumid"].as_str().unwrap_or("").to_string(),
        title: n["title"].as_str().unwrap_or("").to_string(),
        body: n["body"].as_str().unwrap_or("").to_string(),
      })
      .filter(|n| !n.id.is_empty())
      .collect();
    if let Some(icons) = v["icons"].as_object() {
      for (k, url) in icons {
        if let Some(u) = url.as_str() {
          self.icons.insert(k.clone(), u.to_string());
        }
      }
    }
    self.loaded = true;
  }

  fn visible<'a>(&'a self, dismissed: &[String]) -> Vec<&'a Item> {
    self.list.iter().filter(|n| !dismissed.contains(&n.id)).collect()
  }

  /// Groups in the order of their newest notification.
  fn groups<'a>(&'a self, dismissed: &[String]) -> Vec<(String, Vec<&'a Item>)> {
    let mut out: Vec<(String, Vec<&Item>)> = Vec::new();
    for n in self.visible(dismissed) {
      match out.iter_mut().find(|g| g.0 == n.app) {
        Some(g) => g.1.push(n),
        None => out.push((n.app.clone(), vec![n])),
      }
    }
    out
  }

  pub fn count(&self, dismissed: &[String]) -> usize {
    self.visible(dismissed).len()
  }

  /// The items a group shows: all when expanded, else the newest two still there.
  fn shown<'a>(&self, app: &str, items: &[&'a Item]) -> Vec<&'a Item> {
    if self.expanded.contains(app) {
      items.to_vec()
    } else {
      items.iter().filter(|n| !self.gone_items.contains_key(&n.id)).take(2).copied().collect()
    }
  }

  fn item_h(&self, cx: &mut Cx, n: &Item, w: f32, expanded: bool, multiple: bool) -> anyhow::Result<f32> {
    if !expanded {
      return Ok(LINE);
    }
    let text_w = if multiple { w - (w * 0.55).min(self.summary_w(cx, n)?) - 6.0 - 28.0 } else { w };
    Ok(cx.wrapped_h(&n.body, st(13.0), text_w.max(40.0), 2000.0)?.max(LINE))
  }

  fn summary_w(&self, cx: &mut Cx, n: &Item) -> anyhow::Result<f32> {
    Ok(cx.measure(&n.title, stw(13.0, 550.0))?.ceil())
  }

  fn group_h(&self, cx: &mut Cx, app: &str, items: &[&Item], body_w: f32) -> anyhow::Result<f32> {
    let expanded = self.expanded.contains(app);
    let count = items.iter().filter(|n| !self.gone_items.contains_key(&n.id)).count();
    let multiple = count > 1;
    let mut h = 22.0;
    for n in self.shown(app, items) {
      h += 3.0 + self.item_h(cx, n, body_w, expanded, multiple)?;
    }
    Ok(10.0 + h.max(38.0) + 10.0)
  }

  /// The list and the status row in `r` (sidebar.css `.center`).
  #[allow(clippy::too_many_arguments)]
  pub fn paint(
    &mut self,
    cx: &mut Cx,
    r: Rect,
    dismissed: &[String],
    dnd: bool,
    images: &mut Images,
    scroll: &mut HashMap<ScrollId, f32>,
    day: &dyn Fn(i64) -> String,
    animations: bool,
  ) -> anyhow::Result<()> {
    let list_r = Rect::new(r.x, r.y, r.w, r.h - 40.0 - 5.0);
    self.width = list_r.w;
    let groups = self.groups(dismissed);
    let body_w = list_r.w - 10.0 - 38.0 - 10.0 - 10.0;
    let now = cx.now;
    let now_ms = now_ms();
    if groups.is_empty() {
      // `.notif-empty`: the ghost and "no notifications"
      let cy = list_r.y + list_r.h / 2.0;
      let g = Rect::new(list_r.x + list_r.w / 2.0 - 36.0, cy - 50.0, 72.0, 72.0);
      cx.round(g, 30.0, cx.c.layer2)?;
      cx.icon("notifications_active", g.x + 36.0, g.y + 36.0, 36.0, false, cx.c.outline)?;
      cx.text_center(&cx.tr("Bildirim yok"), Rect::new(list_r.x, g.bottom() + 10.0, list_r.w, 20.0), st(15.0), cx.c.outline)?;
    } else {
      let mut heights = Vec::new();
      for (app, items) in &groups {
        heights.push(self.group_h(cx, app, items, body_w)?);
      }
      let content: f32 = heights.iter().sum::<f32>() + GROUP_GAP * (heights.len() as f32 - 1.0);
      let max = (content - list_r.h).max(0.0);
      let off = scroll.get(&ScrollId::Notifs).copied().unwrap_or(0.0).clamp(0.0, max);
      scroll.insert(ScrollId::Notifs, off);
      cx.region(list_r, ScrollId::Notifs, content, false);
      cx.push_clip(list_r);
      let mut y = list_r.y - off;
      let n = groups.len();
      for (i, ((app, items), h)) in groups.iter().zip(heights).enumerate() {
        if y + h >= list_r.y && y <= list_r.bottom() {
          // how far out: its own exit, clear all (staggered from the bottom), a drag
          let mut out = 0.0f32;
          if let Some(at) = self.gone_groups.get(app) {
            let k = (now.duration_since(*at).as_secs_f32() * 1000.0 / 250.0).min(1.0);
            out = if animations { MOVE.at(k) } else { 1.0 };
            cx.busy |= k < 1.0;
          }
          if let Some((at, _)) = &self.clearing {
            let delay = (n - 1 - i) as f32 * 45.0;
            let k = ((now.duration_since(*at).as_secs_f32() * 1000.0 - delay) / 240.0).clamp(0.0, 1.0);
            out = out.max(if animations { POP_OUT.at(k) } else { 1.0 });
            cx.busy = true;
          }
          let mut dx = self.dx.get(app).copied().unwrap_or(0.0);
          if let Some((from, at)) = self.back.get(app) {
            let k = (now.duration_since(*at).as_secs_f32() * 1000.0 / 250.0).min(1.0);
            dx = from * (1.0 - MOVE.at(k));
            cx.busy |= k < 1.0;
          }
          let shift = dx.max(0.0) + out * list_r.w * 1.1;
          let alpha = (1.0 - dx / list_r.w).max(0.2) * (1.0 - out);
          let gr = Rect::new(list_r.x + shift, y, list_r.w, h);
          self.paint_group(cx, gr, app, items, alpha, body_w, images, now_ms, day)?;
        }
        y += h + GROUP_GAP;
      }
      cx.pop_clip();
    }
    // `.statusrow`: do not disturb, the count, clear all
    let sr = Rect::new(r.x, r.bottom() - 40.0, r.w, 40.0);
    let dnd_r = Rect::new(sr.x, sr.y, 56.0, 40.0);
    let hit = Hit::Notif(NHit::Dnd);
    let (bg, fg) = if dnd { (cx.t.primary, cx.t.on_primary) } else { (if cx.hot(&hit) { cx.c.layer2_hover } else { cx.c.layer2 }, cx.t.on_layer1) };
    cx.round(dnd_r, if cx.down(&hit) { 12.0 } else { 20.0 }, bg)?;
    cx.icon("notifications_paused", dnd_r.x + 28.0, dnd_r.y + 20.0, 22.0, dnd, fg)?;
    cx.hit(dnd_r, hit);
    let mid = Rect::new(dnd_r.right() + 5.0, sr.y, sr.w - 2.0 * 56.0 - 10.0, 40.0);
    cx.round(mid, 20.0, cx.c.layer2)?;
    let label = cx.tr(&format!("{} bildirim", self.count(dismissed)));
    cx.text_center(&label, mid, st(13.0), cx.t.on_surface_variant)?;
    let clr = Rect::new(sr.right() - 56.0, sr.y, 56.0, 40.0);
    let hit = Hit::Notif(NHit::ClearAll);
    cx.round(clr, if cx.down(&hit) { 12.0 } else { 20.0 }, if cx.hot(&hit) { cx.c.layer2_hover } else { cx.c.layer2 })?;
    cx.icon("delete_sweep", clr.x + 28.0, clr.y + 20.0, 22.0, false, cx.t.on_layer1)?;
    cx.hit(clr, hit);
    Ok(())
  }

  #[allow(clippy::too_many_arguments)]
  fn paint_group(&self, cx: &mut Cx, r: Rect, app: &str, items: &[&Item], alpha: f32, body_w: f32, images: &mut Images, now_ms: i64, day: &dyn Fn(i64) -> String) -> anyhow::Result<()> {
    let a = alpha;
    let group_hit = Hit::Notif(NHit::Group(app.to_string()));
    let hot = cx.hover.as_ref().is_some_and(|h| matches!(h, Hit::Notif(NHit::Group(x) | NHit::Expand(x) | NHit::Close(x)) if x == app))
      || cx.hover.as_ref().is_some_and(|h| matches!(h, Hit::Notif(NHit::CloseItem(id)) if items.iter().any(|n| &n.id == id)));
    cx.round(r, 17.0, cx.c.layer2.alpha(a))?;
    cx.hit(r, group_hit);
    // the app's icon (or a chat bubble)
    let ic = Rect::new(r.x + 10.0, r.y + 10.0, 38.0, 38.0);
    cx.round(ic, 19.0, cx.t.sec_container.alpha(a))?;
    let icon_url = items.first().and_then(|n| self.icons.get(&n.aumid)).cloned();
    let mut drew = false;
    if let Some(url) = icon_url {
      if let Some(Img::Still(b)) = images.get(&url, 64, 64) {
        let size = unsafe { b.GetSize() };
        let k = (26.0 / size.width.max(1.0)).min(26.0 / size.height.max(1.0));
        let (w, h) = (size.width * k, size.height * k);
        unsafe {
          cx.p.dc.DrawBitmap(
            b,
            Some(&Rect::new(ic.x + (38.0 - w) / 2.0, ic.y + (38.0 - h) / 2.0, w, h).d2d()),
            a,
            windows::Win32::Graphics::Direct2D::D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            None,
            None,
          );
        }
        drew = true;
      }
    }
    if !drew {
      cx.icon("chat", ic.x + 19.0, ic.y + 19.0, 20.0, false, cx.t.on_sec_container.alpha(a))?;
    }
    let bx = ic.right() + 10.0;
    let expanded = self.expanded.contains(app);
    let count = items.iter().filter(|n| !self.gone_items.contains_key(&n.id)).count();
    let multiple = count > 1;
    // head: name (or the single title), time, count and expand, close
    let head_y = r.y + 10.0;
    let close = Rect::new(r.right() - 10.0 - 22.0, head_y, 22.0, 22.0);
    let count_s = count.to_string();
    let cw = cx.measure(&count_s, st(12.0))?.ceil();
    let exp = Rect::new(close.x - 2.0 - (8.0 + cw + 1.0 + 16.0 + 4.0), head_y, 8.0 + cw + 1.0 + 16.0 + 4.0, 22.0);
    let time = friendly(items.first().map_or(0, |n| n.time), now_ms, day, cx.tr);
    let tw = cx.measure(&time, st(12.0))?.ceil();
    let time_x = exp.x - 6.0 - tw;
    let name = if multiple { app.to_string() } else { items.first().map(|n| if n.title.is_empty() { app.to_string() } else { n.title.clone() }).unwrap_or_default() };
    cx.text(&name, Rect::new(bx, head_y, (time_x - 5.0 - bx).max(10.0), 22.0), st(12.0), cx.c.outline.alpha(a))?;
    cx.text(&time, Rect::new(time_x, head_y, tw + 2.0, 22.0), st(12.0), cx.c.outline.alpha(a))?;
    let eh = Hit::Notif(NHit::Expand(app.to_string()));
    cx.round(exp, 11.0, if cx.hot(&eh) { cx.t.on_layer1.alpha(0.16 * a) } else { cx.c.layer3.alpha(a) })?;
    cx.text(&count_s, Rect::new(exp.x + 8.0, head_y, cw + 1.0, 22.0), st(12.0), cx.t.on_layer1.alpha(a))?;
    cx.icon(if expanded { "expand_less" } else { "expand_more" }, exp.x + 8.0 + cw + 1.0 + 8.0, head_y + 11.0, 16.0, false, cx.t.on_layer1.alpha(a))?;
    cx.hit(exp, eh);
    let ch = Hit::Notif(NHit::Close(app.to_string()));
    if hot {
      if cx.hot(&ch) {
        cx.round(close, 11.0, cx.c.layer3.alpha(a))?;
      }
      cx.icon("close", close.x + 11.0, close.y + 11.0, 16.0, false, cx.t.on_surface_variant.alpha(a))?;
    }
    cx.hit(close, ch);
    // the notifications
    let mut y = head_y + 22.0;
    let shown = self.shown(app, items);
    for (i, n) in shown.iter().enumerate() {
      let h = self.item_h(cx, n, body_w, expanded, multiple)?;
      y += 3.0;
      // an item on its way out fades and moves 40 DIP right
      let (mut ia, mut ix) = (a, 0.0);
      if let Some(at) = self.gone_items.get(&n.id) {
        let k = (cx.now.duration_since(*at).as_secs_f32() * 1000.0 / 200.0).min(1.0);
        ia *= 1.0 - k;
        ix = 40.0 * POP_OUT.at(k);
        cx.busy |= k < 1.0;
      }
      if !expanded && i == 1 && count > 2 {
        ia *= 0.5;
      }
      let row = Rect::new(bx + ix, y, body_w, h);
      let mut tx = row.x;
      let mut text_w = row.w;
      if multiple || !expanded {
        let sw = self.summary_w(cx, n)?.min(row.w * 0.55);
        if sw > 0.0 {
          cx.text(&n.title, Rect::new(row.x, row.y, sw + 1.0, LINE), stw(13.0, 550.0), cx.t.on_layer1.alpha(ia))?;
          tx += sw + 6.0;
          text_w -= sw + 6.0;
        }
      }
      if expanded && multiple {
        text_w -= 28.0;
      }
      let body = if expanded { n.body.clone() } else { n.body.replace(['\n', '\r'], " ") };
      if expanded {
        cx.p.text_wrapped(&body, Rect::new(tx, row.y, text_w.max(10.0), h + 2.0), st(13.0), cx.t.on_surface_variant.alpha(ia), false)?;
      } else {
        cx.text(&body, Rect::new(tx, row.y, text_w.max(10.0), LINE), st(13.0), cx.t.on_surface_variant.alpha(ia))?;
      }
      if expanded && multiple {
        let x = Rect::new(row.right() - 22.0, row.y - 2.0, 22.0, 22.0);
        let xh = Hit::Notif(NHit::CloseItem(n.id.clone()));
        let item_hot = cx.hover.as_ref().is_some_and(|hh| *hh == xh) || cx.mouse_in(row);
        if item_hot {
          if cx.hot(&xh) {
            cx.round(x, 11.0, cx.c.layer3.alpha(ia))?;
          }
          cx.icon("close", x.x + 11.0, x.y + 11.0, 16.0, false, cx.t.on_surface_variant.alpha(ia))?;
        }
        cx.hit(x, xh);
      }
      y += h;
    }
    Ok(())
  }

  // ------------------------------------------------------------------ input

  pub fn press(&mut self, app: &str, x: f32, y: f32) {
    self.swipe = Some(Swipe { app: app.to_string(), x0: x, y0: y, active: false });
    self.back.remove(app);
  }

  /// true while a group follows the pointer
  pub fn drag(&mut self, x: f32, y: f32) -> bool {
    let Some(s) = self.swipe.as_mut() else { return false };
    let (mx, my) = (x - s.x0, y - s.y0);
    if !s.active {
      if mx.abs() < 6.0 || my.abs() > mx.abs() {
        return false;
      }
      s.active = true;
    }
    self.dx.insert(s.app.clone(), mx.max(0.0));
    true
  }

  /// The button went up: Some(app) when its group was dragged far enough.
  pub fn release(&mut self) -> Option<String> {
    let s = self.swipe.take()?;
    if !s.active {
      return None;
    }
    self.settle(&s.app)
  }

  fn settle(&mut self, app: &str) -> Option<String> {
    let x = self.dx.remove(app).unwrap_or(0.0);
    if x > self.width.max(1.0) * SWIPE_DISMISS {
      self.dx.insert(app.to_string(), x);
      return Some(app.to_string());
    }
    if x > 0.0 {
      self.back.insert(app.to_string(), (x, Instant::now()));
    }
    None
  }

  /// A horizontal wheel (touchpad) over a group: it follows the fingers.
  pub fn hwheel(&mut self, app: &str, delta: i32) {
    let dist = self.wheel.as_ref().filter(|(a, _)| a == app).map_or(0.0, |(_, d)| *d);
    // fingers to the right scroll left (negative); Chrome: 120 units = 100 px
    let next = (dist - delta as f32 * 100.0 / 120.0).max(0.0);
    self.wheel = Some((app.to_string(), next));
    self.back.remove(app);
    self.dx.insert(app.to_string(), next);
  }

  /// The wheel stopped (140 ms): Some(app) when it went far enough.
  pub fn wheel_end(&mut self) -> Option<String> {
    let (app, _) = self.wheel.take()?;
    self.settle(&app)
  }

  pub fn toggle_expand(&mut self, app: &str) {
    if !self.expanded.remove(app) {
      self.expanded.insert(app.to_string());
    }
  }

  /// Ids of a group (still shown).
  fn group_ids(&self, app: &str, dismissed: &[String]) -> Vec<String> {
    self.visible(dismissed).into_iter().filter(|n| n.app == app).map(|n| n.id.clone()).collect()
  }

  /// What finished going out: ids to dismiss now.
  pub fn finished(&mut self, now: Instant, dismissed: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let done: Vec<String> = self.gone_groups.iter().filter(|(_, at)| now.duration_since(**at) >= Duration::from_millis(250)).map(|(a, _)| a.clone()).collect();
    for app in done {
      self.gone_groups.remove(&app);
      self.dx.remove(&app);
      out.extend(self.group_ids(&app, dismissed));
    }
    let done: Vec<String> = self.gone_items.iter().filter(|(_, at)| now.duration_since(**at) >= Duration::from_millis(220)).map(|(i, _)| i.clone()).collect();
    for id in done {
      self.gone_items.remove(&id);
      out.push(id);
    }
    if let Some((at, ids)) = &self.clearing {
      let groups = self.groups(dismissed).len().max(1);
      if now.duration_since(*at).as_millis() as usize >= (groups - 1) * 45 + 260 {
        out.extend(ids.clone());
        self.clearing = None;
      }
    }
    out
  }

  pub fn busy(&self) -> bool {
    !self.gone_groups.is_empty() || !self.gone_items.is_empty() || self.clearing.is_some() || !self.back.is_empty()
  }
}

impl<'p, 'a> Cx<'p, 'a> {
  /// The pointer is over `r` (hover without a hit of its own).
  pub fn mouse_in(&self, r: Rect) -> bool {
    self.mouse.is_some_and(|(x, y)| r.contains(x, y))
  }
}

impl Ui {
  pub(super) fn sb_notifs_load(&mut self) {
    std::thread::spawn(|| {
      if let Some((200, body)) = core_api::post("/notifications") {
        if let Ok(v) = serde_json::from_slice::<Value>(&body) {
          send(Msg::Sidebar(Ev::Notifs(v)));
        }
      }
    });
  }

  fn sb_kill_group(&mut self, app: String) {
    if self.model.animations {
      self.sidebar.notifs.gone_groups.insert(app, Instant::now());
      self.sb_frames();
    } else {
      let dismissed = self.sidebar.store.notif_dismissed.clone();
      let ids = self.sidebar.notifs.group_ids(&app, &dismissed);
      self.sidebar.store.dismiss(ids);
      self.sidebar.save_soon();
    }
  }

  fn sb_kill_item(&mut self, id: String) {
    let dismissed = self.sidebar.store.notif_dismissed.clone();
    let n = &self.sidebar.notifs;
    let Some(app) = n.list.iter().find(|x| x.id == id).map(|x| x.app.clone()) else { return };
    let left = n.group_ids(&app, &dismissed).iter().filter(|i| !n.gone_items.contains_key(*i)).count();
    if left <= 1 {
      return self.sb_kill_group(app);
    }
    if self.model.animations {
      self.sidebar.notifs.gone_items.insert(id, Instant::now());
      self.sb_frames();
    } else {
      self.sidebar.store.dismiss([id]);
      self.sidebar.save_soon();
    }
  }

  pub(super) fn sb_notif_click(&mut self, h: NHit, button: u8) {
    match (h, button) {
      (NHit::Group(app), 1) | (NHit::Expand(app), 0) => self.sidebar.notifs.toggle_expand(&app),
      (NHit::Group(app), 2) | (NHit::Close(app), 0) => self.sb_kill_group(app),
      (NHit::CloseItem(id), 0) => self.sb_kill_item(id),
      (NHit::Dnd, 0) => self.sb_set_dnd(!self.model.dnd),
      (NHit::ClearAll, 0) => {
        let dismissed = self.sidebar.store.notif_dismissed.clone();
        let n = &mut self.sidebar.notifs;
        if n.clearing.is_some() || n.count(&dismissed) == 0 {
          return;
        }
        let ids: Vec<String> = n.visible(&dismissed).iter().map(|x| x.id.clone()).collect();
        if self.model.animations {
          n.clearing = Some((Instant::now(), ids));
          self.sb_frames();
        } else {
          self.sidebar.store.dismiss(ids);
          self.sidebar.save_soon();
        }
      }
      _ => {}
    }
    self.sb_render();
  }

  /// A drag or a swipe ended: the group goes or springs back.
  pub(super) fn sb_notif_settled(&mut self, app: Option<String>) {
    if let Some(app) = app {
      self.sb_kill_group(app);
    }
    self.sb_frames();
    self.sb_render();
  }

  /// Every frame: notifications whose exit is over leave the list for good.
  pub(super) fn sb_notifs_frame(&mut self) {
    let dismissed = self.sidebar.store.notif_dismissed.clone();
    let done = self.sidebar.notifs.finished(Instant::now(), &dismissed);
    if !done.is_empty() {
      self.sidebar.store.dismiss(done);
      self.sidebar.save_soon();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn item(id: &str, app: &str) -> Value {
    serde_json::json!({ "id": id, "time": 0, "app": app, "aumid": "x", "title": "t", "body": "b" })
  }

  #[test]
  fn groups_follow_the_newest_and_skip_dismissed() {
    let mut n = Notifs::default();
    n.set(&serde_json::json!({ "items": [item("1", "Mail"), item("2", "Chat"), item("3", "Mail")], "icons": {} }));
    let g = n.groups(&["2".into()]);
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].0, "Mail");
    assert_eq!(g[0].1.len(), 2);
    assert_eq!(n.count(&[]), 3);
  }

  #[test]
  fn numeric_ids_are_read() {
    let mut n = Notifs::default();
    n.set(&serde_json::json!({ "items": [{ "id": 42, "app": "A" }] }));
    assert_eq!(n.list[0].id, "42");
  }

  #[test]
  fn times_read_like_the_web_list() {
    let tr = |s: &str| s.to_string();
    let day = |_: i64| "2 Ekim".to_string();
    assert_eq!(friendly(0, 30_000, &day, &tr), "Şimdi");
    assert_eq!(friendly(0, 5 * 60_000, &day, &tr), "5dk");
    assert_eq!(friendly(0, 3 * 3_600_000, &day, &tr), "3sa");
    assert_eq!(friendly(0, 2 * 86_400_000, &day, &tr), "2 Ekim");
  }

  #[test]
  fn a_short_drag_springs_back_and_a_long_one_dismisses() {
    let mut n = Notifs { width: 400.0, ..Default::default() };
    n.press("Mail", 0.0, 0.0);
    assert!(n.drag(50.0, 2.0));
    assert_eq!(n.release(), None);
    n.press("Mail", 0.0, 0.0);
    n.drag(200.0, 0.0);
    assert_eq!(n.release(), Some("Mail".into()));
  }

  #[test]
  fn a_vertical_move_is_no_drag() {
    let mut n = Notifs::default();
    n.press("Mail", 0.0, 0.0);
    assert!(!n.drag(3.0, 40.0));
  }
}
