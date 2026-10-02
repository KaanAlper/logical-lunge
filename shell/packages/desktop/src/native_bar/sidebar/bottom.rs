//! The panel's bottom group (ii BottomWidgetGroup; sidebar.html
//! BottomWidgetGroup): a rail of three tabs -- calendar (with a month / year
//! picker), to-do list, pomodoro timer -- that folds down to one line with
//! the date and the open tasks.

use std::{collections::HashMap, time::Instant};

use windows::Win32::Graphics::{
  Direct2D::D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
  DirectWrite::{DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_RANGE, DWRITE_WORD_WRAPPING_WRAP},
};

use super::{
  super::{
    anim::{POP_IN, SPRING_IN},
    gfx::{pt, Rect},
    model::{local_day, Model},
    view::Align,
    Ui,
  },
  kit::{st, stw, Cx},
  notifs::now_ms,
  store::{phase_secs, Phase, Pomo, Store, Todo},
  text::{TextField, Typed},
  FieldId, Hit, ScrollId,
};

pub(super) const OPEN_H: f32 = 350.0;
pub(super) const FOLDED_H: f32 = 52.0;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum BHit {
  Fold,
  Unfold,
  Tab(usize),
  /// the calendar's body (the wheel turns the months)
  Calendar,
  Title,
  Prev,
  Next,
  /// the month picker: its body (the wheel turns the years)
  Picker,
  PickPrev,
  PickNext,
  Month(u32),
  Today,
  TodoTab(bool),
  TodoDone(usize),
  TodoDelete(usize),
  TodoAdd,
  PomoStart,
  PomoReset,
}

const TABS: [(&str, &str); 3] = [("Takvim", "calendar_month"), ("Yapılacaklar", "done_outline"), ("Zamanlayıcı", "schedule")];

pub(super) fn days_in_month(y: i32, m: u32) -> u32 {
  match m {
    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
    4 | 6 | 9 | 11 => 30,
    _ => {
      if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
        29
      } else {
        28
      }
    }
  }
}

/// 0 Monday .. 6 Sunday (Sakamoto's method).
pub(super) fn weekday(y: i32, m: u32, d: u32) -> u32 {
  const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
  let y = if m < 3 { y - 1 } else { y };
  let sunday0 = (y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d as i32).rem_euclid(7) as u32;
  (sunday0 + 6) % 7
}

/// (year, month) `offset` months from (y, m).
pub(super) fn add_months(y: i32, m: u32, offset: i32) -> (i32, u32) {
  let total = y * 12 + m as i32 - 1 + offset;
  (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32)
}

/// The 42 cells of a month page: (day, in this month, today).
pub(super) fn cells(y: i32, m: u32, today: (i32, u32, u32)) -> Vec<(u32, bool, bool)> {
  let first = weekday(y, m, 1);
  let days = days_in_month(y, m);
  let (py, pm) = add_months(y, m, -1);
  let prev = days_in_month(py, pm);
  (0..42)
    .map(|i| {
      let d = i as i32 - first as i32 + 1;
      if d < 1 {
        ((prev as i32 + d) as u32, false, false)
      } else if d as u32 > days {
        (d as u32 - days, false, false)
      } else {
        (d as u32, true, (y, m, d as u32) == today)
      }
    })
    .collect()
}

#[derive(Default)]
pub(super) struct Bottom {
  /// months from today's
  pub offset: i32,
  /// the picker: the year it shows, the direction it last moved, when
  picker: Option<(i32, i32, Instant)>,
  pub show_done: bool,
  tab_at: Option<Instant>,
  /// folding / unfolding: the height it started from, when
  fold: Option<(f32, Instant)>,
  pub cal_rect: Rect,
  pub picker_rect: Option<Rect>,
}

impl Bottom {
  pub fn height(&self, collapsed: bool, now: Instant, animations: bool) -> (f32, bool) {
    let target = if collapsed { FOLDED_H } else { OPEN_H };
    match self.fold {
      Some((from, at)) if animations => {
        let k = (now.duration_since(at).as_secs_f32() * 1000.0 / 300.0).min(1.0);
        (from + (target - from) * SPRING_IN.at(k), k < 1.0)
      }
      _ => (target, false),
    }
  }

  pub fn set_folded(&mut self, now: Instant, from_h: f32) {
    self.fold = Some((from_h, now));
  }

  pub fn picker_open(&self) -> bool {
    self.picker.is_some()
  }

  pub fn close_picker(&mut self) {
    self.picker = None;
  }

  /// The wheel over the picker: a year back or forward.
  pub fn picker_step(&mut self, d: i32) {
    if let Some(p) = self.picker.as_mut() {
      *p = (p.0 + d, d, Instant::now());
    }
  }

  #[allow(clippy::too_many_arguments)]
  pub fn paint(&mut self, cx: &mut Cx, m: &Model, r: Rect, store: &Store, todo_field: &mut TextField, scroll: &mut HashMap<ScrollId, f32>, animations: bool) -> anyhow::Result<()> {
    cx.round(r, 17.0, cx.t.layer1)?;
    cx.push_clip(r);
    let result = self.paint_inner(cx, m, r, store, todo_field, scroll, animations);
    cx.pop_clip();
    result
  }

  #[allow(clippy::too_many_arguments)]
  fn paint_inner(&mut self, cx: &mut Cx, m: &Model, r: Rect, store: &Store, todo_field: &mut TextField, scroll: &mut HashMap<ScrollId, f32>, animations: bool) -> anyhow::Result<()> {
    self.picker_rect = None;
    let today = local_day(None);
    if store.collapsed && r.h <= FOLDED_H + 0.5 {
      // `.collapsed-row`: unfold, the date, the open tasks
      let b = Rect::new(r.x + 10.0, r.y + 10.0, 32.0, 32.0);
      cx.round_btn(b, "keyboard_arrow_up", 22.0, false, None, cx.t.on_layer1, Hit::Bottom(BHit::Unfold))?;
      let open = store.todo.iter().filter(|t| !t.done).count();
      let line = format!("{}   •   {}", m.format_day(today.0, today.1, today.2, "dddd, d MMMM"), cx.tr(&format!("{open} görev")));
      cx.text(&line, Rect::new(b.right() + 15.0, r.y, r.w - 32.0 - 35.0, 52.0), st(17.0), cx.t.on_layer1)?;
      return Ok(());
    }
    // the rail
    let fold = Rect::new(r.x + 10.0, r.y + 10.0, 32.0, 32.0);
    cx.round_btn(fold, "keyboard_arrow_down", 22.0, false, None, cx.t.on_layer1, Hit::Bottom(BHit::Fold))?;
    let rail_h = 3.0 * 50.0 + 2.0 * 6.0;
    let mut y = r.y + (OPEN_H - rail_h) / 2.0;
    for (i, (name, icon)) in TABS.iter().enumerate() {
      let on = store.tab == i;
      let hit = Hit::Bottom(BHit::Tab(i));
      let pill = Rect::new(r.x + 15.0, y, 56.0, 32.0);
      if on {
        cx.round(pill, 16.0, cx.t.sec_container)?;
      } else if cx.hot(&hit) {
        cx.round(pill, 16.0, cx.t.layer1_hover)?;
      }
      let fg = if on { cx.t.on_sec_container } else { cx.t.on_surface_variant };
      cx.icon(icon, pill.x + 28.0, pill.y + 16.0, 22.0, on, fg)?;
      cx.text_center(&cx.tr(name), Rect::new(pill.x - 12.0, pill.bottom() + 2.0, 80.0, 16.0), st(12.0), if on { cx.t.on_layer1 } else { cx.t.on_surface_variant })?;
      cx.hit(Rect::new(pill.x - 4.0, pill.y, 64.0, 50.0), hit);
      y += 56.0;
    }
    // the open tab, coming up 10 DIP and fading in
    let body = Rect::new(r.x + 15.0 + 56.0 + 20.0, r.y + 10.0, r.w - 91.0 - 10.0, OPEN_H - 20.0);
    let (alpha, rise) = match self.tab_at {
      Some(at) if animations => {
        let k = (cx.now.duration_since(at).as_secs_f32() * 1000.0 / 200.0).min(1.0);
        if k < 1.0 {
          cx.busy = true;
        }
        let e = POP_IN.at(k);
        (e, 10.0 * (1.0 - e))
      }
      _ => (1.0, 0.0),
    };
    let body = Rect::new(body.x, body.y + rise, body.w, body.h);
    let _ = alpha;
    match store.tab {
      0 => self.paint_calendar(cx, m, body, today)?,
      1 => self.paint_todo(cx, body, &store.todo, todo_field, scroll)?,
      _ => paint_pomo(cx, body, &store.pomo)?,
    }
    Ok(())
  }

  fn paint_calendar(&mut self, cx: &mut Cx, m: &Model, r: Rect, today: (i32, u32, u32)) -> anyhow::Result<()> {
    self.cal_rect = r;
    cx.hit(r, Hit::Bottom(BHit::Calendar));
    let (y, mo) = add_months(today.0, today.1, self.offset);
    // head: the month (opens the picker), previous / next
    let title = m.format_day(y, mo, 1, "MMMM yyyy");
    let tw = cx.measure(&title, st(16.0))?.ceil();
    let tb = Rect::new(r.x, r.y, tw + 24.0, 34.0);
    let th = Hit::Bottom(BHit::Title);
    if self.picker.is_some() {
      cx.round(tb, 17.0, cx.t.sec_container)?;
    } else if cx.hot(&th) {
      cx.round(tb, 17.0, cx.t.layer1_hover)?;
    }
    cx.text(&title, Rect::new(tb.x + 12.0, tb.y, tw + 2.0, 34.0), st(16.0), if self.picker.is_some() { cx.t.on_sec_container } else { cx.t.on_layer1 })?;
    cx.hit(tb, th);
    cx.round_btn(Rect::new(r.right() - 66.0, r.y + 1.0, 32.0, 32.0), "chevron_left", 22.0, false, None, cx.t.on_layer1, Hit::Bottom(BHit::Prev))?;
    cx.round_btn(Rect::new(r.right() - 32.0, r.y + 1.0, 32.0, 32.0), "chevron_right", 22.0, false, None, cx.t.on_layer1, Hit::Bottom(BHit::Next))?;
    // the grid: weekdays from Monday, then six weeks
    let gy = r.y + 34.0 + 6.0;
    let cw = (r.w - 6.0 * 2.0) / 7.0;
    for i in 0..7u32 {
      // 2024-01-01 was a Monday
      let wd: String = m.format_day(2024, 1, 1 + i, "ddd").chars().take(2).collect();
      cx.text_center(&wd, Rect::new(r.x + i as f32 * (cw + 2.0), gy, cw, 29.0), st(13.0), cx.t.on_surface_variant)?;
    }
    let mut cy = gy + 29.0 + 2.0;
    for (i, (d, inside, is_today)) in cells(y, mo, today).into_iter().enumerate() {
      let col = (i % 7) as f32;
      if i > 0 && i % 7 == 0 {
        cy += 36.0 + 2.0;
      }
      let cell = Rect::new(r.x + col * (cw + 2.0), cy, cw, 36.0);
      if is_today {
        let dd = cell.h.min(cell.w);
        cx.round(Rect::new(cell.x + (cell.w - dd) / 2.0, cell.y, dd, dd), dd / 2.0, cx.t.primary)?;
        cx.text_center(&d.to_string(), cell, stw(14.0, 600.0), cx.t.on_primary)?;
      } else {
        let c = if inside { cx.t.on_layer1 } else { cx.c.outline.alpha(0.6) };
        cx.text_center(&d.to_string(), cell, st(14.0), c)?;
      }
    }
    if let Some((year, dir, at)) = self.picker {
      self.paint_picker(cx, m, Rect::new(r.x, r.y + 44.0, r.w, 0.0), year, dir, at, (y, mo), today)?;
    }
    Ok(())
  }

  #[allow(clippy::too_many_arguments)]
  fn paint_picker(&mut self, cx: &mut Cx, m: &Model, at: Rect, year: i32, dir: i32, since: Instant, view: (i32, u32), today: (i32, u32, u32)) -> anyhow::Result<()> {
    let h = 10.0 + 36.0 + 8.0 + 3.0 * 40.0 + 2.0 * 6.0 + 8.0 + 32.0 + 10.0;
    let r = Rect::new(at.x, at.y, at.w, h);
    self.picker_rect = Some(r);
    cx.shadow(r, 17.0, 1.0)?;
    cx.round(r, 17.0, cx.c.layer2)?;
    cx.hit(r, Hit::Bottom(BHit::Picker));
    // a year change slides the year and the months in from its side
    let k = (cx.now.duration_since(since).as_secs_f32() * 1000.0 / 220.0).min(1.0);
    if k < 1.0 {
      cx.busy = true;
    }
    let slide = dir as f32 * 24.0 * (1.0 - POP_IN.at(k));
    let fade = if dir == 0 { 1.0 } else { POP_IN.at(k) };
    cx.round_btn(Rect::new(r.x + 12.0, r.y + 12.0, 32.0, 32.0), "chevron_left", 22.0, false, None, cx.t.on_layer1, Hit::Bottom(BHit::PickPrev))?;
    cx.round_btn(Rect::new(r.right() - 44.0, r.y + 12.0, 32.0, 32.0), "chevron_right", 22.0, false, None, cx.t.on_layer1, Hit::Bottom(BHit::PickNext))?;
    cx.text_center(&year.to_string(), Rect::new(r.x + slide, r.y + 10.0, r.w, 36.0), stw(18.0, 600.0), cx.t.on_layer1.alpha(fade))?;
    let gw = (r.w - 20.0 - 3.0 * 6.0) / 4.0;
    for i in 0..12u32 {
      let (col, row) = ((i % 4) as f32, (i / 4) as f32);
      let b = Rect::new(r.x + 10.0 + col * (gw + 6.0) + slide, r.y + 10.0 + 36.0 + 8.0 + row * 46.0, gw, 40.0);
      let name = m.format_day(2000, i + 1, 1, "MMM").replace('.', "");
      let sel = year == view.0 && i + 1 == view.1;
      let now = year == today.0 && i + 1 == today.1;
      let hit = Hit::Bottom(BHit::Month(i + 1));
      if sel {
        cx.round(b, 12.0, cx.t.primary)?;
      } else if cx.hot(&hit) {
        cx.round(b, 20.0, cx.c.layer2_hover)?;
      }
      if now && !sel {
        cx.p.stroke_round(b.inset(0.75, 0.75), 20.0, cx.t.primary, 1.5)?;
      }
      cx.text_center(&name, b, st(14.0), if sel { cx.t.on_primary.alpha(fade) } else { cx.t.on_layer1.alpha(fade) })?;
      cx.hit(b, hit);
    }
    let label = cx.tr("Bugün");
    let w = cx.chip_w(&label, Some("today"))?;
    cx.chip(r.right() - 10.0 - w, r.bottom() - 10.0 - 32.0, 32.0, &label, Some("today"), false, true, Hit::Bottom(BHit::Today))?;
    Ok(())
  }

  fn paint_todo(&mut self, cx: &mut Cx, r: Rect, list: &[Todo], field: &mut TextField, scroll: &mut HashMap<ScrollId, f32>) -> anyhow::Result<()> {
    let mut x = r.x;
    for done in [false, true] {
      let label = cx.tr(if done { "Tamamlanan" } else { "Yapılacak" });
      x += cx.chip(x, r.y, 32.0, &label, None, self.show_done == done, true, Hit::Bottom(BHit::TodoTab(done)))? + 6.0;
    }
    let list_top = r.y + 32.0 + 8.0;
    let add_h = if self.show_done { 0.0 } else { 8.0 + 36.0 };
    let area = Rect::new(r.x, list_top, r.w, r.bottom() - list_top - add_h);
    let shown: Vec<(usize, &Todo)> = list.iter().enumerate().filter(|(_, t)| t.done == self.show_done).collect();
    if shown.is_empty() {
      let (icon, text) = if self.show_done { ("checklist", "Tamamlanan yok") } else { ("check_circle", "Görev yok") };
      cx.icon(icon, area.x + area.w / 2.0, area.y + 60.0 + 20.0, 40.0, false, cx.c.outline)?;
      cx.text_center(&cx.tr(text), Rect::new(area.x, area.y + 60.0 + 46.0, area.w, 20.0), st(15.0), cx.c.outline)?;
    } else {
      let text_w = area.w - 20.0 - 2.0 * 32.0 - 2.0 * 8.0;
      let mut heights = Vec::new();
      for (_, t) in &shown {
        heights.push(cx.wrapped_h(&t.content, st(14.0), text_w, 1000.0)?.max(20.0) + 16.0);
      }
      let content = heights.iter().sum::<f32>() + 4.0 * (heights.len() as f32 - 1.0);
      let max = (content - area.h).max(0.0);
      let off = scroll.get(&ScrollId::Todo).copied().unwrap_or(0.0).clamp(0.0, max);
      scroll.insert(ScrollId::Todo, off);
      cx.region(area, ScrollId::Todo, content, false);
      cx.push_clip(area);
      let mut y = area.y - off;
      for ((i, t), h) in shown.iter().zip(heights) {
        let row = Rect::new(area.x, y, area.w, h);
        cx.round(row, 12.0, cx.c.layer2)?;
        strike_text(cx, &t.content, Rect::new(row.x + 10.0, row.y + 8.0, text_w, h - 16.0), t.done)?;
        let b1 = Rect::new(row.right() - 10.0 - 32.0 - 8.0 - 32.0, row.y + (h - 32.0) / 2.0, 32.0, 32.0);
        cx.round_btn(b1, if t.done { "remove_done" } else { "check" }, 18.0, false, None, cx.t.on_layer1, Hit::Bottom(BHit::TodoDone(*i)))?;
        let b2 = Rect::new(row.right() - 10.0 - 32.0, b1.y, 32.0, 32.0);
        cx.round_btn(b2, "delete_forever", 18.0, false, None, cx.t.on_layer1, Hit::Bottom(BHit::TodoDelete(*i)))?;
        y += h + 4.0;
      }
      cx.pop_clip();
    }
    if !self.show_done {
      let fy = r.bottom() - 36.0;
      let fr = Rect::new(r.x, fy, r.w - 36.0 - 6.0, 36.0);
      let ph = cx.tr("Görev ekle");
      cx.field_box(fr, 18.0, field, FieldId::Todo, &ph, st(14.0), 14.0, None)?;
      let fab = Rect::new(fr.right() + 6.0, fy, 36.0, 36.0);
      let hit = Hit::Bottom(BHit::TodoAdd);
      cx.round(fab, 12.0, if cx.hot(&hit) { super::kit::blend(cx.t.primary_container, cx.t.on_primary_container, 0.08) } else { cx.t.primary_container })?;
      cx.icon("add", fab.x + 18.0, fab.y + 18.0, 22.0, false, cx.t.on_primary_container)?;
      cx.hit(fab, hit);
    }
    Ok(())
  }

  pub fn tab_changed(&mut self) {
    self.tab_at = Some(Instant::now());
    self.picker = None;
  }
}

/// Wrapped text, struck through when done (`.todo-item.done .t`).
fn strike_text(cx: &mut Cx, s: &str, r: Rect, done: bool) -> anyhow::Result<()> {
  let color = if done { cx.c.outline } else { cx.t.on_layer1 };
  let layout = cx.p.layout(s, st(14.0), r.w, 10_000.0, false)?;
  unsafe {
    layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
    layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
    layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, 14.0 * 1.35, 14.0 * 1.35 * 0.8)?;
    if done {
      layout.SetStrikethrough(true, DWRITE_TEXT_RANGE { startPosition: 0, length: u32::MAX })?;
    }
    let b = cx.p.brush(color)?;
    cx.p.dc.DrawTextLayout(pt(r.x, r.y), &layout, &b, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
  }
  Ok(())
}

/// pomodoro/PomodoroWidget.qml: the ring of the time left, the time, the
/// phase and round; start / pause and reset.
fn paint_pomo(cx: &mut Cx, r: Rect, pomo: &Pomo) -> anyhow::Result<()> {
  let p = pomo.advance(now_ms());
  let total = phase_secs(p.phase) as f32;
  let (cxx, cyy) = (r.x + r.w / 2.0, r.y + (r.h - 14.0 - 40.0) / 2.0);
  cx.p.ring(cxx, cyy, 80.0, 8.0, p.left as f32 / total, cx.t.sec_container, cx.t.primary)?;
  let time = format!("{:02}:{:02}", p.left / 60, p.left % 60);
  cx.p.text(&time, Rect::new(cxx - 90.0, cyy - 30.0, 180.0, 44.0), stw(36.0, 500.0), cx.t.on_layer1, Align::Center, true)?;
  let phase = match p.phase {
    Phase::Focus => "Odak",
    Phase::Long => "Uzun mola",
    Phase::Break => "Mola",
  };
  let label = format!("{} · {}/4", cx.tr(phase), p.cycle % 4);
  cx.text_center(&label, Rect::new(cxx - 90.0, cyy + 14.0, 180.0, 18.0), st(13.0), cx.t.on_surface_variant)?;
  // buttons: 40 high, 18 padding, 14 text
  let start = cx.tr(if p.running { "Duraklat" } else { "Başlat" });
  let reset = cx.tr("Sıfırla");
  let sw = cx.measure(&start, st(14.0))?.ceil() + 36.0;
  let rw = cx.measure(&reset, st(14.0))?.ceil() + 36.0;
  let by = cyy + 90.0 + 14.0;
  let bx = cxx - (sw + 8.0 + rw) / 2.0;
  let hs = Hit::Bottom(BHit::PomoStart);
  let sb = Rect::new(bx, by, sw, 40.0);
  cx.round(sb, 20.0, if cx.hot(&hs) { super::kit::blend(cx.t.primary, cx.t.on_primary, 0.08) } else { cx.t.primary })?;
  cx.text_center(&start, sb, st(14.0), cx.t.on_primary)?;
  cx.hit(sb, hs);
  let hr = Hit::Bottom(BHit::PomoReset);
  let rb = Rect::new(sb.right() + 8.0, by, rw, 40.0);
  cx.round(rb, 20.0, if cx.hot(&hr) { cx.c.layer2_hover } else { cx.c.layer2 })?;
  cx.text_center(&reset, rb, st(14.0), cx.t.on_layer1)?;
  cx.hit(rb, hr);
  Ok(())
}

impl Ui {
  pub(super) fn sb_bottom_click(&mut self, h: BHit) {
    let now = Instant::now();
    let animations = self.model.animations;
    let sb = &mut self.sidebar;
    match h {
      BHit::Fold | BHit::Unfold => {
        let from = sb.bottom.height(sb.store.collapsed, now, animations).0;
        sb.store.collapsed = matches!(h, BHit::Fold);
        sb.bottom.set_folded(now, from);
        sb.bottom.close_picker();
        sb.save_soon();
        self.sb_frames();
      }
      BHit::Tab(i) => {
        if sb.store.tab != i {
          sb.store.tab = i;
          sb.bottom.tab_changed();
          sb.save_soon();
          self.sb_frames();
        }
      }
      BHit::Title => {
        let (ty, tm, _) = local_day(None);
        let b = &mut sb.bottom;
        b.picker = if b.picker.is_some() { None } else { Some((add_months(ty, tm, b.offset).0, 0, now)) };
      }
      BHit::Prev => sb.bottom.offset -= 1,
      BHit::Next => sb.bottom.offset += 1,
      BHit::PickPrev => {
        sb.bottom.picker_step(-1);
        self.sb_frames();
      }
      BHit::PickNext => {
        sb.bottom.picker_step(1);
        self.sb_frames();
      }
      BHit::Month(mo) => {
        let (ty, tm, _) = local_day(None);
        if let Some((year, _, _)) = sb.bottom.picker {
          sb.bottom.offset = (year - ty) * 12 + mo as i32 - tm as i32;
        }
        sb.bottom.picker = None;
      }
      BHit::Today => {
        sb.bottom.offset = 0;
        sb.bottom.picker = None;
      }
      BHit::TodoTab(done) => sb.bottom.show_done = done,
      BHit::TodoDone(i) => {
        if let Some(t) = sb.store.todo.get_mut(i) {
          t.done = !t.done;
          sb.save_soon();
        }
      }
      BHit::TodoDelete(i) => {
        if i < sb.store.todo.len() {
          sb.store.todo.remove(i);
          sb.save_soon();
        }
      }
      BHit::TodoAdd => self.sb_todo_add(),
      BHit::PomoStart => {
        let p = sb.store.pomo.advance(now_ms()).toggle(now_ms());
        sb.store.pomo = p;
        sb.save_soon();
      }
      BHit::PomoReset => {
        sb.store.pomo = Pomo::default();
        sb.save_soon();
      }
      BHit::Calendar | BHit::Picker => {}
    }
    self.sb_render();
  }

  pub(super) fn sb_todo_add(&mut self) {
    let sb = &mut self.sidebar;
    let text = sb.field(FieldId::Todo).text();
    let text = text.trim();
    if text.is_empty() {
      return;
    }
    sb.store.todo.push(Todo { content: text.to_string(), done: false });
    sb.field(FieldId::Todo).set("");
    sb.save_soon();
    self.sb_render();
  }

  pub(super) fn sb_todo_key(&mut self, t: Typed) {
    if t == Typed::Submit {
      self.sb_todo_add();
    }
  }

  /// The wheel over the calendar turns the months; over the picker, the years.
  pub(super) fn sb_bottom_wheel(&mut self, h: &BHit, up: bool) -> bool {
    let d = if up { -1 } else { 1 };
    match h {
      BHit::Picker | BHit::PickPrev | BHit::PickNext | BHit::Month(_) | BHit::Today => {
        self.sidebar.bottom.picker_step(d);
        self.sb_frames();
      }
      BHit::Calendar | BHit::Title | BHit::Prev | BHit::Next if !self.sidebar.bottom.picker_open() => self.sidebar.bottom.offset += d,
      _ => return false,
    }
    self.sb_render();
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn weekdays_start_on_monday() {
    assert_eq!(weekday(2024, 1, 1), 0, "Monday");
    assert_eq!(weekday(2026, 10, 2), 4, "Friday");
    assert_eq!(weekday(2000, 2, 29), 1, "Tuesday");
  }

  #[test]
  fn months_add_across_years() {
    assert_eq!(add_months(2026, 12, 1), (2027, 1));
    assert_eq!(add_months(2026, 1, -1), (2025, 12));
    assert_eq!(add_months(2026, 10, -22), (2024, 12));
  }

  #[test]
  fn a_page_has_six_weeks_with_neighbours_greyed() {
    let c = cells(2026, 10, (2026, 10, 2));
    assert_eq!(c.len(), 42);
    // 1 October 2026 is a Thursday: three days of September first
    assert_eq!(c[0], (28, false, false));
    assert_eq!(c[3], (1, true, false));
    assert_eq!(c[4], (2, true, true));
    assert_eq!(c[3 + 31], (1, false, false));
  }

  #[test]
  fn leap_years() {
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(1900, 2), 28);
    assert_eq!(days_in_month(2000, 2), 29);
  }
}
