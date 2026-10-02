//! The settings window's controls (settings.css): cards of rows, switches,
//! segmented choices, drop-down boxes, sliders, buttons, text fields. Each
//! control drawn leaves a [`Region`]: what the mouse and the keyboard find.

use windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE;

use super::{is_light, Fld, Hit, Key};
use crate::native_bar::{
  fonts::TextStyle,
  gfx::{Rect, Rgba},
  view::{Align, Painter, Theme},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cursor {
  Arrow,
  Hand,
  Text,
  Resize,
}

pub struct Region {
  /// window DIP
  pub r: Rect,
  pub hit: Hit,
  /// Tab stops here
  pub focus: bool,
  pub cursor: Cursor,
  /// inside the scrolled view (a control scrolled away is not clicked)
  pub visible: bool,
}

pub const ROW_MIN: f32 = 64.0;
pub const ROW_PAD_X: f32 = 18.0;
pub const ROW_PAD_Y: f32 = 12.0;
pub const GAP: f32 = 14.0;
pub const CTRL_GAP: f32 = 10.0;
pub const CARD_R: f32 = 20.0;
const LABEL: TextStyle = TextStyle { size: 15.0, weight: 450.0 };
pub const SUB: TextStyle = TextStyle { size: 12.5, weight: 450.0 };
pub const BODY: TextStyle = TextStyle { size: 14.0, weight: 450.0 };
const BTN_TEXT: TextStyle = TextStyle { size: 14.0, weight: 560.0 };

/// settings.css colours the bar's theme does not carry.
pub fn outline(t: &Theme) -> Rgba {
  if is_light(t) { Rgba::hex(0x79747e) } else { Rgba::hex(0x938f99) }
}
pub fn layer2(t: &Theme) -> Rgba {
  t.surface_container_high
}
pub fn layer3(t: &Theme) -> Rgba {
  if is_light(t) { Rgba::hex(0xe6e0e9) } else { Rgba::hex(0x36343b) }
}
pub fn ok_color(t: &Theme) -> Rgba {
  if is_light(t) { Rgba::hex(0x2e7d4f) } else { Rgba::hex(0xa8dab5) }
}
pub fn card_border(t: &Theme) -> Rgba {
  if is_light(t) { Rgba::hex(0xe6e0e9) } else { Rgba::hex(0x2c2a31) }
}
/// a light overlay on the theme's text colour (`rgba(255 255 255 / a)` on dark)
pub fn ink(t: &Theme, a: f32) -> Rgba {
  t.on_layer1.alpha(a)
}

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
  Primary,
  Tonal,
  Outline,
}

pub enum Ctrl {
  Switch(bool, Hit),
  /// key, (label, icon), selected
  Seg(Key, Vec<(String, Option<&'static str>)>, usize),
  /// shown value, the list, minimum width
  Select(String, Hit, f32),
  Slider { value: i32, min: i32, max: i32, label: String, hit: Hit },
  Button { label: String, icon: Option<&'static str>, kind: Btn, hit: Hit, disabled: bool },
  /// chevron / open-in-new of a clickable row
  Icon(&'static str),
  /// − [n] +
  Stepper { text: String, fld: Fld, can_dec: bool, can_inc: bool },
  Field { text: String, placeholder: String, w: f32, fld: Fld },
  Label(String, Rgba),
  /// a dot and a word (green, or red when not)
  State(bool, String),
}

pub struct Row {
  pub icon: Option<&'static str>,
  pub label: String,
  pub sub: Option<String>,
  pub ctrls: Vec<Ctrl>,
  /// the whole row is a button
  pub click: Option<Hit>,
}

impl Row {
  pub fn new(icon: &'static str, label: String) -> Self {
    Row { icon: Some(icon), label, sub: None, ctrls: Vec::new(), click: None }
  }
  pub fn sub(mut self, s: String) -> Self {
    self.sub = Some(s);
    self
  }
  pub fn ctrl(mut self, c: Ctrl) -> Self {
    self.ctrls.push(c);
    self
  }
  pub fn click(mut self, h: Hit) -> Self {
    self.click = Some(h);
    self
  }
}

pub struct Ctx<'p, 'a> {
  pub p: &'p mut Painter<'a>,
  pub t: &'p Theme,
  pub tr: &'p dyn Fn(&str) -> String,
  pub regions: &'p mut Vec<Region>,
  pub hover: Option<Hit>,
  pub focus: Option<Hit>,
  pub ring: bool,
  /// the scrolled view: regions outside it are not hit
  pub clip: Rect,
  /// a text field is being typed in (its caret shows)
  pub typing: Option<Fld>,
}

impl Ctx<'_, '_> {
  pub fn tr(&self, s: &str) -> String {
    (self.tr)(s)
  }

  pub fn push(&mut self, r: Rect, hit: Hit, focus: bool, cursor: Cursor) {
    let visible = r.bottom() > self.clip.y && r.y < self.clip.bottom() && r.right() > self.clip.x && r.x < self.clip.right();
    self.regions.push(Region { r, hit, focus, cursor, visible });
  }

  pub fn hot(&self, hit: Hit) -> bool {
    self.hover == Some(hit)
  }

  /// The keyboard focus ring (only once the keyboard is used).
  pub fn ring(&mut self, r: Rect, radius: f32, hit: Hit) -> anyhow::Result<()> {
    if self.ring && self.focus == Some(hit) {
      let t = self.t.primary;
      self.p.stroke_round(Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0), radius + 3.0, t, 2.0)?;
    }
    Ok(())
  }

  pub fn sec_title(&mut self, x: f32, y: f32, text: &str) -> anyhow::Result<f32> {
    let s = self.tr(text);
    let primary = self.t.primary;
    self.p.text(&s, Rect::new(x + 4.0, y + 18.0, 560.0, 18.0), TextStyle { size: 13.0, weight: 600.0 }, primary, Align::Left, false)?;
    Ok(y + 18.0 + 18.0 + 8.0)
  }

  /// A rectangle rounded only at the top and / or bottom (a card's first
  /// and last row).
  pub fn fill_part(&mut self, r: Rect, radius: f32, top: bool, bottom: bool, c: Rgba) -> anyhow::Result<()> {
    if !top && !bottom {
      return Ok(self.p.fill(r, c)?);
    }
    self.p.fill_round(r, radius, c)?;
    if !top {
      self.p.fill(Rect::new(r.x, r.y, r.w, radius), c)?;
    }
    if !bottom {
      self.p.fill(Rect::new(r.x, r.bottom() - radius, r.w, radius), c)?;
    }
    Ok(())
  }

  /// A rectangle rounded only at the left and / or right end (a part of
  /// the number line).
  pub fn fill_part_h(&mut self, r: Rect, radius: f32, left: bool, right: bool, c: Rgba) -> anyhow::Result<()> {
    if !left && !right {
      return Ok(self.p.fill(r, c)?);
    }
    self.p.fill_round(r, radius, c)?;
    if !left {
      self.p.fill(Rect::new(r.x, r.y, radius.min(r.w), r.h), c)?;
    }
    if !right {
      self.p.fill(Rect::new(r.right() - radius.min(r.w), r.y, radius.min(r.w), r.h), c)?;
    }
    Ok(())
  }

  pub fn ctrl_width(&mut self, c: &Ctrl) -> anyhow::Result<f32> {
    Ok(match c {
      Ctrl::Switch(..) => 52.0,
      Ctrl::Seg(_, items, _) => {
        let mut w = 0.0;
        for (label, icon) in items {
          w += 28.0 + self.p.measure(label, BODY)? + if icon.is_some() { 24.0 } else { 0.0 };
        }
        w
      }
      Ctrl::Select(text, _, min) => (self.p.measure(text, BODY)? + 52.0).max(*min),
      Ctrl::Slider { .. } => 300.0,
      Ctrl::Button { label, icon, .. } => self.p.measure(label, BTN_TEXT)? + 32.0 + if icon.is_some() { 26.0 } else { 0.0 },
      Ctrl::Icon(_) => 24.0,
      Ctrl::Stepper { .. } => 116.0,
      Ctrl::Field { w, .. } => *w,
      Ctrl::Label(s, _) => self.p.measure(s, BODY)? + 2.0,
      Ctrl::State(_, s) => 16.0 + self.p.measure(s, SUB)?,
    })
  }

  fn ctrls_width(&mut self, ctrls: &[Ctrl]) -> anyhow::Result<f32> {
    let mut w = 0.0;
    for c in ctrls {
      w += self.ctrl_width(c)?;
    }
    Ok(w + CTRL_GAP * ctrls.len().saturating_sub(1) as f32)
  }

  fn text_width(&mut self, w: f32, row: &Row) -> anyhow::Result<f32> {
    let cw = self.ctrls_width(&row.ctrls)?;
    let icon = if row.icon.is_some() { 22.0 + GAP } else { 0.0 };
    let ctrls = if row.ctrls.is_empty() { 0.0 } else { cw + GAP };
    Ok((w - 2.0 * ROW_PAD_X - icon - ctrls).max(120.0))
  }

  pub fn row_height(&mut self, w: f32, row: &Row) -> anyhow::Result<f32> {
    let tw = self.text_width(w, row)?;
    let mut h = 20.0;
    if let Some(sub) = &row.sub {
      let sub = self.tr(sub);
      h += 2.0 + self.p.measure_wrapped(&sub, SUB, tw, 400.0, false)?;
    }
    Ok((h + 2.0 * ROW_PAD_Y).max(ROW_MIN))
  }

  pub fn rows_height(&mut self, w: f32, rows: &[Row]) -> anyhow::Result<f32> {
    let mut h = 0.0;
    for r in rows {
      h += self.row_height(w, r)?;
    }
    Ok(h + rows.len().saturating_sub(1) as f32)
  }

  /// A card of rows at (x, y), `extra` DIP taller (custom content below the
  /// rows); returns its height.
  pub fn card(&mut self, x: f32, y: f32, w: f32, rows: &[Row], extra: f32) -> anyhow::Result<f32> {
    let h = self.rows_height(w, rows)? + extra;
    let layer1 = self.t.layer1;
    self.p.fill_round(Rect::new(x, y, w, h), CARD_R, layer1)?;
    let mut ry = y;
    for (i, row) in rows.iter().enumerate() {
      let rh = self.row_height(w, row)?;
      if i > 0 {
        let sep = self.t.layer0;
        self.p.fill(Rect::new(x, ry, w, 1.0), sep)?;
        ry += 1.0;
      }
      let first = i == 0;
      let last = i == rows.len() - 1 && extra == 0.0;
      self.row(Rect::new(x, ry, w, rh), row, first, last)?;
      ry += rh;
    }
    Ok(h)
  }

  fn row(&mut self, r: Rect, row: &Row, first: bool, last: bool) -> anyhow::Result<()> {
    if let Some(hit) = row.click {
      if self.hot(hit) {
        let c = self.t.layer1_hover;
        self.fill_part(r, CARD_R, first, last, c)?;
      }
      self.push(r, hit, true, Cursor::Hand);
      self.ring(r.inset(3.0, 3.0), 16.0, hit)?;
    }
    let mut x = r.x + ROW_PAD_X;
    if let Some(icon) = row.icon {
      let c = self.t.on_surface_variant;
      self.p.icon(icon, x + 11.0, r.y + r.h / 2.0, 22.0, false, c)?;
      x += 22.0 + GAP;
    }
    let tw = self.text_width(r.w, row)?;
    let label = self.tr(&row.label);
    let on = self.t.on_layer1;
    match &row.sub {
      Some(sub) => {
        let sub = self.tr(sub);
        let sh = self.p.measure_wrapped(&sub, SUB, tw, 400.0, false)?;
        let top = r.y + (r.h - (20.0 + 2.0 + sh)) / 2.0;
        self.p.text(&label, Rect::new(x, top, tw, 20.0), LABEL, on, Align::Left, false)?;
        let sv = self.t.on_surface_variant;
        self.p.text_wrapped(&sub, Rect::new(x, top + 22.0, tw, sh + 2.0), SUB, sv, false)?;
      }
      None => {
        self.p.text(&label, Rect::new(x, r.y + (r.h - 20.0) / 2.0, tw, 20.0), LABEL, on, Align::Left, false)?;
      }
    }
    // the controls, right-aligned
    let cw = self.ctrls_width(&row.ctrls)?;
    let mut cx = r.right() - ROW_PAD_X - cw;
    let cy = r.y + r.h / 2.0;
    for c in &row.ctrls {
      let w = self.ctrl_width(c)?;
      self.draw_ctrl(c, cx, cy, w)?;
      cx += w + CTRL_GAP;
    }
    Ok(())
  }

  /// A control `w` wide whose left edge is `x`, centred on `cy`.
  pub fn draw_ctrl(&mut self, c: &Ctrl, x: f32, cy: f32, w: f32) -> anyhow::Result<()> {
    match c {
      Ctrl::Switch(on, hit) => self.switch(x, cy, *on, *hit),
      Ctrl::Seg(key, items, sel) => self.seg(x, cy, *key, items, *sel),
      Ctrl::Select(text, hit, _) => self.select(Rect::new(x, cy - 18.0, w, 36.0), text, *hit),
      Ctrl::Slider { value, min, max, label, hit } => self.slider(x, cy, w, *value, *min, *max, label, *hit),
      Ctrl::Button { label, icon, kind, hit, disabled } => self.button(Rect::new(x, cy - 18.0, w, 36.0), label, *icon, *kind, *hit, *disabled),
      Ctrl::Icon(name) => {
        let c = self.t.on_surface_variant;
        Ok(self.p.icon(name, x + 12.0, cy, 22.0, false, c)?)
      }
      Ctrl::Stepper { text, fld, can_dec, can_inc } => self.stepper(x, cy, text, *fld, *can_dec, *can_inc),
      Ctrl::Field { text, placeholder, w, fld } => self.field(Rect::new(x, cy - 18.0, *w, 36.0), text, placeholder, *fld),
      Ctrl::Label(s, c) => {
        self.p.text(s, Rect::new(x, cy - 10.0, w, 20.0), BODY, *c, Align::Left, true)?;
        Ok(())
      }
      Ctrl::State(good, s) => self.state(x, cy, *good, s),
    }
  }

  /// settings.css `.switch`
  pub fn switch(&mut self, x: f32, cy: f32, on: bool, hit: Hit) -> anyhow::Result<()> {
    let r = Rect::new(x, cy - 16.0, 52.0, 32.0);
    let t = self.t;
    if on {
      self.p.fill_round(r, 16.0, t.primary)?;
      self.p.fill_circle(r.x + 24.0 + 12.0, cy, 12.0, t.on_primary)?;
    } else {
      self.p.fill_round(r, 16.0, layer3(t))?;
      self.p.stroke_round(r, 16.0, outline(t), 2.0)?;
      let d = if self.hot(hit) { 9.0 } else { 8.0 };
      self.p.fill_circle(r.x + 8.0 + 8.0, cy, d, outline(t))?;
    }
    self.push(r, hit, true, Cursor::Hand);
    self.ring(r, 16.0, hit)
  }

  /// settings.css `.seg`
  pub fn seg(&mut self, x: f32, cy: f32, key: Key, items: &[(String, Option<&'static str>)], sel: usize) -> anyhow::Result<()> {
    let t = self.t;
    let mut widths = Vec::new();
    for (label, icon) in items {
      widths.push(28.0 + self.p.measure(label, BODY)? + if icon.is_some() { 24.0 } else { 0.0 });
    }
    let total: f32 = widths.iter().sum();
    let r = Rect::new(x, cy - 18.0, total, 36.0);
    let mut bx = x;
    for (i, ((label, icon), w)) in items.iter().zip(&widths).enumerate() {
      let b = Rect::new(bx, r.y, *w, 36.0);
      let hit = Hit::Seg(key, i);
      let (bg, fg) = if i == sel {
        (Some(t.sec_container), t.on_sec_container)
      } else if self.hot(hit) {
        (Some(t.layer1_hover), t.on_layer1)
      } else {
        (None, t.on_layer1)
      };
      if let Some(bg) = bg {
        // the ends of the pill are round, the inner edges straight
        let first = i == 0;
        let last = i == items.len() - 1;
        if first && last {
          self.p.fill_round(b, 18.0, bg)?;
        } else if first {
          self.p.fill_round(b, 18.0, bg)?;
          self.p.fill(Rect::new(b.right() - 18.0, b.y, 18.0, b.h), bg)?;
        } else if last {
          self.p.fill_round(b, 18.0, bg)?;
          self.p.fill(Rect::new(b.x, b.y, 18.0, b.h), bg)?;
        } else {
          self.p.fill(b, bg)?;
        }
      }
      let mut tx = bx + 14.0;
      if let Some(icon) = icon {
        self.p.icon(icon, tx + 9.0, cy, 18.0, i == sel, fg)?;
        tx += 24.0;
      }
      self.p.text(label, Rect::new(tx, r.y, w - (tx - bx) - 10.0, 36.0), BODY, fg, Align::Left, false)?;
      if i > 0 {
        self.p.fill(Rect::new(bx, r.y, 1.0, 36.0), outline(t))?;
      }
      self.push(b, hit, i == sel, Cursor::Hand);
      bx += w;
    }
    self.p.stroke_round(r, 18.0, outline(t), 1.0)?;
    // one Tab stop: the selected part
    if let Some(i) = (0..items.len()).find(|i| self.focus == Some(Hit::Seg(key, *i))) {
      let b = Rect::new(x + widths[..i].iter().sum::<f32>(), r.y, widths[i], 36.0);
      self.ring(b, 18.0, Hit::Seg(key, i))?;
    }
    Ok(())
  }

  /// settings.css `select`
  pub fn select(&mut self, r: Rect, text: &str, hit: Hit) -> anyhow::Result<()> {
    let t = self.t;
    let hot = self.hot(hit);
    self.p.fill_round(r, 12.0, if hot { t.layer1_hover } else { t.layer1 })?;
    let border = if self.focus == Some(hit) { t.primary } else { outline(t) };
    self.p.stroke_round(r, 12.0, border, 1.0)?;
    self.p.text(text, Rect::new(r.x + 12.0, r.y, r.w - 44.0, r.h), BODY, t.on_layer1, Align::Left, false)?;
    self.p.icon("expand_more", r.right() - 20.0, r.y + r.h / 2.0, 22.0, false, t.on_surface_variant)?;
    self.push(r, hit, true, Cursor::Hand);
    self.ring(r, 12.0, hit)
  }

  /// settings.css `.slider`: the track, the thumb and the value.
  #[allow(clippy::too_many_arguments)]
  pub fn slider(&mut self, x: f32, cy: f32, w: f32, value: i32, min: i32, max: i32, label: &str, hit: Hit) -> anyhow::Result<()> {
    let t = self.t;
    let val_w = 92.0;
    let track = Rect::new(x + 8.0, cy - 2.0, w - val_w - 12.0 - 16.0, 4.0);
    let k = if max > min { (value - min) as f32 / (max - min) as f32 } else { 0.0 };
    let tx = track.x + track.w * k.clamp(0.0, 1.0);
    self.p.fill_round(track, 2.0, layer3(t))?;
    self.p.fill_round(Rect::new(track.x, track.y, tx - track.x, 4.0), 2.0, t.primary)?;
    let hot = self.hot(hit);
    self.p.fill_circle(tx, cy, if hot { 9.0 } else { 8.0 }, t.primary)?;
    let sv = t.on_surface_variant;
    self.p.text(label, Rect::new(x + w - val_w, cy - 10.0, val_w, 20.0), BODY, sv, Align::Left, true)?;
    // the hit area spans the track's height generously
    let area = Rect::new(track.x - 8.0, cy - 14.0, track.w + 16.0, 28.0);
    self.push(Rect::new(track.x, cy - 14.0, track.w, 28.0), hit, true, Cursor::Hand);
    self.ring(area, 14.0, hit)
  }

  /// settings.css `.btn`
  #[allow(clippy::too_many_arguments)]
  pub fn button(&mut self, r: Rect, label: &str, icon: Option<&'static str>, kind: Btn, hit: Hit, disabled: bool) -> anyhow::Result<()> {
    let t = self.t;
    let (bg, fg) = match kind {
      Btn::Primary => (Some(t.primary), t.on_primary),
      Btn::Tonal => (Some(t.sec_container), t.on_sec_container),
      Btn::Outline => (None, t.primary),
    };
    let alpha = if disabled { 0.45 } else { 1.0 };
    let radius = 18.0;
    if let Some(bg) = bg {
      self.p.fill_round(r, radius, bg.alpha(bg.3 * alpha))?;
    } else {
      self.p.stroke_round(r, radius, outline(t), 1.0)?;
    }
    if !disabled && self.hot(hit) {
      self.p.fill_round(r, radius, Rgba(255, 255, 255, 0.08))?;
    }
    let fg = fg.alpha(fg.3 * alpha);
    let tw = self.p.measure(label, BTN_TEXT)?;
    let iw = if icon.is_some() { 26.0 } else { 0.0 };
    let mut tx = r.x + (r.w - tw - iw) / 2.0;
    if let Some(icon) = icon {
      self.p.icon(icon, tx + 9.0, r.y + r.h / 2.0, 18.0, false, fg)?;
      tx += iw;
    }
    self.p.text(label, Rect::new(tx, r.y, tw + 2.0, r.h), BTN_TEXT, fg, Align::Left, false)?;
    if !disabled {
      self.push(r, hit, true, Cursor::Hand);
      self.ring(r, radius, hit)?;
    }
    Ok(())
  }

  /// settings.css `.ws-count`: − [n] +
  pub fn stepper(&mut self, x: f32, cy: f32, text: &str, fld: Fld, can_dec: bool, can_inc: bool) -> anyhow::Result<()> {
    let t = self.t;
    let r = Rect::new(x, cy - 18.0, 116.0, 36.0);
    self.p.fill_round(r, 12.0, layer2(t))?;
    let mid = Rect::new(x + 34.0, r.y, 48.0, 36.0);
    self.p.fill(mid, t.layer1)?;
    let border = if self.focus == Some(Hit::Field(fld)) { t.primary } else { t.outline_variant };
    self.p.stroke_round(r, 12.0, border, 1.0)?;
    for (bx, d, ok, sign) in [(x, -1, can_dec, "−"), (x + 82.0, 1, can_inc, "+")] {
      let b = Rect::new(bx, r.y, 34.0, 36.0);
      let hit = Hit::Step(fld, d);
      let c = t.on_layer1.alpha(if ok { 1.0 } else { 0.35 });
      if ok && self.hot(hit) {
        self.p.fill_round(b.inset(2.0, 2.0), 10.0, t.layer1_hover)?;
      }
      self.p.text(sign, b, BODY, c, Align::Center, false)?;
      if ok {
        self.push(b, hit, false, Cursor::Hand);
      }
    }
    let typing = self.typing == Some(fld);
    let tw = self.p.text(text, mid, BODY, t.on_layer1, Align::Center, true)?;
    if typing {
      let cx = mid.x + (mid.w + tw) / 2.0 + 1.0;
      self.p.fill(Rect::new(cx, cy - 9.0, 1.5, 18.0), t.primary)?;
    }
    self.push(mid, Hit::Field(fld), true, Cursor::Text);
    self.ring(r, 12.0, Hit::Field(fld))
  }

  /// settings.css `.hexrow input` / `input[type=time]`
  pub fn field(&mut self, r: Rect, text: &str, placeholder: &str, fld: Fld) -> anyhow::Result<()> {
    let t = self.t;
    let focused = self.focus == Some(Hit::Field(fld));
    if focused {
      self.p.stroke_round(r.inset(1.0, 1.0), 12.0, t.primary, 2.0)?;
    } else {
      self.p.stroke_round(r, 12.0, outline(t), 1.0)?;
    }
    let inner = Rect::new(r.x + 12.0, r.y, r.w - 24.0, r.h);
    let tw = if text.is_empty() {
      self.p.text(placeholder, inner, BODY, t.on_surface_variant, Align::Left, false)?;
      0.0
    } else {
      self.p.text(text, inner, BODY, t.on_layer1, Align::Left, true)?
    };
    if self.typing == Some(fld) {
      self.p.fill(Rect::new(inner.x + tw + 1.0, r.y + 9.0, 1.5, 18.0), t.primary)?;
    }
    self.push(r, Hit::Field(fld), true, Cursor::Text);
    Ok(())
  }

  pub fn state(&mut self, x: f32, cy: f32, good: bool, s: &str) -> anyhow::Result<()> {
    let c = if good { ok_color(self.t) } else { self.t.error };
    self.p.fill_circle(x + 4.0, cy, 4.0, c)?;
    let fg = if good { self.t.on_layer1 } else { self.t.error };
    self.p.text(s, Rect::new(x + 14.0, cy - 10.0, 200.0, 20.0), SUB, fg, Align::Left, false)?;
    Ok(())
  }

  /// A clip for the scrolled view.
  pub fn push_clip(&mut self, r: Rect) {
    unsafe { self.p.dc.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
  }

  pub fn pop_clip(&mut self) {
    unsafe { self.p.dc.PopAxisAlignedClip() };
  }
}
