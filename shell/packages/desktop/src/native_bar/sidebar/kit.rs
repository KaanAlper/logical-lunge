//! The right panel's drawing kit: one paint records what can be clicked
//! (`Hit`) and what scrolls (`Region`) while it draws, with the sizes and
//! colours of the web panel's sidebar.css (chips, round buttons, switches,
//! sliders, messages, section titles, text fields, spinners).

use std::time::Instant;

use windows::{
  Foundation::Numerics::Matrix3x2,
  Win32::Graphics::Direct2D::{
    Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_GRADIENT_STOP},
    ID2D1Factory, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BUFFER_PRECISION_8BPC_UNORM,
    D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT, D2D1_COLOR_SPACE_SRGB, D2D1_EXTEND_MODE_CLAMP,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
  },
};

use super::{
  super::{
    fonts::TextStyle,
    gfx::{pt, Rect, Rgba},
    view::{Align, Painter, Theme},
  },
  text::TextField,
  FieldId, Hit, ScrollId,
};

/// A scrolling area of the last paint.
#[derive(Clone, Copy, Debug)]
pub(super) struct Region {
  pub rect: Rect,
  pub id: ScrollId,
  /// how far it can scroll (DIPs)
  pub max: f32,
  pub horizontal: bool,
}

pub(super) const fn st(size: f32) -> TextStyle {
  TextStyle { size, weight: 450.0 }
}

pub(super) const fn stw(size: f32, weight: f32) -> TextStyle {
  TextStyle { size, weight }
}

/// The panel's colours beyond the bar's theme (sidebar.css `--colLayer2`,
/// `--colLayer3`, `--colLayer0Border`, `--m3outline`).
pub(super) struct Colors {
  pub layer2: Rgba,
  pub layer2_hover: Rgba,
  pub layer3: Rgba,
  pub border0: Rgba,
  pub outline: Rgba,
}

pub(super) fn colors(t: &Theme) -> Colors {
  Colors {
    layer2: t.surface_container_high,
    layer2_hover: t.layer1_hover,
    layer3: t.layer1_hover,
    border0: t.border,
    outline: t.subtext,
  }
}

pub(super) struct Cx<'p, 'a> {
  pub p: &'p mut Painter<'a>,
  pub t: Theme,
  pub c: Colors,
  pub hits: &'p mut Vec<(Rect, Hit)>,
  pub regions: &'p mut Vec<Region>,
  pub hover: Option<Hit>,
  pub pressed: Option<Hit>,
  pub focus: Option<FieldId>,
  clips: Vec<Rect>,
  pub tr: &'p dyn Fn(&str) -> String,
  pub now: Instant,
  /// set when something on screen moves: another frame is wanted
  pub busy: bool,
  /// the spinners' angle (degrees)
  pub spin: f32,
  /// the pointer (panel DIPs), when it is over the window
  pub mouse: Option<(f32, f32)>,
}

impl<'p, 'a> Cx<'p, 'a> {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    p: &'p mut Painter<'a>,
    t: Theme,
    hits: &'p mut Vec<(Rect, Hit)>,
    regions: &'p mut Vec<Region>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    focus: Option<FieldId>,
    tr: &'p dyn Fn(&str) -> String,
    spin: f32,
  ) -> Self {
    let c = colors(&t);
    Self { p, t, c, hits, regions, hover, pressed, focus, clips: Vec::new(), tr, now: Instant::now(), busy: false, spin, mouse: None }
  }

  pub fn tr(&self, s: &str) -> String {
    (self.tr)(s)
  }

  /// A clickable area, cut to the current clip (nothing when outside it).
  pub fn hit(&mut self, r: Rect, h: Hit) {
    let r = match self.clips.last() {
      Some(c) => intersect(r, *c),
      None => Some(r),
    };
    if let Some(r) = r {
      self.hits.push((r, h));
    }
  }

  pub fn hot(&self, h: &Hit) -> bool {
    self.hover.as_ref() == Some(h)
  }

  pub fn down(&self, h: &Hit) -> bool {
    self.pressed.as_ref() == Some(h) && self.hot(h)
  }

  /// Whether `r` shows inside the current clip (off-screen items are skipped).
  pub fn visible(&self, r: Rect) -> bool {
    self.clips.last().map_or(true, |c| intersect(r, *c).is_some())
  }

  pub fn push_clip(&mut self, r: Rect) {
    let r = match self.clips.last() {
      Some(c) => intersect(r, *c).unwrap_or(Rect::new(r.x, r.y, 0.0, 0.0)),
      None => r,
    };
    self.clips.push(r);
    unsafe {
      self.p.dc.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    }
  }

  pub fn pop_clip(&mut self) {
    if self.clips.pop().is_some() {
      unsafe { self.p.dc.PopAxisAlignedClip() };
    }
  }

  /// A scrolling area: `content` DIPs of content in `rect`.
  pub fn region(&mut self, rect: Rect, id: ScrollId, content: f32, horizontal: bool) {
    let view = if horizontal { rect.w } else { rect.h };
    self.regions.push(Region { rect, id, max: (content - view).max(0.0), horizontal });
  }

  // ---------------------------------------------------------------- shapes

  pub fn round(&mut self, r: Rect, radius: f32, c: Rgba) -> anyhow::Result<()> {
    Ok(self.p.fill_round(r, radius, c)?)
  }

  /// `box-shadow: 0 6px 20px rgba(0 0 0 / 45%)`, a few soft layers.
  pub fn shadow(&mut self, r: Rect, radius: f32, strength: f32) -> anyhow::Result<()> {
    for k in 1..=8 {
      let g = k as f32 * 2.2;
      self.p.fill_round(Rect::new(r.x - g, r.y + 5.0 - g, r.w + 2.0 * g, r.h + 2.0 * g), radius + g, Rgba(0, 0, 0, 0.05 * strength))?;
    }
    Ok(())
  }

  /// A filled polygon (the Windows logo of the uptime pill and the keycaps).
  pub fn polygon(&mut self, points: &[(f32, f32)], c: Rgba) -> anyhow::Result<()> {
    if points.len() < 3 {
      return Ok(());
    }
    let brush = self.p.brush(c)?;
    unsafe {
      let factory: ID2D1Factory = self.p.dc.GetFactory()?;
      let geo = factory.CreatePathGeometry()?;
      let sink = geo.Open()?;
      sink.BeginFigure(pt(points[0].0, points[0].1), D2D1_FIGURE_BEGIN_FILLED);
      for &(x, y) in &points[1..] {
        sink.AddLine(pt(x, y));
      }
      sink.EndFigure(D2D1_FIGURE_END_CLOSED);
      sink.Close()?;
      self.p.dc.FillGeometry(&geo, &brush, None);
    }
    Ok(())
  }

  /// The four panes of the Windows logo in a `size` square at (x, y)
  /// (sidebar.html's `.distro` path, a 24 x 24 view box).
  pub fn windows_logo(&mut self, x: f32, y: f32, size: f32, c: Rgba) -> anyhow::Result<()> {
    let k = size / 24.0;
    let panes: [[(f32, f32); 4]; 4] = [
      [(3.0, 5.5), (10.0, 4.5), (10.0, 11.5), (3.0, 11.5)],
      [(11.0, 4.4), (21.0, 3.0), (21.0, 11.5), (11.0, 11.5)],
      [(3.0, 12.5), (10.0, 12.5), (10.0, 19.5), (3.0, 18.5)],
      [(11.0, 12.5), (21.0, 12.5), (21.0, 21.0), (11.0, 19.6)],
    ];
    for pane in panes {
      let pts: Vec<(f32, f32)> = pane.iter().map(|(px, py)| (x + px * k, y + py * k)).collect();
      self.polygon(&pts, c)?;
    }
    Ok(())
  }

  /// A horizontal gradient (the night light's warmth track).
  pub fn gradient(&mut self, r: Rect, radius: f32, stops: &[(f32, Rgba)]) -> anyhow::Result<()> {
    unsafe {
      let s: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|(at, c)| D2D1_GRADIENT_STOP { position: *at, color: (*c).into() }).collect();
      let collection = self.p.dc.CreateGradientStopCollection(
        &s,
        D2D1_COLOR_SPACE_SRGB,
        D2D1_COLOR_SPACE_SRGB,
        D2D1_BUFFER_PRECISION_8BPC_UNORM,
        D2D1_EXTEND_MODE_CLAMP,
        D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
      )?;
      let brush = self.p.dc.CreateLinearGradientBrush(
        &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(r.x, r.y), endPoint: pt(r.right(), r.y) },
        None,
        &collection,
      )?;
      self.p.dc.FillRoundedRectangle(&r.rounded(radius), &brush);
    }
    Ok(())
  }

  /// Draws with a transform (rotation, scale) around a point, then restores.
  pub fn transformed(&mut self, m: Matrix3x2, f: impl FnOnce(&mut Self) -> anyhow::Result<()>) -> anyhow::Result<()> {
    let mut before = Matrix3x2::default();
    unsafe {
      self.p.dc.GetTransform(&mut before);
      self.p.dc.SetTransform(&(m * before));
    }
    let r = f(self);
    unsafe { self.p.dc.SetTransform(&before) };
    r
  }

  /// `progress_activity`, turning (the busy indicator).
  pub fn spinner(&mut self, cx: f32, cy: f32, size: f32, c: Rgba) -> anyhow::Result<()> {
    self.busy = true;
    let m = Matrix3x2::rotation(self.spin, cx, cy);
    self.transformed(m, |cx2| Ok(cx2.p.icon("progress_activity", cx, cy, size, false, c)?))
  }

  // ---------------------------------------------------------------- widgets

  pub fn icon(&mut self, name: &str, cx: f32, cy: f32, size: f32, fill: bool, c: Rgba) -> anyhow::Result<()> {
    Ok(self.p.icon(name, cx, cy, size, fill, c)?)
  }

  pub fn text(&mut self, s: &str, r: Rect, style: TextStyle, c: Rgba) -> anyhow::Result<f32> {
    self.p.text(s, r, style, c, Align::Left, false)
  }

  pub fn text_center(&mut self, s: &str, r: Rect, style: TextStyle, c: Rgba) -> anyhow::Result<f32> {
    self.p.text(s, r, style, c, Align::Center, false)
  }

  /// Right-aligned single line; returns its width.
  pub fn text_right(&mut self, s: &str, r: Rect, style: TextStyle, c: Rgba) -> anyhow::Result<f32> {
    let w = self.measure(s, style)?.min(r.w);
    self.p.text(s, Rect::new(r.right() - w - 1.0, r.y, w + 2.0, r.h), style, c, Align::Left, false)?;
    Ok(w)
  }

  pub fn measure(&mut self, s: &str, style: TextStyle) -> anyhow::Result<f32> {
    self.p.measure(s, style)
  }

  /// Wrapped text height for `w`.
  pub fn wrapped_h(&mut self, s: &str, style: TextStyle, w: f32, max_h: f32) -> anyhow::Result<f32> {
    self.p.measure_wrapped(s, style, w, max_h, false)
  }

  /// `.chip`: 32 high, 12 padding, an 18 icon then the label.
  pub fn chip_w(&mut self, label: &str, icon: Option<&str>) -> anyhow::Result<f32> {
    let tw = if label.is_empty() { 0.0 } else { self.measure(label, st(13.0))?.ceil() };
    let iw = if icon.is_some() { 18.0 + if label.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
    Ok(24.0 + iw + tw)
  }

  /// `.chip` / `.chip.on`; returns its width. Disabled: dimmed, not clickable.
  #[allow(clippy::too_many_arguments)]
  pub fn chip(&mut self, x: f32, y: f32, h: f32, label: &str, icon: Option<&str>, on: bool, enabled: bool, hit: Hit) -> anyhow::Result<f32> {
    let w = self.chip_w(label, icon)?;
    self.chip_in(Rect::new(x, y, w, h), label, icon, on, enabled, hit)?;
    Ok(w)
  }

  /// A chip filling `r` (content centred).
  pub fn chip_in(&mut self, r: Rect, label: &str, icon: Option<&str>, on: bool, enabled: bool, hit: Hit) -> anyhow::Result<()> {
    let hot = enabled && self.hot(&hit);
    let (bg, fg) = if on {
      (self.t.sec_container, self.t.on_sec_container)
    } else {
      (if hot { self.c.layer2_hover } else { self.c.layer2 }, self.t.on_layer1)
    };
    let a = if enabled { 1.0 } else { 0.4 };
    self.p.fill_round(r, r.h / 2.0, bg.alpha(bg.3 * a))?;
    let content = self.chip_w(label, icon)? - 24.0;
    let mut x = r.x + (r.w - content) / 2.0;
    if let Some(i) = icon {
      self.p.icon(i, x + 9.0, r.y + r.h / 2.0, 18.0, on, fg.alpha(a))?;
      x += 18.0 + 6.0;
    }
    if !label.is_empty() {
      self.p.text(label, Rect::new(x, r.y, r.right() - x, r.h), st(13.0), fg.alpha(a), Align::Left, false)?;
    }
    if enabled {
      self.hit(r, hit);
    }
    Ok(())
  }

  /// A round button (`.qbtn`, `.circle-btn`): hover background, icon.
  #[allow(clippy::too_many_arguments)]
  pub fn round_btn(&mut self, r: Rect, icon: &str, size: f32, fill: bool, bg: Option<Rgba>, fg: Rgba, hit: Hit) -> anyhow::Result<()> {
    let hot = self.hot(&hit);
    let radius = if hot && bg.is_some() { 15.0_f32.min(r.h / 2.0) } else { r.h / 2.0 };
    match (bg, hot) {
      (Some(c), true) => self.p.fill_round(r, radius, blend(c, self.t.on_layer1, 0.08))?,
      (Some(c), false) => self.p.fill_round(r, radius, c)?,
      (None, true) => self.p.fill_round(r, r.h / 2.0, self.t.layer1_hover)?,
      (None, false) => {}
    }
    self.p.icon(icon, r.x + r.w / 2.0, r.y + r.h / 2.0, size, fill, fg)?;
    self.hit(r, hit);
    Ok(())
  }

  /// `.switch` (52 x 32): the knob moves right and grows when on.
  pub fn switch(&mut self, x: f32, y: f32, on: bool) -> anyhow::Result<()> {
    let r = Rect::new(x, y, 52.0, 32.0);
    if on {
      self.p.fill_round(r, 16.0, self.t.primary)?;
      self.p.fill_circle(x + 24.0 + 12.0 + 2.0, y + 16.0, 12.0, self.t.on_primary)?;
    } else {
      self.p.fill_round(r, 16.0, self.c.layer3)?;
      self.p.stroke_round(r.inset(1.0, 1.0), 15.0, self.c.outline, 2.0)?;
      self.p.fill_circle(x + 6.0 + 8.0 + 2.0, y + 16.0, 8.0, self.c.outline)?;
    }
    Ok(())
  }

  /// An 18 x 18 checkbox (`accent-color: primary`).
  pub fn checkbox(&mut self, x: f32, y: f32, size: f32, on: bool) -> anyhow::Result<()> {
    let r = Rect::new(x, y, size, size);
    if on {
      self.p.fill_round(r, 3.0, self.t.primary)?;
      self.p.icon("check", x + size / 2.0, y + size / 2.0, size - 2.0, true, self.t.on_primary)?;
    } else {
      self.p.stroke_round(r.inset(0.75, 0.75), 3.0, self.c.outline, 1.5)?;
    }
    Ok(())
  }

  /// `.kb-gtitle`: a 17 icon and a 13 label in the secondary colour.
  pub fn section_title(&mut self, x: f32, y: f32, w: f32, icon: &str, label: &str) -> anyhow::Result<f32> {
    let fg = self.t.on_surface_variant;
    self.p.icon(icon, x + 8.0 + 8.5, y + 13.0, 17.0, false, fg)?;
    self.p.text(label, Rect::new(x + 8.0 + 17.0 + 6.0, y + 4.0, w - 40.0, 18.0), st(13.0), fg, Align::Left, false)?;
    Ok(28.0)
  }

  /// `.kb-hint`: a 16 info icon and wrapped 12 text; returns its height.
  pub fn hint(&mut self, x: f32, y: f32, w: f32, text: &str) -> anyhow::Result<f32> {
    let fg = self.t.on_surface_variant;
    let tw = w - 4.0 - 16.0 - 6.0 - 4.0;
    let h = self.wrapped_h(text, st(12.0), tw, 200.0)?.max(16.0);
    self.p.icon("info", x + 4.0 + 8.0, y + 8.0, 16.0, false, fg)?;
    self.p.text_wrapped(text, Rect::new(x + 4.0 + 16.0 + 6.0, y, tw, h + 2.0), st(12.0), fg, false)?;
    Ok(h)
  }

  /// `.kb-msg.ok` / `.kb-msg.error`; returns its height.
  pub fn message(&mut self, x: f32, y: f32, w: f32, ok: bool, text: &str) -> anyhow::Result<f32> {
    let (bg, fg) = if ok {
      (self.t.sec_container, self.t.on_sec_container)
    } else {
      (Rgba::hex(0x93000a), Rgba::hex(0xffdad6))
    };
    let tw = w - 12.0 - 18.0 - 8.0 - 12.0;
    let th = self.wrapped_h(text, st(13.0), tw, 200.0)?.max(18.0);
    let h = th + 16.0;
    self.p.fill_round(Rect::new(x, y, w, h), 12.0, bg)?;
    self.p.icon(if ok { "check_circle" } else { "error" }, x + 12.0 + 9.0, y + h / 2.0, 18.0, false, fg)?;
    self.p.text_wrapped(text, Rect::new(x + 12.0 + 18.0 + 8.0, y + 8.0, tw, th + 2.0), st(13.0), fg, false)?;
    Ok(h)
  }

  /// A rounded track with the filled part and a thin handle (the audio
  /// and the night light cards: `input[type=range]` with a 5 x 26 thumb).
  pub fn slider(&mut self, track: Rect, frac: f32, fill: Rgba, rest: Rgba, hit: Hit) -> anyhow::Result<()> {
    let f = frac.clamp(0.0, 1.0);
    self.p.fill_round(track, track.h / 2.0, rest)?;
    if f > 0.0 {
      self.p.fill_round(Rect::new(track.x, track.y, (track.w * f).max(track.h), track.h), track.h / 2.0, fill)?;
    }
    self.thumb(track, f)?;
    self.hit(Rect::new(track.x - 6.0, track.y - 10.0, track.w + 12.0, track.h + 20.0), hit);
    Ok(())
  }

  /// The slider handle: a 5 x 26 primary bar with a ring of the card colour.
  pub fn thumb(&mut self, track: Rect, f: f32) -> anyhow::Result<()> {
    let x = track.x + track.w * f.clamp(0.0, 1.0);
    let cy = track.y + track.h / 2.0;
    self.p.fill_round(Rect::new(x - 5.5, cy - 16.0, 11.0, 32.0), 5.5, self.c.layer2)?;
    self.p.fill_round(Rect::new(x - 2.5, cy - 13.0, 5.0, 26.0), 2.5, self.t.primary)?;
    Ok(())
  }

  /// A text field box (`border: 1px solid outline-variant`, primary when
  /// focused) with the field drawn inside.
  #[allow(clippy::too_many_arguments)]
  pub fn field_box(&mut self, r: Rect, radius: f32, f: &mut TextField, id: FieldId, placeholder: &str, style: TextStyle, pad_x: f32, bg: Option<Rgba>) -> anyhow::Result<()> {
    let focused = self.focus == Some(id);
    if let Some(bg) = bg {
      self.p.fill_round(r, radius, bg)?;
    }
    let border = if focused { self.t.primary } else { self.t.outline_variant };
    self.p.stroke_round(r.inset(0.5, 0.5), radius, border, 1.0)?;
    let inner = if f.multiline {
      Rect::new(r.x + pad_x, r.y + 10.0, r.w - 2.0 * pad_x, r.h - 20.0)
    } else {
      Rect::new(r.x + pad_x, r.y, r.w - 2.0 * pad_x, r.h)
    };
    let color = self.t.on_layer1;
    f.paint(self.p, &self.t, inner, style, color, placeholder, focused)?;
    self.hit(r, Hit::Field(id));
    Ok(())
  }
}

pub(super) fn intersect(a: Rect, b: Rect) -> Option<Rect> {
  let x = a.x.max(b.x);
  let y = a.y.max(b.y);
  let r = a.right().min(b.right());
  let btm = a.bottom().min(b.bottom());
  (r > x && btm > y).then(|| Rect::new(x, y, r - x, btm - y))
}

/// `a` with `b` mixed in (hover tints).
pub(super) fn blend(a: Rgba, b: Rgba, k: f32) -> Rgba {
  let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round().clamp(0.0, 255.0) as u8;
  Rgba(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2), a.3)
}

pub(super) fn lerp(a: f32, b: f32, k: f32) -> f32 {
  a + (b - a) * k
}

pub(super) fn lerp_rect(a: Rect, b: Rect, k: f32) -> Rect {
  Rect::new(lerp(a.x, b.x, k), lerp(a.y, b.y, k), lerp(a.w, b.w, k), lerp(a.h, b.h, k))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rectangles_intersect() {
    let a = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert_eq!(intersect(a, Rect::new(5.0, 5.0, 10.0, 10.0)), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
    assert_eq!(intersect(a, Rect::new(20.0, 0.0, 5.0, 5.0)), None);
  }

  #[test]
  fn blending_mixes_channels() {
    assert_eq!(blend(Rgba(0, 0, 0, 1.0), Rgba(255, 255, 255, 1.0), 0.5), Rgba(128, 128, 128, 1.0));
  }
}
