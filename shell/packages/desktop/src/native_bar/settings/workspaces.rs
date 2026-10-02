//! The workspaces page (settings.html WorkspacesPage): how many there are,
//! which comes first, and which monitor each is on, either as ranges split
//! by draggable boundaries on a number line or one by one. The core checks
//! and applies (`--set-workspaces count:N first:F name:monitor ...`).

use std::collections::HashMap;

use serde_json::Value;

use super::{
  pages::status_note,
  widgets::{ink, layer2, layer3, ok_color, Btn, Ctrl, Ctx, Cursor, Row, BODY, SUB},
  Act, Fld, Hit, Key, Sel, Settings,
};
use crate::native_bar::{
  fonts::TextStyle,
  gfx::{Rect, Rgba},
  view::Align,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Mon {
  pub index: i64,
  pub name: String,
  pub w: i64,
  pub h: i64,
}

impl Mon {
  pub fn label(&self) -> String {
    format!("{} · {}×{}", self.name, self.w, self.h)
  }
}

#[derive(Default)]
pub struct Ws {
  /// 0: ranges, 1: one by one
  pub mode: usize,
  pub count: i32,
  pub first: i32,
  pub count_text: String,
  pub first_text: String,
  /// workspace names in config order, their monitor binding
  pub names: Vec<i64>,
  pub binds: Vec<Option<i64>>,
  pub mons: Vec<Mon>,
  pub error_text: String,
  /// range boundaries (positions in the order) and each range's monitor
  pub dividers: Vec<usize>,
  pub seg_mon: Vec<i64>,
  /// one by one: name -> monitor index
  pub custom: HashMap<i64, i64>,
  pub busy: bool,
  pub error: String,
  pub saved: bool,
  /// the pointer over the number line (0..1): its lens
  pub hover_pct: Option<f32>,
  /// what the layout was made from (made again when it changes)
  key: String,
}

impl Ws {
  /// New settings from the core.
  pub fn sync(&mut self, s: &Value) {
    let ws = s["workspaces"].as_array().cloned().unwrap_or_default();
    let names: Vec<i64> = ws.iter().map(|w| w["name"].as_str().and_then(|n| n.parse().ok()).or_else(|| w["name"].as_i64()).unwrap_or(0)).collect();
    let binds: Vec<Option<i64>> = ws.iter().map(|w| w["bind_to_monitor"].as_i64()).collect();
    let mons: Vec<Mon> = s["monitors"]
      .as_array()
      .map(|a| {
        a.iter()
          .map(|m| Mon {
            index: m["index"].as_i64().unwrap_or(0),
            name: m["name"].as_str().unwrap_or("").to_string(),
            w: m["w"].as_i64().unwrap_or(0),
            h: m["h"].as_i64().unwrap_or(0),
          })
          .collect()
      })
      .unwrap_or_default();
    // settings.html: the count follows the list's length, the first its first name
    if names.len() != self.names.len() || self.count == 0 {
      self.count = if names.is_empty() { 30 } else { names.len() as i32 };
    }
    if names.first() != self.names.first() || self.first == 0 {
      self.first = names.first().map_or(1, |n| *n as i32);
    }
    self.names = names;
    self.binds = binds;
    self.mons = mons;
    self.error_text = s["workspaceError"].as_str().unwrap_or("").to_string();
    self.clean_fields();
    let key = format!(
      "{}/{}",
      self.names.iter().zip(&self.binds).map(|(n, b)| format!("{n}:{b:?}")).collect::<Vec<_>>().join("|"),
      self.mons.iter().map(|m| m.index.to_string()).collect::<Vec<_>>().join("|")
    );
    if key != self.key {
      self.key = key;
      self.layout();
    }
  }

  pub fn clean_fields(&mut self) {
    self.count_text = self.count.to_string();
    self.first_text = self.first.to_string();
  }

  fn total(&self) -> usize {
    self.names.len()
  }

  /// Old configs numbered monitors from 1: every binding is 1..=n and one is n.
  pub fn legacy(&self) -> bool {
    let n = self.mons.len() as i64;
    n > 0
      && !self.binds.is_empty()
      && self.binds.iter().all(|b| b.is_some_and(|b| (1..=n).contains(&b)))
      && self.binds.iter().any(|b| *b == Some(n))
  }

  /// The names in the order they will be in (from the chosen first).
  pub fn ordered(&self) -> Vec<i64> {
    let n = self.total() as i64;
    let mut v = self.names.clone();
    if n == 0 {
      return v;
    }
    let first = self.first as i64;
    v.sort_by_key(|name| (name - first).rem_euclid(n));
    v
  }

  /// settings.html's layout effect: the current bindings as ranges if they
  /// are ranges, else even ranges; and the one-by-one map.
  fn layout(&mut self) {
    if self.names.is_empty() || self.mons.is_empty() {
      return;
    }
    let legacy = self.legacy();
    let valid: Vec<i64> = self.mons.iter().map(|m| m.index).collect();
    let map: Vec<i64> = self
      .binds
      .iter()
      .map(|b| match b {
        Some(b) if legacy => b - 1,
        Some(b) if valid.contains(b) => *b,
        _ => self.mons[0].index,
      })
      .collect();
    self.custom = self.names.iter().copied().zip(map.iter().copied()).collect();
    let mut boundaries = Vec::new();
    let mut order = vec![map[0]];
    for i in 1..map.len() {
      if map[i] != map[i - 1] {
        boundaries.push(i);
        order.push(map[i]);
      }
    }
    let mut unique = order.clone();
    unique.sort_unstable();
    unique.dedup();
    if boundaries.len() == self.mons.len() - 1 && unique.len() == self.mons.len() {
      self.dividers = boundaries;
      self.seg_mon = order;
    } else {
      let n = self.names.len();
      let m = self.mons.len();
      self.dividers = (1..m).map(|i| (n * i / m).max(1)).collect();
      self.seg_mon = self.mons.iter().map(|m| m.index).collect();
    }
  }

  pub fn step(&mut self, f: Fld, d: i32) {
    let n = self.total() as i32;
    let min_count = self.mons.len() as i32;
    match f {
      Fld::WsCount => self.count = (self.count + d).clamp(min_count, 100),
      Fld::WsFirst => self.first = (self.first + d).clamp(1, n.max(1)),
      _ => {}
    }
    self.clean_fields();
  }

  /// Typing in a number field.
  pub fn fields_changed(&mut self, f: Fld) {
    match f {
      Fld::WsCount => {
        if let Ok(v) = self.count_text.parse::<i32>() {
          self.count = v;
        }
      }
      Fld::WsFirst => {
        if let Ok(v) = self.first_text.parse::<i32>() {
          self.first = v.clamp(1, (self.total() as i32).max(1));
        }
      }
      _ => {}
    }
  }

  fn args(&mut self, count: Option<i32>, first: i32, map: Option<&HashMap<i64, i64>>) -> Option<Vec<String>> {
    if self.busy {
      return None;
    }
    self.busy = true;
    self.error.clear();
    self.saved = false;
    let mut args = vec!["--set-workspaces".to_string()];
    if let Some(c) = count {
      args.push(format!("count:{c}"));
    }
    args.push(format!("first:{first}"));
    if let Some(map) = map {
      let limit = count.unwrap_or(self.total() as i32) as i64;
      let mut entries: Vec<(&i64, &i64)> = map.iter().filter(|(k, _)| **k <= limit).collect();
      entries.sort();
      args.extend(entries.into_iter().map(|(k, v)| format!("{k}:{v}")));
    }
    Some(args)
  }

  pub fn count_args(&mut self) -> Option<Vec<String>> {
    let count = self.count.clamp(self.mons.len() as i32, 100);
    self.count = count;
    self.clean_fields();
    let first = if self.first <= count { self.first } else { 1 };
    self.args(Some(count), first, None)
  }

  pub fn first_args(&mut self) -> Option<Vec<String>> {
    let first = self.first;
    self.args(None, first, None)
  }

  /// The ranges as a map (settings.html applyDivider).
  pub fn divider_map(&self) -> HashMap<i64, i64> {
    let mut map = HashMap::new();
    for (i, name) in self.ordered().into_iter().enumerate() {
      let mut seg = 0;
      while seg < self.dividers.len() && i >= self.dividers[seg] {
        seg += 1;
      }
      let mon = self.seg_mon.get(seg).copied().or_else(|| self.mons.get(seg).map(|m| m.index)).unwrap_or(0);
      map.insert(name, mon);
    }
    map
  }

  pub fn divider_args(&mut self) -> Option<Vec<String>> {
    let map = self.divider_map();
    let first = self.first;
    self.args(None, first, Some(&map))
  }

  pub fn custom_args(&mut self) -> Option<Vec<String>> {
    let map = self.custom.clone();
    let first = self.first;
    self.args(None, first, Some(&map))
  }

  /// A boundary dragged to `k` (0..1 along the line).
  pub fn drag_divider(&mut self, i: usize, k: f32) {
    let total = self.total();
    if i >= self.dividers.len() || total < 2 {
      return;
    }
    let val = (k * total as f32).round() as usize;
    self.set_divider(i, val);
  }

  pub fn nudge_divider(&mut self, i: usize, d: i32) {
    if let Some(v) = self.dividers.get(i).copied() {
      self.set_divider(i, (v as i32 + d).max(0) as usize);
    }
  }

  fn set_divider(&mut self, i: usize, val: usize) {
    let total = self.total();
    let min = if i > 0 { self.dividers[i - 1] + 1 } else { 1 };
    let max = if i + 1 < self.dividers.len() { self.dividers[i + 1] - 1 } else { total - 1 };
    if min <= max {
      self.dividers[i] = val.clamp(min, max);
    }
  }

  pub fn segment_monitor_pos(&self, seg: usize) -> usize {
    let idx = self.seg_mon.get(seg).copied().or_else(|| self.mons.get(seg).map(|m| m.index));
    idx.and_then(|i| self.mons.iter().position(|m| m.index == i)).unwrap_or(0)
  }

  pub fn set_segment_monitor(&mut self, seg: usize, pos: usize) {
    let Some(m) = self.mons.get(pos) else { return };
    while self.seg_mon.len() <= seg {
      let next = self.mons.get(self.seg_mon.len()).map_or(0, |m| m.index);
      self.seg_mon.push(next);
    }
    self.seg_mon[seg] = m.index;
  }

  /// One by one: the next monitor for the `i`th workspace in the order.
  pub fn cycle_custom(&mut self, i: usize) {
    let Some(name) = self.ordered().get(i).copied() else { return };
    if self.mons.is_empty() {
      return;
    }
    let cur = self.custom.get(&name).copied().unwrap_or(self.mons[0].index);
    let pos = self.mons.iter().position(|m| m.index == cur).unwrap_or(0);
    self.custom.insert(name, self.mons[(pos + 1) % self.mons.len()].index);
  }
}

/// settings.html segColors
fn seg_color(cx: &Ctx, i: usize) -> Rgba {
  match i % 6 {
    0 => cx.t.primary,
    1 => Rgba::hex(0x4fc3f7),
    2 => Rgba::hex(0x81c784),
    3 => Rgba::hex(0xffb74d),
    4 => Rgba::hex(0xe57373),
    _ => Rgba::hex(0xba68c8),
  }
}

/// Labels on the number line that do not overlap (settings.html buildLabels).
fn labels(total: usize, dividers: &[usize], width: f32) -> Vec<usize> {
  let mut cand = vec![1, total];
  cand.extend_from_slice(dividers);
  cand.extend((5..total).step_by(5));
  cand.sort_unstable();
  cand.dedup();
  let min_gap = 28.0;
  let mut out: Vec<usize> = Vec::new();
  let mut last = f32::NEG_INFINITY;
  for v in cand {
    let px = v as f32 / total as f32 * width;
    if px - last >= min_gap || v == 1 || v == total {
      if (v == 1 || v == total) && (px - last).abs() < min_gap / 2.0 && !out.is_empty() {
        continue;
      }
      out.push(v);
      last = px;
    }
  }
  out
}

pub fn paint(cx: &mut Ctx, s: &Settings, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let ws = &s.ws;
  let t = cx.t;
  let total = ws.total();
  let nm = ws.mons.len();
  if total == 0 || nm == 0 {
    if s.s.is_null() {
      return Ok(y);
    }
    let text = if ws.error_text.is_empty() {
      cx.tr("Çalışma alanları veya monitörler okunamadı. Pencere yöneticisini kontrol edin.")
    } else {
      cx.tr(&ws.error_text)
    };
    let y = y + 18.0;
    let h = cx.p.measure_wrapped(&text, BODY, w - 40.0, 400.0, false)?;
    cx.p.fill_round(Rect::new(x, y, w, h + 40.0), 20.0, t.layer1)?;
    cx.p.text_wrapped(&text, Rect::new(x + 20.0, y + 20.0, w - 40.0, h + 2.0), BODY, t.error, false)?;
    return Ok(y + h + 40.0);
  }
  let ordered = ws.ordered();
  let mut y = cx.sec_title(x, y, "Genel")?;
  let first_name = ws.names.first().copied().unwrap_or(1);
  let rows = [
    Row::new("grid_view", "Çalışma alanı sayısı".into())
      .sub(format!("{total} alan tanımlı; en az {nm} olmalı."))
      .ctrl(Ctrl::Stepper { text: ws.count_text.clone(), fld: Fld::WsCount, can_dec: !ws.busy && ws.count > nm as i32, can_inc: !ws.busy && ws.count < 100 })
      .ctrl(Ctrl::Button { label: cx.tr("Kaydet"), icon: None, kind: Btn::Tonal, hit: Hit::Act(Act::WsCountSave), disabled: ws.busy || ws.count == total as i32 }),
    Row::new("first_page", "İlk sıradaki çalışma alanı".into())
      .sub("Sıra buradan başlar; alanların adları değişmez.".into())
      .ctrl(Ctrl::Stepper { text: ws.first_text.clone(), fld: Fld::WsFirst, can_dec: !ws.busy && ws.first > 1, can_inc: !ws.busy && ws.first < total as i32 })
      .ctrl(Ctrl::Button { label: cx.tr("Sırayı kaydet"), icon: None, kind: Btn::Tonal, hit: Hit::Act(Act::WsFirstSave), disabled: ws.busy || ws.first as i64 == first_name }),
  ];
  y += cx.card(x, y, w, &rows, 0.0)?;

  // .ws-order: the order preview
  y += 10.0;
  let lbl = cx.tr("Sıra önizlemesi");
  let lw = cx.p.measure(&lbl, SUB)?;
  cx.p.text(&lbl, Rect::new(x + 4.0, y, lw + 2.0, 24.0), SUB, t.on_surface_variant, Align::Left, false)?;
  let (head, tail, hidden) = if ordered.len() > 14 {
    (&ordered[..10], &ordered[ordered.len() - 2..], ordered.len() - 12)
  } else {
    (&ordered[..], &ordered[..0], 0)
  };
  let mut chips: Vec<(String, bool)> = head.iter().enumerate().map(|(i, n)| (n.to_string(), i == 0)).collect();
  if hidden > 0 {
    chips.push((format!("+{hidden}"), false));
  }
  chips.extend(tail.iter().map(|n| (n.to_string(), false)));
  let mut cxp = x + 4.0 + lw + 12.0;
  let chip = TextStyle { size: 12.0, weight: 550.0 };
  for (text, firsty) in chips {
    let cw = (cx.p.measure(&text, chip)? + 6.0).max(24.0);
    if cxp + cw > x + w {
      cxp = x + 4.0 + lw + 12.0;
      y += 28.0;
    }
    let (bg, fg) = if firsty { (t.sec_container, t.on_sec_container) } else { (layer2(t), t.on_layer1) };
    cx.p.fill_round(Rect::new(cxp, y, cw, 24.0), 7.0, bg)?;
    cx.p.text(&text, Rect::new(cxp, y, cw, 24.0), chip, fg, Align::Center, true)?;
    cxp += cw + 4.0;
  }
  y += 24.0;
  if !ws.error.is_empty() {
    y = status_note(cx, x, y, w, "error", &ws.error, t.error)?;
  }
  if ws.saved {
    y = status_note(cx, x, y, w, "check_circle", "Kaydedildi", ok_color(t))?;
  }

  y = cx.sec_title(x, y, "Çoklu Monitör Desteği")?;
  let list = ws.mons.iter().map(|m| m.label()).collect::<Vec<_>>().join(" · ");
  let rows = [
    Row::new("desktop_windows", "Bağlı monitörler".into())
      .sub(if list.is_empty() { "Bilgi alınamadı".into() } else { list })
      .ctrl(Ctrl::Label(nm.to_string(), t.on_layer1.alpha(0.6))),
    Row::new("splitscreen", "Dağıtım şekli".into())
      .sub("Çalışma alanlarını monitörlere nasıl dağıtacağınızı seçin".into())
      .ctrl(Ctrl::Seg(Key::WsMode, vec![(cx.tr("Aralıklar"), Some("align_vertical_center")), (cx.tr("Tek tek"), Some("touch_app"))], ws.mode)),
  ];
  y += cx.card(x, y, w, &rows, 0.0)?;
  if ws.legacy() {
    y = status_note(cx, x, y, w, "info", "Eski monitör numaraları bulundu. Dağılımı kaydettiğinde geçerli monitör indekslerine dönüştürülecek.", t.on_surface_variant)?;
  }

  if ws.mode == 0 {
    y = cx.sec_title(x, y, "Dağılım Çizelgesi")?;
    y = ranges(cx, s, &ordered, x, y, w)?;
  } else {
    y = cx.sec_title(x, y, "Tek tek atama")?;
    y = one_by_one(cx, s, &ordered, x, y, w)?;
  }
  Ok(y)
}

/// The number line with its boundaries, each range's monitor, apply.
fn ranges(cx: &mut Ctx, s: &Settings, ordered: &[i64], x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let ws = &s.ws;
  let t = cx.t;
  let total = ws.total();
  let nm = ws.mons.len();
  let iw = w - 40.0;
  let hint = cx.tr("Ayraçları sürükleyerek sıralı çalışma alanlarını monitörlere bölün.");
  let hint_h = cx.p.measure_wrapped(&hint, SUB, iw, 200.0, false)?;
  let seg_h = 54.0;
  let segs: Vec<(usize, usize)> = (0..nm)
    .map(|i| {
      let start = if i == 0 { 0 } else { ws.dividers.get(i - 1).copied().unwrap_or(0) };
      let end = if i == nm - 1 { total } else { ws.dividers.get(i).copied().unwrap_or(total) };
      (start, end.max(start))
    })
    .collect();
  let line_top = 20.0 + hint_h + 16.0 + 38.0;
  let h = line_top + 52.0 + 4.0 + 12.0 + segs.len() as f32 * (seg_h + 8.0) - 8.0 + 14.0 + 36.0 + 20.0;
  cx.p.fill_round(Rect::new(x, y, w, h), 20.0, t.layer1)?;
  let ix = x + 20.0;
  cx.p.text_wrapped(&hint, Rect::new(ix, y + 20.0, iw, hint_h + 2.0), SUB, t.on_surface_variant, false)?;

  // the number line
  let ly = y + line_top;
  let pct = |v: usize| v as f32 / total as f32;
  let line = Rect::new(ix, ly, iw, 52.0);
  cx.push(line, Hit::WsLine, false, Cursor::Arrow);
  cx.p.fill_round(Rect::new(ix, ly + 14.0, iw, 10.0), 5.0, ink(t, 0.06))?;
  for (i, (start, end)) in segs.iter().enumerate() {
    let (l, r) = (pct(*start) * iw, pct(*end) * iw);
    if r <= l {
      continue;
    }
    let c = seg_color(cx, i);
    let c = c.alpha(0.4);
    let rr = Rect::new(ix + l, ly + 14.0, r - l, 10.0);
    // round only the line's ends
    cx.fill_part_h(rr, 5.0, i == 0, i == nm - 1, c)?;
  }
  for tick in 1..total {
    if ws.dividers.contains(&tick) {
      continue;
    }
    let major = tick % 5 == 0;
    let tx = ix + pct(tick) * iw;
    cx.p.fill(Rect::new(tx, ly + if major { 26.0 } else { 28.0 }, 1.0, if major { 6.0 } else { 3.0 }), ink(t, if major { 0.25 } else { 0.1 }))?;
  }
  let tiny = TextStyle { size: 8.4, weight: 400.0 };
  let tiny_b = TextStyle { size: 8.4, weight: 600.0 };
  for v in labels(total, &ws.dividers, iw) {
    let Some(name) = ordered.get(v - 1) else { continue };
    let is_div = ws.dividers.contains(&v);
    let (st, c) = if is_div { (tiny_b, t.primary.alpha(0.9)) } else { (tiny, ink(t, 0.45)) };
    let tx = ix + pct(v) * iw;
    cx.p.text(&name.to_string(), Rect::new(tx - 20.0, ly + 36.0, 40.0, 12.0), st, c, Align::Center, true)?;
  }
  for (i, v) in ws.dividers.iter().enumerate() {
    let tx = ix + pct(*v) * iw;
    let hit = Hit::WsDivider(i);
    let active = cx.focus == Some(hit) && cx.ring || cx.hover == Some(hit);
    let b = Rect::new(tx - 12.0, ly + 4.0, 24.0, 34.0);
    let bar_w = if active { 6.0 } else { 3.0 };
    cx.p.fill_round(Rect::new(tx - bar_w / 2.0, ly + 4.0, bar_w, 20.0), 2.0, if active { t.on_layer1 } else { ink(t, 0.65) })?;
    let badge = v.to_string();
    let bs = TextStyle { size: 9.1, weight: 600.0 };
    let bw = cx.p.measure(&badge, bs)? + 10.0;
    let br = Rect::new(tx - bw / 2.0, ly + 25.0, bw, 13.0);
    cx.p.fill_round(br, 3.0, if active { t.primary } else { ink(t, 0.12) })?;
    cx.p.text(&badge, br, bs, if active { t.on_primary } else { ink(t, 0.8) }, Align::Center, true)?;
    cx.push(b, hit, true, Cursor::Resize);
    cx.ring(b, 7.0, hit)?;
  }
  // the lens: five names around the pointer
  if let Some(k) = ws.hover_pct {
    let idx = ((k * total as f32).round() as usize).clamp(1, total);
    let half = (110.0f32).min(iw / 2.0);
    let left = (k * iw).clamp(half, iw - half);
    let lr = Rect::new(ix + left - 85.0, ly - 37.0, 170.0, 31.0);
    cx.p.fill_round(lr, 11.0, layer3(t).alpha(0.95))?;
    cx.p.stroke_round(lr, 11.0, t.outline_variant, 1.0)?;
    for (j, off) in (-2i32..=2).enumerate() {
      let at = idx as i32 - 1 + off;
      let name = (at >= 0).then(|| ordered.get(at as usize)).flatten().map_or("·".to_string(), |n| n.to_string());
      let (size, c, weight) = match off.abs() {
        0 => (13.0, t.primary, 700.0),
        1 => (11.9, t.on_surface_variant.alpha(0.8), 450.0),
        _ => (11.0, t.on_surface_variant.alpha(0.58), 450.0),
      };
      let cell = Rect::new(lr.x + 7.0 + j as f32 * 31.2, lr.y, 31.2, lr.h);
      cx.p.text(&name, cell, TextStyle { size, weight }, c, Align::Center, true)?;
    }
  }

  // each range's monitor
  let mut sy = ly + 52.0 + 4.0 + 12.0;
  for (i, (start, end)) in segs.iter().enumerate() {
    let r = Rect::new(ix, sy, iw, seg_h);
    cx.p.fill_round(r, 10.0, layer2(t))?;
    cx.p.fill_round(Rect::new(r.x, r.y, 3.0, r.h), 1.5, seg_color(cx, i))?;
    let a = ordered.get(*start).map_or(String::new(), |n| n.to_string());
    let b = ordered.get(end.saturating_sub(1)).map_or(String::new(), |n| n.to_string());
    let title = cx.tr(&format!("Workspace {a}–{b}"));
    let count = cx.tr(&format!("{} çalışma alanı", end - start));
    let mon = ws.mons.get(ws.segment_monitor_pos(i)).map(|m| m.label()).unwrap_or_default();
    let sel_w = (cx.p.measure(&mon, BODY)? + 52.0).max(200.0).min(iw * 0.55);
    let tw = iw - 28.0 - sel_w - 12.0;
    cx.p.text(&title, Rect::new(r.x + 14.0, r.y + 9.0, tw, 18.0), TextStyle { size: 12.6, weight: 500.0 }, t.on_layer1, Align::Left, false)?;
    cx.p.text(&count, Rect::new(r.x + 14.0, r.y + 28.0, tw, 16.0), TextStyle { size: 11.2, weight: 450.0 }, t.on_surface_variant, Align::Left, false)?;
    cx.select(Rect::new(r.right() - 14.0 - sel_w, r.y + (seg_h - 36.0) / 2.0, sel_w, 36.0), &mon, Hit::Select(Sel::SegMon(i)))?;
    sy += seg_h + 8.0;
  }
  sy += 14.0 - 8.0;
  apply_row(cx, ix, sy, iw, "Dağılımı uygula", Act::WsApplyDivider, ws.busy)?;
  Ok(y + h)
}

/// The apply button, right-aligned (greyed while the core works).
fn apply_row(cx: &mut Ctx, x: f32, y: f32, w: f32, label: &str, act: Act, busy: bool) -> anyhow::Result<()> {
  let label = cx.tr(label);
  let bw = cx.ctrl_width(&Ctrl::Button { label: label.clone(), icon: None, kind: Btn::Primary, hit: Hit::Act(act), disabled: busy })?;
  cx.button(Rect::new(x + w - bw, y, bw, 36.0), &label, None, Btn::Primary, Hit::Act(act), busy)
}

fn one_by_one(cx: &mut Ctx, s: &Settings, ordered: &[i64], x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let ws = &s.ws;
  let t = cx.t;
  let iw = w - 40.0;
  let hint = cx.tr("Kutucuğa tıklayarak monitör seçin. Değişiklikleri birlikte kaydedebilirsiniz.");
  let hint_h = cx.p.measure_wrapped(&hint, SUB, iw, 200.0, false)?;
  // .ws-custom-grid: columns at least 96 wide, 8 apart
  let cols = (((iw + 8.0) / (96.0 + 8.0)).floor() as usize).max(1);
  let cell_w = (iw - (cols - 1) as f32 * 8.0) / cols as f32;
  let rows = ordered.len().div_ceil(cols);
  let grid_h = rows as f32 * (58.0 + 8.0) - 8.0;
  let h = 20.0 + hint_h + 16.0 + grid_h + 16.0 + 36.0 + 20.0;
  cx.p.fill_round(Rect::new(x, y, w, h), 20.0, t.layer1)?;
  let ix = x + 20.0;
  cx.p.text_wrapped(&hint, Rect::new(ix, y + 20.0, iw, hint_h + 2.0), SUB, t.on_surface_variant, false)?;
  let gy = y + 20.0 + hint_h + 16.0;
  for (i, name) in ordered.iter().enumerate() {
    let r = Rect::new(ix + (i % cols) as f32 * (cell_w + 8.0), gy + (i / cols) as f32 * (58.0 + 8.0), cell_w, 58.0);
    let hit = Hit::WsCustom(i);
    let hot = cx.hot(hit);
    let mon_idx = ws.custom.get(name).copied().unwrap_or(ws.mons[0].index);
    let pos = ws.mons.iter().position(|m| m.index == mon_idx).unwrap_or(0);
    let mon = &ws.mons[pos];
    cx.p.fill_round(r, 12.0, if hot { layer3(t) } else { layer2(t) })?;
    cx.p.stroke_round(r, 12.0, if hot { t.primary } else { t.outline_variant }, 1.0)?;
    cx.p.text(&name.to_string(), Rect::new(r.x + 8.0, r.y + 7.0, r.w - 16.0, 20.0), TextStyle { size: 14.0, weight: 700.0 }, t.on_layer1, Align::Left, true)?;
    cx.p.fill_circle(r.x + 8.0 + 3.5, r.bottom() - 7.0 - 8.0, 3.5, seg_color(cx, pos))?;
    cx.p.text(&mon.name, Rect::new(r.x + 8.0 + 12.0, r.bottom() - 7.0 - 16.0, r.w - 28.0, 16.0), TextStyle { size: 11.0, weight: 450.0 }, t.on_surface_variant, Align::Left, false)?;
    cx.push(r, hit, true, Cursor::Hand);
    cx.ring(r, 12.0, hit)?;
  }
  apply_row(cx, ix, gy + grid_h + 16.0, iw, "Atamaları kaydet", Act::WsCustomSave, ws.busy)?;
  Ok(y + h)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn settings(binds: &[Option<i64>], mons: usize) -> Value {
    let ws: Vec<Value> = binds
      .iter()
      .enumerate()
      .map(|(i, b)| match b {
        Some(b) => json!({ "name": (i + 1).to_string(), "bind_to_monitor": b }),
        None => json!({ "name": (i + 1).to_string() }),
      })
      .collect();
    let mons: Vec<Value> = (0..mons).map(|i| json!({ "index": i, "name": format!("M{i}"), "w": 1920, "h": 1080 })).collect();
    json!({ "workspaces": ws, "monitors": mons })
  }

  #[test]
  fn ranges_are_read_back_from_the_bindings() {
    let mut ws = Ws::default();
    ws.sync(&settings(&[Some(0), Some(0), Some(0), Some(1), Some(1), Some(1)], 2));
    assert_eq!(ws.dividers, vec![3]);
    assert_eq!(ws.seg_mon, vec![0, 1]);
    assert!(!ws.legacy());
  }

  #[test]
  fn scattered_bindings_fall_back_to_even_ranges() {
    let mut ws = Ws::default();
    ws.sync(&settings(&[Some(0), Some(1), Some(0), Some(1)], 2));
    assert_eq!(ws.dividers, vec![2]);
    assert_eq!(ws.seg_mon, vec![0, 1]);
  }

  #[test]
  fn old_one_based_numbers_are_legacy() {
    let mut ws = Ws::default();
    ws.sync(&settings(&[Some(1), Some(1), Some(2), Some(2)], 2));
    assert!(ws.legacy());
    assert_eq!(ws.dividers, vec![2]);
    assert_eq!(ws.seg_mon, vec![0, 1]);
  }

  #[test]
  fn the_order_starts_at_the_chosen_first() {
    let mut ws = Ws::default();
    ws.sync(&settings(&[None, None, None, None, None], 1));
    ws.first = 4;
    assert_eq!(ws.ordered(), vec![4, 5, 1, 2, 3]);
  }

  #[test]
  fn boundaries_stay_between_their_neighbours() {
    let mut ws = Ws::default();
    ws.sync(&settings(&[Some(0); 9], 3));
    assert_eq!(ws.dividers, vec![3, 6]);
    ws.drag_divider(0, 0.99);
    assert_eq!(ws.dividers, vec![5, 6]);
    ws.drag_divider(1, 0.0);
    assert_eq!(ws.dividers, vec![5, 6]);
    ws.nudge_divider(1, 5);
    assert_eq!(ws.dividers, vec![5, 8]);
  }

  #[test]
  fn ranges_become_a_map_in_order() {
    let mut ws = Ws::default();
    ws.sync(&settings(&[Some(0), Some(0), Some(1), Some(1)], 2));
    let map = ws.divider_map();
    assert_eq!(map[&1], 0);
    assert_eq!(map[&3], 1);
    let args = ws.divider_args().unwrap();
    assert_eq!(args, vec!["--set-workspaces", "first:1", "1:0", "2:0", "3:1", "4:1"]);
    assert!(ws.divider_args().is_none(), "busy until the core answers");
  }

  #[test]
  fn line_labels_do_not_crowd() {
    let l = labels(30, &[10], 500.0);
    assert_eq!(l.first(), Some(&1));
    assert_eq!(l.last(), Some(&30));
    assert!(l.contains(&10));
  }
}
