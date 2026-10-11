//! The custom accent's colour picker (settings.html `.picker`): the dot left
//! of the hex field opens it by the colour row. A saturation / brightness
//! square and a hue bar; the hex field follows it and it follows the field.
//!
//! The colour is applied when the mouse lets go and a moment after the last
//! arrow key, not at every move: the core rewrites config.yaml, reloads the
//! window manager and every shell part repaints for each colour. Esc, a
//! click outside or the dot again close it (what was applied stays).

use windows::Win32::Graphics::Direct2D::{
  Common::D2D1_GRADIENT_STOP, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
  D2D1_COLOR_SPACE_SRGB, D2D1_EXTEND_MODE_CLAMP, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
};

use super::{
  color::{hsv_to_rgb, parse_hex, rgb_to_hsv, to_hex, Hsv},
  widgets::{layer2, Ctx, Cursor, SUB},
  Hit, Settings,
};
use crate::native_bar::{
  fonts::TextStyle,
  gfx::{pt, Rect, Rgba},
  view::{Align, Painter},
};

/// The popover (DIP)
pub const W: f32 = 296.0;
pub const H: f32 = 282.0;
const PAD: f32 = 16.0;
const SV_TOP: f32 = 44.0;
const SV_H: f32 = 150.0;
const HUE_TOP: f32 = SV_TOP + SV_H + 16.0;
const HUE_H: f32 = 12.0;
const FOOT_TOP: f32 = HUE_TOP + HUE_H + 16.0;
/// arrow keys: a small step, with Shift a big one
const STEP: f32 = 0.01;
const BIG: f32 = 0.1;
const HUE_STEP: f32 = 1.0;
const HUE_BIG: f32 = 10.0;

/// The picker's parts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(in crate::native_bar) enum Part {
  /// saturation (across) and brightness (up)
  Sv,
  Hue,
  /// the rest of the popover (a click there is not a click outside)
  Panel,
}

pub(in crate::native_bar) struct Picker {
  pub hsv: Hsv,
  /// the colour when it opened (the preview's left half)
  pub was: (u8, u8, u8),
}

impl Picker {
  pub fn new(hex: &str) -> Self {
    let rgb = parse_hex(hex).unwrap_or((0xb6, 0x9d, 0xf8));
    Picker { hsv: rgb_to_hsv(rgb, 0.0), was: rgb }
  }

  pub fn rgb(&self) -> (u8, u8, u8) {
    hsv_to_rgb(self.hsv)
  }

  pub fn hex(&self) -> String {
    to_hex(self.rgb())
  }

  /// The hex field changed: follow it when it is a whole colour (the hue
  /// stays when it is a grey).
  pub fn follow(&mut self, text: &str) -> bool {
    match parse_hex(text) {
      Some(rgb) if rgb != self.rgb() => {
        self.hsv = rgb_to_hsv(rgb, self.hsv.h);
        true
      }
      _ => false,
    }
  }

  /// The pointer at (x, y) over a part `r` (window DIP).
  pub fn drag(&mut self, part: Part, r: Rect, x: f32, y: f32) {
    let kx = ((x - r.x) / r.w.max(1.0)).clamp(0.0, 1.0);
    match part {
      Part::Sv => {
        self.hsv.s = kx;
        self.hsv.v = 1.0 - ((y - r.y) / r.h.max(1.0)).clamp(0.0, 1.0);
      }
      Part::Hue => self.hsv.h = kx * 360.0,
      Part::Panel => {}
    }
  }

  /// An arrow key on a part: dx across, dy up (+1 / -1).
  pub fn nudge(&mut self, part: Part, dx: i32, dy: i32, big: bool) {
    match part {
      Part::Sv => {
        let k = if big { BIG } else { STEP };
        self.hsv.s = (self.hsv.s + dx as f32 * k).clamp(0.0, 1.0);
        self.hsv.v = (self.hsv.v + dy as f32 * k).clamp(0.0, 1.0);
      }
      Part::Hue => {
        let k = if big { HUE_BIG } else { HUE_STEP };
        self.hsv.h = (self.hsv.h + (dx + dy) as f32 * k).clamp(0.0, 360.0);
      }
      Part::Panel => {}
    }
  }
}

fn rgba((r, g, b): (u8, u8, u8)) -> Rgba {
  Rgba(r, g, b, 1.0)
}

/// A linear gradient from `a` to `b` filling a rounded rectangle.
fn gradient(p: &mut Painter, r: Rect, radius: f32, a: (f32, f32), b: (f32, f32), stops: &[(f32, Rgba)]) -> anyhow::Result<()> {
  unsafe {
    let s: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|(at, c)| D2D1_GRADIENT_STOP { position: *at, color: (*c).into() }).collect();
    let collection = p.dc.CreateGradientStopCollection(
      &s,
      D2D1_COLOR_SPACE_SRGB,
      D2D1_COLOR_SPACE_SRGB,
      D2D1_BUFFER_PRECISION_8BPC_UNORM,
      D2D1_EXTEND_MODE_CLAMP,
      D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
    )?;
    let brush = p.dc.CreateLinearGradientBrush(
      &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(a.0, a.1), endPoint: pt(b.0, b.1) },
      None,
      &collection,
    )?;
    p.dc.FillRoundedRectangle(&r.rounded(radius), &brush);
  }
  Ok(())
}

/// A round handle: a white ring around the colour, a soft edge for light
/// backgrounds.
fn thumb(p: &mut Painter, x: f32, y: f32, c: Rgba, big: bool) -> anyhow::Result<()> {
  let r = if big { 10.0 } else { 9.0 };
  p.fill_circle(x, y + 1.0, r + 1.5, Rgba(0, 0, 0, 0.22))?;
  p.fill_circle(x, y, r, Rgba(255, 255, 255, 1.0))?;
  p.fill_circle(x, y, r - 3.0, c)?;
  Ok(())
}

/// Where the popover goes: under the colour row, or over it when the page
/// has no room below; always inside the page's view.
pub fn place(anchor: Rect, view: Rect) -> Rect {
  let x = (anchor.x - 6.0).clamp(view.x + 8.0, (view.right() - 8.0 - W).max(view.x + 8.0));
  let below = anchor.bottom() + 14.0;
  let above = anchor.y - 14.0 - H;
  let y = if below + H <= view.bottom() - 8.0 || above < view.y + 8.0 { below } else { above };
  let y = y.clamp(view.y + 8.0, (view.bottom() - 8.0 - H).max(view.y + 8.0));
  Rect::new(x, y, W, H)
}

/// The popover, over everything (`anchor`: the dot, `view`: the page).
pub fn paint(cx: &mut Ctx, s: &Settings, anchor: Rect, view: Rect) -> anyhow::Result<()> {
  let Some(pk) = &s.picker else { return Ok(()) };
  let t = cx.t;
  let r = place(anchor, view);
  // the menus' shadow and surface
  for k in 1..=6 {
    let g = k as f32 * 2.0;
    cx.p.fill_round(Rect::new(r.x - g, r.y + 4.0 - g, r.w + 2.0 * g, r.h + 2.0 * g), 20.0 + g, Rgba(0, 0, 0, 0.035))?;
  }
  cx.p.fill_round(r, 20.0, layer2(t))?;
  cx.p.stroke_round(r, 20.0, t.outline_variant, 1.0)?;
  cx.push(r, Hit::Pick(Part::Panel), false, Cursor::Arrow);

  let now = pk.rgb();
  let hex = to_hex(now);
  let title = cx.tr("Renk seç");
  let iw = W - 2.0 * PAD;
  cx.p.text(&title, Rect::new(r.x + PAD, r.y + 12.0, iw - 80.0, 22.0), TextStyle { size: 14.0, weight: 560.0 }, t.on_layer1, Align::Left, false)?;
  let hw = cx.p.measure_with(&hex, SUB, true)?;
  cx.p.text(&hex, Rect::new(r.right() - PAD - hw - 2.0, r.y + 12.0, hw + 4.0, 22.0), SUB, t.on_surface_variant, Align::Left, true)?;

  // saturation across, brightness down: the hue, white to the left, black below
  let sv = Rect::new(r.x + PAD, r.y + SV_TOP, iw, SV_H);
  let pure = rgba(hsv_to_rgb(Hsv { h: pk.hsv.h, s: 1.0, v: 1.0 }));
  cx.p.fill_round(sv, 12.0, pure)?;
  let white = Rgba(255, 255, 255, 1.0);
  let black = Rgba(0, 0, 0, 1.0);
  gradient(cx.p, sv, 12.0, (sv.x, sv.y), (sv.right(), sv.y), &[(0.0, white), (1.0, white.alpha(0.0))])?;
  gradient(cx.p, sv, 12.0, (sv.x, sv.y), (sv.x, sv.bottom()), &[(0.0, black.alpha(0.0)), (1.0, black)])?;
  cx.p.stroke_round(sv, 12.0, t.outline_variant.alpha(0.6), 1.0)?;
  let hot = cx.hot(Hit::Pick(Part::Sv)) || s.drag == Some(Hit::Pick(Part::Sv));
  thumb(cx.p, sv.x + pk.hsv.s * sv.w, sv.y + (1.0 - pk.hsv.v) * sv.h, rgba(now), hot)?;
  cx.push(sv, Hit::Pick(Part::Sv), true, Cursor::Cross);
  cx.ring(sv, 12.0, Hit::Pick(Part::Sv))?;

  // the hue bar: red, yellow, green, cyan, blue, magenta, red
  let bar = Rect::new(r.x + PAD, r.y + HUE_TOP, iw, HUE_H);
  let stops: Vec<(f32, Rgba)> = (0..=6).map(|i| (i as f32 / 6.0, rgba(hsv_to_rgb(Hsv { h: i as f32 * 60.0, s: 1.0, v: 1.0 })))).collect();
  gradient(cx.p, bar, HUE_H / 2.0, (bar.x, bar.y), (bar.right(), bar.y), &stops)?;
  let hot = cx.hot(Hit::Pick(Part::Hue)) || s.drag == Some(Hit::Pick(Part::Hue));
  thumb(cx.p, bar.x + pk.hsv.h / 360.0 * bar.w, bar.y + HUE_H / 2.0, pure, hot)?;
  // the hit area spans more than the bar
  let area = Rect::new(bar.x, bar.y - 8.0, bar.w, HUE_H + 16.0);
  cx.push(area, Hit::Pick(Part::Hue), true, Cursor::Hand);
  cx.ring(Rect::new(bar.x - 6.0, area.y, bar.w + 12.0, area.h), 14.0, Hit::Pick(Part::Hue))?;

  // before | now, and when it applies
  let pill = Rect::new(r.x + PAD, r.y + FOOT_TOP, 72.0, 28.0);
  cx.p.fill_round(pill, 14.0, rgba(pk.was))?;
  cx.p.fill_round(Rect::new(pill.x + 36.0, pill.y, 36.0, 28.0), 14.0, rgba(now))?;
  cx.p.fill(Rect::new(pill.x + 36.0, pill.y, 14.0, 28.0), rgba(now))?;
  cx.p.stroke_round(pill, 14.0, t.outline_variant, 1.0)?;
  let hint = cx.tr("Bırakınca uygulanır");
  let hx = pill.right() + 12.0;
  cx.p.text(&hint, Rect::new(hx, pill.y, r.right() - PAD - hx, 28.0), SUB, t.on_surface_variant, Align::Left, false)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn dragging_maps_the_square_and_the_bar() {
    let mut p = Picker::new("#ff0000");
    let sq = Rect::new(10.0, 20.0, 200.0, 100.0);
    p.drag(Part::Sv, sq, 110.0, 45.0);
    assert!((p.hsv.s - 0.5).abs() < 1e-5 && (p.hsv.v - 0.75).abs() < 1e-5);
    // outside the square sticks to its edges
    p.drag(Part::Sv, sq, -50.0, 500.0);
    assert_eq!((p.hsv.s, p.hsv.v), (0.0, 0.0));
    p.drag(Part::Hue, sq, 210.0, 0.0);
    assert_eq!(p.hsv.h, 360.0);
    p.drag(Part::Hue, sq, 60.0, 0.0);
    assert!((p.hsv.h - 90.0).abs() < 1e-4);
  }

  #[test]
  fn arrows_nudge_and_the_field_leads() {
    let mut p = Picker::new("#808080");
    p.hsv.h = 200.0;
    p.nudge(Part::Hue, 1, 0, true);
    assert_eq!(p.hsv.h, 210.0);
    p.nudge(Part::Hue, -1, 0, false);
    assert_eq!(p.hsv.h, 209.0);
    p.nudge(Part::Sv, 1, 1, false);
    assert!((p.hsv.s - 0.01).abs() < 1e-5);
    // a grey typed in keeps the hue; a half-typed colour is ignored
    assert!(p.follow("#404040"));
    assert_eq!(p.hsv.h, 209.0);
    assert!(!p.follow("#40404"));
    assert!(p.follow("b69df8"));
    assert_eq!(p.hex(), "#b69df8");
  }

  #[test]
  fn the_popover_stays_in_the_page() {
    let view = Rect::new(236.0, 80.0, 640.0, 536.0);
    // room below: under the row
    let r = place(Rect::new(282.0, 200.0, 24.0, 24.0), view);
    assert_eq!(r.y, 238.0);
    // no room below: over it
    let r = place(Rect::new(282.0, 560.0, 24.0, 24.0), view);
    assert!(r.bottom() <= 560.0);
    // scrolled far away: still inside
    let r = place(Rect::new(282.0, -900.0, 24.0, 24.0), view);
    assert!(r.y >= view.y && r.bottom() <= view.bottom());
  }
}
