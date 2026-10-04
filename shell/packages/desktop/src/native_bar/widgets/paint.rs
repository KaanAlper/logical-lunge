//! Drawing the desktop widgets: a card in the theme's colours and each
//! kind's contents. Returns the clickable parts.

use windows::{
  core::Interface,
  Win32::Graphics::{
    Direct2D::{ID2D1Bitmap1, ID2D1Image, ID2D1Layer, ID2D1Factory, ID2D1Geometry, ID2D1PathGeometry,
      D2D1_LAYER_PARAMETERS1, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_DRAW_TEXT_OPTIONS_NONE,
      Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_WINDING}},
    DirectWrite::{IDWriteTextLayout, DWRITE_HIT_TEST_METRICS},
  },
};

use super::{
  layout::{ClockStyle, Kind, Spec},
  weather::{self, Report},
  shape,
};
use super::super::{
  fonts::{TextStyle, with_widget_font},
  gfx::{pt, Rect, Rgba},
  view::{Align, Painter, Theme},
};

pub const RADIUS: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
  Prev,
  Play,
  Next,
  /// the note's text (editing)
  Text,
  Grip,
  Settings,
}

pub struct Clock {
  pub h: u32,
  pub m: u32,
  pub s: u32,
  pub time: String,
  pub date: String,
}

pub struct Media<'a> {
  pub title: String,
  pub artist: String,
  pub playing: bool,
  /// 0..1 (None: no timeline)
  pub progress: Option<f32>,
  pub art: Option<&'a ID2D1Bitmap1>,
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct System {
  pub cpu: Option<f32>,
  pub ram: Option<f32>,
  pub gpu: Option<f32>,
  pub cpu_temp: Option<f32>,
  pub gpu_temp: Option<f32>,
}

/// Everything a widget may show; each kind reads its part.
pub struct Data<'a> {
  pub clock: Clock,
  pub media: Option<Media<'a>>,
  pub system: System,
  pub weather: Option<&'a Result<Report, String>>,
  pub day_big: String,
  pub day_line: String,
  pub todos: &'a [String],
  pub tr: &'a dyn Fn(&str) -> String,
}

/// A note being edited: its text with the caret at `caret` (characters).
pub struct Editing<'a> {
  pub text: &'a str,
  pub caret: usize,
  pub selection: (usize, usize),
}

const SMALL: TextStyle = TextStyle { size: 12.5, weight: 450.0 };
const BODY: TextStyle = TextStyle { size: 14.0, weight: 450.0 };

thread_local! { static GLYPH_OUTLINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
const OUTLINE_OFFSETS: [(f32,f32);8] = [(-0.8,0.0),(0.8,0.0),(0.0,-0.8),(0.0,0.8),(-0.6,-0.6),(0.6,-0.6),(-0.6,0.6),(0.6,0.6)];
fn opposing(c: Rgba) -> Rgba { if 0.2126*c.0 as f32+0.7152*c.1 as f32+0.0722*c.2 as f32 > 140.0 { Rgba(12,15,19,c.3) } else { Rgba(250,250,247,c.3) } }
fn with_text_outline<T>(outline: bool, draw: impl FnOnce() -> T) -> T {
  struct Reset(bool); impl Drop for Reset { fn drop(&mut self) { GLYPH_OUTLINE.with(|flag| flag.set(self.0)); } }
  let _reset = Reset(GLYPH_OUTLINE.with(|flag| flag.replace(outline))); draw()
}
fn glyph_text(p: &mut Painter, value: &str, rect: Rect, style: TextStyle, color: Rgba, align: Align, tabular: bool) -> anyhow::Result<()> {
  if GLYPH_OUTLINE.with(|flag| flag.get()) {
    for (dx,dy) in OUTLINE_OFFSETS { p.text(value,Rect::new(rect.x+dx,rect.y+dy,rect.w,rect.h),style,opposing(color),align,tabular)?; }
  }
  p.text(value,rect,style,color,align,tabular).map(|_| ())
}
fn glyph_icon(p: &mut Painter, value: &str, x: f32, y: f32, size: f32, filled: bool, color: Rgba) -> anyhow::Result<()> {
  if GLYPH_OUTLINE.with(|flag| flag.get()) {
    for (dx,dy) in OUTLINE_OFFSETS { p.icon(value,x+dx,y+dy,size,filled,opposing(color))?; }
  }
  p.icon(value,x,y,size,filled,color)
}

fn geometry(p: &Painter, plan: &shape::Plan) -> anyhow::Result<ID2D1PathGeometry> {
  unsafe {
    let factory: ID2D1Factory = p.dc.GetFactory()?; let geometry = factory.CreatePathGeometry()?; let sink = geometry.Open()?;
    sink.SetFillMode(D2D1_FILL_MODE_WINDING);
    for points in &plan.contours {
      sink.BeginFigure(pt(points[0].0,points[0].1),D2D1_FIGURE_BEGIN_FILLED);
      for &(x,y) in &points[1..] { sink.AddLine(pt(x,y)); }
      sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    }
    sink.Close()?; Ok(geometry)
  }
}

fn with_clip<T>(p: &mut Painter, bounds: Rect, geometry: &ID2D1PathGeometry, draw: impl FnOnce(&mut Painter) -> anyhow::Result<T>) -> anyhow::Result<T> {
  let mask: ID2D1Geometry = geometry.cast()?;
  let mut params = D2D1_LAYER_PARAMETERS1 { contentBounds: bounds.d2d(), opacity: 1.0,
    geometricMask: std::mem::ManuallyDrop::new(Some(mask)), maskTransform: windows::Foundation::Numerics::Matrix3x2::identity(), ..Default::default() };
  unsafe { p.dc.PushLayer(&params,None::<&ID2D1Layer>); }
  let result = draw(p);
  unsafe { p.dc.PopLayer(); std::mem::ManuallyDrop::drop(&mut params.geometricMask); }
  result
}

pub fn paint(
  p: &mut Painter,
  t: &Theme,
  s: &Spec,
  d: &Data,
  hover: bool,
  editing: Option<&Editing>,
  note_layout: &mut Option<IDWriteTextLayout>,
) -> anyhow::Result<Vec<(Rect, Hit)>> {
  let card = Rect::new(0.0, 0.0, s.w, s.h);
  let plan = shape::plan(&s.shape,s.w,s.h); let geometry = geometry(p,&plan)?;
  with_clip(p,card,&geometry, |p| {
  with_opacity(p, card, s.background_opacity, |p| material_shaped(p, t, s, card, hover, &plan, &geometry))?;
  let themed = material_theme(t, &s.appearance);
  let t = &themed;
  let inner = plan.inner;
  let mut hits = Vec::new();
  with_widget_font(&s.appearance, || with_text_outline(s.appearance == "outline", || with_opacity(p, card, s.content_opacity, |p| {
    if let Some(lead) = plan.lead.filter(|_| matches!(s.kind,Kind::Note | Kind::System)) {
      glyph_icon(p,s.kind.icon(),lead.x+lead.w/2.0,lead.y+lead.h/2.0,42.0,false,t.primary)?;
    }
    match s.kind {
    Kind::Clock => clock_shaped(p,t,s,d,&plan)?,
    Kind::Media => media(p, t, d, inner, &plan, &mut hits)?,
    Kind::System => system(p, t, s, d, inner)?,
    Kind::Weather => weather_card(p, t, s, d, inner, &plan)?,
    Kind::Agenda => agenda(p, t, d, inner, &plan)?,
    Kind::Note => note(p, t, s, d, inner, editing, note_layout, &mut hits)?,
    }
    if let Some(caption) = plan.caption {
      let caption_text = match s.kind {
        Kind::Clock => d.clock.date.clone(), Kind::Weather => d.weather.and_then(|w| w.as_ref().ok()).map(|w| w.place.clone()).unwrap_or_default(),
        Kind::Media => d.media.as_ref().map(|m| m.artist.clone()).unwrap_or_default(), _ => (d.tr)(s.kind.label()),
      };
      glyph_text(p,&caption_text,caption,SMALL,t.on_surface_variant,Align::Center,false)?;
    }
    Ok(())
  })))?;
  let grip = plan.grip;
  if hover {
    let brush = p.brush(t.primary.alpha(0.65))?; unsafe { p.dc.DrawGeometry(&geometry,&brush,1.0,None); }
    // three dots along the corner
    for (dx, dy) in [(12.0, 4.0), (8.0, 8.0), (4.0, 12.0), (12.0, 8.0), (8.0, 12.0), (12.0, 12.0)] {
      p.fill_circle(grip.x + dx, grip.y + dy, 1.3, t.on_surface_variant.alpha(0.7))?;
    }
  }
  hits.push((grip, Hit::Grip));
  // This control is outside both opacity layers, so even a fully hidden
  // widget can be found and restored with its native settings menu.
  let settings = plan.settings;
  if hover || (s.background_opacity <= 0.05 && s.content_opacity <= 0.05) {
    p.fill_round(settings, 8.0, t.layer0.alpha(if hover { 0.95 } else { 0.5 }))?;
    glyph_icon(p, "tune", settings.x + 12.0, settings.y + 12.0, 16.0, false, t.primary.alpha(if hover { 1.0 } else { 0.55 }))?;
  }
  hits.push((settings, Hit::Settings));
  Ok(hits)
  })
}

fn material_shaped(p: &mut Painter, t: &Theme, s: &Spec, card: Rect, hover: bool, plan: &shape::Plan, geometry: &ID2D1PathGeometry) -> anyhow::Result<()> {
  if matches!(s.appearance.as_str(),"transparent" | "outline") { return Ok(()); }
  if s.shape == "card" { return material(p,t,s,card,hover); }
  let themed = material_theme(t,&s.appearance);
  let alpha = match s.appearance.as_str() { "glass" => 0.35, "futuristic" => 0.94, "paper" | "cartoon" | "pixel" => 1.0, _ => 0.97 };
  let base = p.brush(themed.layer0.alpha(alpha))?;
  unsafe { p.dc.FillGeometry(geometry,&base,None); }
  // Texture and highlights follow the mask; the rectangular card renderer
  // would fill glass twice and leave a second frame inside tickets/bubbles.
  match s.appearance.as_str() {
    "glass" | "cartoon" => {
      p.fill_round(Rect::new(plan.inner.x,plan.inner.y-7.0,plan.inner.w,2.0),1.0,
        Rgba(255,255,255,if s.appearance == "glass" { 0.35 } else { 0.65 }))?;
    }
    "futuristic" => {
      for x in (16..s.w as usize).step_by(24) { p.fill(Rect::new(x as f32,0.0,1.0,s.h),themed.primary.alpha(0.04))?; }
      for y in (16..s.h as usize).step_by(24) { p.fill(Rect::new(0.0,y as f32,s.w,1.0),themed.primary.alpha(0.04))?; }
    }
    "paper" => {
      for y in (32..s.h as usize).step_by(20) { p.fill(Rect::new(0.0,y as f32,s.w,0.7),Rgba(148,126,88,0.12))?; }
      p.fill(Rect::new(plan.inner.x-5.0,plan.inner.y,1.0,plan.inner.h),Rgba(185,84,70,0.4))?;
    }
    "pixel" => {
      for x in (12..(s.w as usize).saturating_sub(12)).step_by(8) { p.fill(Rect::new(x as f32,plan.inner.y-6.0,3.0,3.0),themed.primary.alpha(0.2))?; }
    }
    _ => {},
  }
  let (color,width) = match s.appearance.as_str() {
    "glass" => (Rgba(255,255,255,0.38),1.0), "futuristic" => (themed.primary.alpha(0.8),1.0),
    "cartoon" => (Rgba::hex(0x302b25),3.0), "paper" => (Rgba::hex(0xd1c2a3),1.0),
    "pixel" => (themed.primary,3.0), _ => (themed.on_layer0.alpha(if hover { 0.20 } else { 0.10 }),1.0),
  };
  let border = p.brush(color)?;
  unsafe { p.dc.DrawGeometry(geometry,&border,width,None); }
  if let Some(x) = plan.divider { for y in (8..(s.h as usize).saturating_sub(8)).step_by(8) { p.fill(Rect::new(x,y as f32,1.0,3.0),themed.on_layer0.alpha(0.35))?; } }
  if s.shape == "polaroid" {
    p.fill_round(plan.inner.inset(-3.0,-3.0),2.0,themed.on_layer0.alpha(0.05))?;
    p.stroke_round(plan.inner.inset(-3.0,-3.0),2.0,themed.on_layer0.alpha(0.14),1.0)?;
    p.fill(Rect::new(s.w*0.38,3.0,s.w*0.24,26.0),themed.primary.alpha(0.32))?;
  }
  Ok(())
}

/// Composite every content pixel (including album art and colour emoji)
/// once; changing brush alpha alone cannot fade those resources correctly.
fn with_opacity(p: &mut Painter, bounds: Rect, opacity: f32, draw: impl FnOnce(&mut Painter) -> anyhow::Result<()>) -> anyhow::Result<()> {
  let parameters = D2D1_LAYER_PARAMETERS1 {
    contentBounds: bounds.d2d(), opacity: opacity.clamp(0.0, 1.0),
    maskTransform: windows::Foundation::Numerics::Matrix3x2::identity(), ..Default::default()
  };
  unsafe { p.dc.PushLayer(&parameters, None::<&ID2D1Layer>); }
  let result = draw(p);
  unsafe { p.dc.PopLayer(); }
  result
}

fn material_theme(theme: &Theme, appearance: &str) -> Theme {
  let mut theme = *theme;
  match appearance {
    "paper" | "cartoon" => {
      theme.layer0 = Rgba::hex(if appearance == "paper" { 0xf5eedc } else { 0xffdf8f });
      theme.on_layer0 = Rgba::hex(0x302b25); theme.on_surface_variant = Rgba::hex(0x64584a);
      theme.primary = Rgba::hex(if appearance == "paper" { 0x956243 } else { 0xb84458 });
      theme.surface_container_high = Rgba::hex(0xe9d9b9);
      theme.surface_container = Rgba::hex(0xe9d9b9);
      theme.on_primary = Rgba::hex(0xfff8e8);
    }
    "futuristic" => {
      theme.layer0 = Rgba::hex(0x08151d); theme.on_layer0 = Rgba::hex(0xc4fbff);
      theme.primary = Rgba::hex(0x4aefdc); theme.on_surface_variant = Rgba::hex(0x85b8c8);
    }
    _ => {},
  }
  theme
}

fn material(p: &mut Painter, theme: &Theme, s: &Spec, card: Rect, hover: bool) -> anyhow::Result<()> {
  let themed = material_theme(theme, &s.appearance); let t = &themed;
  let edge = card.inset(1.0, 1.0);
  match s.appearance.as_str() {
    "transparent" => {},
    "outline" => {},
    "glass" => {
      p.fill_round(edge, RADIUS - 1.0, t.layer0.alpha(0.35))?;
      // Layered tint and highlights give the pane a reflective surface.
      for i in 0..12 {
        let inset = 2.0 + i as f32;
        p.stroke_round(card.inset(inset, inset), (RADIUS - inset).max(0.0), Rgba(255, 255, 255, (12 - i) as f32 * 0.006), 1.0)?;
      }
      p.fill_round(Rect::new(24.0, 9.0, (s.w - 48.0).max(1.0), 2.0), 1.0, Rgba(255, 255, 255, 0.45))?;
      p.stroke_round(edge, RADIUS - 1.0, Rgba(255, 255, 255, 0.38), 1.0)?;
    }
    "futuristic" => {
      p.fill_round(edge, 8.0, t.layer0.alpha(0.94))?;
      for x in (16..s.w as usize).step_by(24) { p.fill(Rect::new(x as f32, 10.0, 1.0, (s.h - 20.0).max(1.0)), t.primary.alpha(0.04))?; }
      for y in (16..s.h as usize).step_by(24) { p.fill(Rect::new(10.0, y as f32, (s.w - 20.0).max(1.0), 1.0), t.primary.alpha(0.04))?; }
      p.stroke_round(card.inset(3.0, 3.0), 7.0, t.primary.alpha(0.10), 5.0)?;
      p.stroke_round(edge, 8.0, t.primary.alpha(0.8), 1.0)?;
      for (x, y) in [(8.0, 8.0), (s.w - 30.0, 8.0), (8.0, s.h - 10.0), (s.w - 30.0, s.h - 10.0)] {
        p.fill(Rect::new(x, y, 22.0, 2.0), t.primary)?;
      }
    }
    "cartoon" => {
      p.fill_round(Rect::new(5.0, 6.0, s.w - 6.0, s.h - 7.0), 18.0, Rgba::hex(0x302b25))?;
      let face = Rect::new(1.0, 1.0, s.w - 7.0, s.h - 8.0);
      p.fill_round(face, 18.0, t.layer0)?;
      p.stroke_round(face, 18.0, Rgba::hex(0x302b25), 3.0)?;
      p.fill_round(Rect::new(20.0, 7.0, (s.w - 66.0).max(1.0), 3.0), 1.5, Rgba(255, 255, 255, 0.65))?;
    }
    "paper" => {
      p.fill_round(Rect::new(3.0, 4.0, s.w - 4.0, s.h - 5.0), 3.0, Rgba(0, 0, 0, 0.2))?;
      p.fill_round(card.inset(1.0, 1.0), 3.0, t.layer0)?;
      p.stroke_round(edge, 3.0, Rgba::hex(0xd1c2a3), 1.0)?;
      for y in (32..(s.h as usize).saturating_sub(8)).step_by(20) { p.fill(Rect::new(12.0, y as f32, s.w - 24.0, 0.7), Rgba(148, 126, 88, 0.12))?; }
      p.fill(Rect::new(10.0, 9.0, 1.0, s.h - 18.0), Rgba(185, 84, 70, 0.4))?;
    }
    "pixel" => {
      p.fill(Rect::new(4.0, 0.0, s.w - 8.0, s.h), t.layer0)?;
      p.fill(Rect::new(0.0, 4.0, s.w, s.h - 8.0), t.layer0)?;
      for (r, c) in [
        (Rect::new(4.0, 0.0, s.w - 8.0, 4.0), t.primary),
        (Rect::new(0.0, 4.0, 4.0, s.h - 8.0), t.primary),
        (Rect::new(4.0, s.h - 4.0, s.w - 8.0, 4.0), t.on_layer0.alpha(0.3)),
        (Rect::new(s.w - 4.0, 4.0, 4.0, s.h - 8.0), t.on_layer0.alpha(0.3)),
      ] { p.fill(r, c)?; }
      for x in (12..(s.w as usize).saturating_sub(12)).step_by(8) { p.fill(Rect::new(x as f32, 8.0, 3.0, 3.0), t.primary.alpha(0.2))?; }
    }
    _ => {
      p.fill_round(card, RADIUS, Rgba(0, 0, 0, 0.18))?;
      p.fill_round(edge, RADIUS - 1.0, t.layer0.alpha(0.97))?;
      p.stroke_round(edge, RADIUS - 1.0, t.on_layer0.alpha(if hover { 0.20 } else { 0.10 }), 1.0)?;
    }
  }
  if s.kind == Kind::Note && s.appearance != "transparent" { p.fill_round(Rect::new(8.0, 18.0, 3.0, s.h - 36.0), 1.5, t.primary.alpha(0.6))?; }
  Ok(())
}

fn fitted_clock_style(p: &mut Painter, time: &str, r: Rect, date_h: f32, large: bool) -> anyhow::Result<TextStyle> {
  let max = if large { r.h-date_h } else { (r.h-date_h)*0.8 };
  let mut style = TextStyle { size: max.max(12.0), weight: if large { 300.0 } else { 400.0 } };
  let width = (r.w-2.0).max(1.0);
  // Optical sizing changes the font's proportions when its size changes.
  // Re-measure at the final size instead of relying on a single linear ratio.
  for _ in 0..8 {
    let measured = p.measure_with(time,style,true)?;
    if measured <= width { break; }
    style.size *= width/measured*0.98;
  }
  Ok(style)
}

fn clock(p: &mut Painter, t: &Theme, s: &Spec, d: &Data, r: Rect) -> anyhow::Result<()> {
  let c = &d.clock;
  match s.clock {
    ClockStyle::Analog => {
      let size = r.w.min(r.h);
      let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
      let rad = size / 2.0;
      if !GLYPH_OUTLINE.with(|flag| flag.get()) { p.fill_circle(cx, cy, rad, t.surface_container)?; }
      for i in 0..60 {
        let a = i as f32 / 60.0 * std::f32::consts::TAU;
        let (sx, sy) = (a.sin(), -a.cos());
        let major = i % 5 == 0;
        let b = p.brush(t.on_layer0.alpha(if major { 0.7 } else { 0.20 }))?;
        unsafe {
          p.dc.DrawLine(pt(cx + sx * rad * if major { 0.77 } else { 0.84 }, cy + sy * rad * if major { 0.77 } else { 0.84 }),
            pt(cx + sx * rad * 0.90, cy + sy * rad * 0.90), &b, if major { 2.0 } else { 1.0 }, None);
        }
      }
      let hand = |p: &mut Painter, frac: f32, len: f32, width: f32, c: Rgba| -> anyhow::Result<()> {
        let a = frac * std::f32::consts::TAU;
        let b = p.brush(c)?;
        unsafe {
          p.dc.DrawLine(pt(cx, cy), pt(cx + a.sin() * rad * len, cy - a.cos() * rad * len), &b, width, None);
        }
        Ok(())
      };
      let (h, m, sec) = (c.h as f32, c.m as f32, c.s as f32);
      hand(p, ((h % 12.0) + m / 60.0) / 12.0, 0.5, 4.0, t.on_layer0)?;
      hand(p, (m + sec / 60.0) / 60.0, 0.72, 3.0, t.on_layer0)?;
      if s.seconds {
        hand(p, sec / 60.0, 0.8, 1.5, t.primary)?;
      }
      p.fill_circle(cx, cy, 3.5, t.primary)?;
    }
    style => {
      let large = style == ClockStyle::Large;
      let date_h = if s.date { 20.0 } else { 0.0 };
      let style_t = fitted_clock_style(p,&c.time,r,date_h,large)?;
      let size = style_t.size;
      let block = size * 1.25 + date_h;
      let top = r.y + (r.h - block) / 2.0;
      glyph_text(p, &c.time, Rect::new(r.x, top, r.w, size * 1.25), style_t, t.on_layer0, Align::Center, true)?;
      if s.date {
        glyph_text(p, &c.date, Rect::new(r.x, top + size * 1.25, r.w, date_h), SMALL, t.on_surface_variant, Align::Center, false)?;
      }
    }
  }
  if s.clock == ClockStyle::Analog && s.date && r.h > r.w + 24.0 {
    glyph_text(p, &c.date, Rect::new(r.x, r.bottom() - 18.0, r.w, 18.0), SMALL, t.on_surface_variant, Align::Center, false)?;
  }
  Ok(())
}

fn clock_shaped(p: &mut Painter,t: &Theme,s: &Spec,d: &Data,plan: &shape::Plan) -> anyhow::Result<()> {
  let mut shown = s.clone(); if plan.caption.is_some() { shown.date = false; }
  if let Some(lead) = plan.lead {
    let mut dial = s.clone(); dial.clock = ClockStyle::Analog; dial.date = false;
    clock(p,t,&dial,d,lead)?;
    if shown.clock == ClockStyle::Analog { shown.clock = ClockStyle::Digital; }
  }
  if s.shape == "circle" && s.clock != ClockStyle::Analog {
    let (cx,cy,rad) = (s.w/2.0,s.h/2.0,s.w.min(s.h)*0.435);
    let brush = p.brush(t.primary.alpha(0.5))?;
    for i in 0..60 { let a = i as f32*std::f32::consts::TAU/60.0; let len = if i%5 == 0 { 7.0 } else { 3.0 };
      unsafe { p.dc.DrawLine(pt(cx+(rad-len)*a.sin(),cy-(rad-len)*a.cos()),pt(cx+rad*a.sin(),cy-rad*a.cos()),&brush,1.0,None); }
    }
  }
  clock(p,t,&shown,d,plan.inner)
}

fn button(p: &mut Painter, t: &Theme, r: Rect, icon: &str, primary: bool) -> anyhow::Result<()> {
  if primary && !GLYPH_OUTLINE.with(|flag| flag.get()) {
    p.fill_circle(r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, t.primary)?;
  }
  glyph_icon(p, icon, r.x + r.w / 2.0, r.y + r.h / 2.0, 22.0, true, if primary { t.on_primary } else { t.on_layer0 })
}

fn media(p: &mut Painter, t: &Theme, d: &Data, r: Rect, plan: &shape::Plan, hits: &mut Vec<(Rect, Hit)>) -> anyhow::Result<()> {
  let Some(m) = &d.media else {
    glyph_icon(p, "music_off", r.x + 18.0, r.y + r.h / 2.0, 26.0, false, t.on_surface_variant)?;
    glyph_text(p, &(d.tr)("Çalan bir şey yok"), Rect::new(r.x + 42.0, r.y, r.w - 42.0, r.h), BODY, t.on_surface_variant, Align::Left, false)?;
    return Ok(());
  };
  let (cover,details) = if let Some(lead) = plan.lead {
    let side = lead.w.min(lead.h).min(100.0);
    (Rect::new(lead.x+(lead.w-side)/2.0,lead.y+(lead.h-side)/2.0,side,side),r)
  } else if plan.vertical {
    let side = (r.h-84.0).clamp(38.0,88.0).min(r.w*0.6);
    (Rect::new(r.x+(r.w-side)/2.0,r.y,side,side),Rect::new(r.x,r.y+side+8.0,r.w,r.h-side-8.0))
  } else {
    let side = r.h.min(r.w * 0.32); let cover = Rect::new(r.x,r.y,side,side);
    (cover,Rect::new(cover.right()+14.0,r.y,(r.right()-cover.right()-14.0).max(10.0),r.h))
  };
  let side = cover.w;
  match m.art {
    Some(bmp) => {
      let size = unsafe { bmp.GetSize() };
      let img: ID2D1Image = bmp.cast()?;
      p.image_round(&img, size.width, size.height, cover, 16.0, 1.0)?;
    }
    None => {
      if !GLYPH_OUTLINE.with(|flag| flag.get()) { p.fill_round(cover, 16.0, t.primary_container)?; }
      glyph_icon(p, "music_note", cover.x + side / 2.0, cover.y + side / 2.0, side * 0.4, false, t.on_primary_container)?;
    }
  }
  let (x,w) = (details.x,details.w); let r = details;
  let align = if plan.vertical { Align::Center } else { Align::Left };
  glyph_text(p, &m.title, Rect::new(x, r.y, w, 20.0), TextStyle { size: 15.0, weight: 600.0 }, t.on_layer0, align, false)?;
  if plan.caption.is_none() { glyph_text(p, &m.artist, Rect::new(x, r.y + 20.0, w, 18.0), SMALL, t.on_surface_variant, align, false)?; }
  let btn = 32.0;
  let by = r.bottom() - btn;
  if let Some(f) = m.progress.filter(|_| r.h >= 82.0) {
    let bar = Rect::new(x, by - 10.0, w, 3.0);
    p.fill_round(bar, 2.0, t.on_layer0.alpha(0.12))?;
    p.fill_round(Rect::new(bar.x, bar.y, bar.w * f.clamp(0.0, 1.0), bar.h), 2.0, t.primary)?;
  }
  let row = btn * 3.0 + 16.0;
  let bx = x + (w - row).max(0.0) / 2.0;
  let prev = Rect::new(bx, by, btn, btn);
  let play = Rect::new(bx + btn + 8.0, by, btn, btn);
  let next = Rect::new(bx + 2.0 * (btn + 8.0), by, btn, btn);
  button(p, t, prev, "skip_previous", false)?;
  button(p, t, play, if m.playing { "pause" } else { "play_arrow" }, true)?;
  button(p, t, next, "skip_next", false)?;
  hits.extend([(prev, Hit::Prev), (play, Hit::Play), (next, Hit::Next)]);
  Ok(())
}

fn system(p: &mut Painter, t: &Theme, s: &Spec, d: &Data, r: Rect) -> anyhow::Result<()> {
  let sys = d.system;
  let mut gauges: Vec<(&str, Option<f32>, Option<f32>)> = vec![("CPU", sys.cpu, if s.temps { sys.cpu_temp } else { None }), ("RAM", sys.ram, None)];
  if sys.gpu.is_some() || (s.temps && sys.gpu_temp.is_some()) {
    gauges.push(("GPU", sys.gpu, if s.temps { sys.gpu_temp } else { None }));
  }
  let row_h = (r.h / gauges.len() as f32).min(38.0);
  let top = r.y + (r.h - row_h * gauges.len() as f32) / 2.0;
  for (i, (label, value, temp)) in gauges.iter().enumerate() {
    if s.shape == "capsule" {
      let cell = r.w/gauges.len() as f32; let x = r.x+i as f32*cell;
      let pct = value.map_or("–".into(),|v| format!("{}%",v.round() as i32));
      glyph_text(p,label,Rect::new(x,r.y,cell,18.0),SMALL,t.on_surface_variant,Align::Center,false)?;
      glyph_text(p,&pct,Rect::new(x,r.y+19.0,cell,26.0),TextStyle{size:21.0,weight:500.0},t.on_layer0,Align::Center,true)?;
      if let Some(temp) = temp { glyph_text(p,&format!("{}°",temp.round() as i32),Rect::new(x,r.y+47.0,cell,18.0),SMALL,t.primary,Align::Center,true)?; }
      let track = Rect::new(x+5.0,r.bottom()-4.0,cell-10.0,3.0); p.fill(track,t.on_layer0.alpha(0.12))?;
      p.fill(Rect::new(track.x,track.y,track.w*value.unwrap_or(0.0).clamp(0.0,100.0)/100.0,3.0),t.primary)?;
      continue;
    }
    let y = top + row_h * i as f32;
    let frac = value.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0;
    let pct = value.map_or("–".to_string(), |v| format!("{}%", v.round() as i32));
    glyph_text(p, label, Rect::new(r.x, y, 40.0, 18.0), SMALL, t.on_layer0, Align::Left, false)?;
    if let Some(c) = temp {
      glyph_text(p, &format!("{}°", c.round() as i32), Rect::new(r.x + 42.0, y, 40.0, 18.0), SMALL, t.on_surface_variant, Align::Left, true)?;
    }
    glyph_text(p, &pct, Rect::new(r.right() - 48.0, y, 48.0, 18.0), TextStyle { size: 13.0, weight: 600.0 }, t.on_layer0, Align::Center, true)?;
    let track = Rect::new(r.x, y + row_h - 6.0, r.w, 4.0);
    p.fill_round(track, 2.0, t.on_layer0.alpha(0.10))?;
    if value.is_some() && frac > 0.0 {
      p.fill_round(Rect::new(track.x, track.y, track.w * frac, track.h), 2.0, t.primary)?;
    }
  }
  Ok(())
}

fn weather_card(p: &mut Painter, t: &Theme, s: &Spec, d: &Data, r: Rect, plan: &shape::Plan) -> anyhow::Result<()> {
  let (w,h)=shape::preferred(s.kind,&s.shape);
  let (_,_,w,h)=shape::clamp(s.kind,&s.shape,(0.0,0.0,w,h),10000.0,10000.0);
  let zoom=(s.w/w).min(s.h/h).clamp(1.0,1.75);
  let scaled=|r:Rect| Rect::new(r.x/zoom,r.y/zoom,r.w/zoom,r.h/zoom);
  let mut logical=plan.clone(); logical.inner=scaled(plan.inner);
  logical.lead=plan.lead.map(scaled); logical.caption=plan.caption.map(scaled);
  let mut previous=windows::Foundation::Numerics::Matrix3x2::identity();
  unsafe { p.dc.GetTransform(&mut previous); }
  let mut transform=previous;
  transform.M11*=zoom; transform.M12*=zoom; transform.M21*=zoom; transform.M22*=zoom;
  unsafe { p.dc.SetTransform(&transform); }
  let result=weather_content(p,t,d,scaled(r),&logical);
  unsafe { p.dc.SetTransform(&previous); }
  result
}

fn weather_content(p: &mut Painter, t: &Theme, d: &Data, r: Rect, plan: &shape::Plan) -> anyhow::Result<()> {
  match d.weather {
    Some(Ok(w)) => {
      let (icon, text) = weather::describe(w.code, w.day);
      if plan.vertical || plan.lead.is_some() {
        let line = if w.high.is_finite() && w.low.is_finite() { format!("{} · ↑{}° ↓{}°",(d.tr)(text),w.high.round() as i32,w.low.round() as i32) } else { (d.tr)(text) };
        let unit = if w.fahrenheit { "°F" } else { "°" }; let temp = format!("{}{unit}",w.temp.round() as i32);
        if let Some(lead) = plan.lead {
          self::glyph_icon(p,icon,lead.x+lead.w/2.0,lead.y+lead.h/2.0,lead.w.min(lead.h).min(72.0),true,t.primary)?;
          let big = (r.h-48.0).clamp(26.0,52.0);
          let y = r.y+(r.h-big-48.0).max(0.0)/2.0;
          self::glyph_text(p,&temp,Rect::new(r.x,y,r.w,big+4.0),TextStyle{size:big,weight:400.0},t.on_layer0,Align::Center,true)?;
          self::glyph_text(p,&line,Rect::new(r.x,y+big+8.0,r.w,20.0),SMALL,t.on_layer0,Align::Center,false)?;
          self::glyph_text(p,&w.place,Rect::new(r.x,y+big+28.0,r.w,20.0),SMALL,t.on_surface_variant,Align::Center,false)?;
        } else {
          let footer = if plan.caption.is_some() { 20.0 } else { 40.0 };
          let big = ((r.h-footer-8.0)/2.0).clamp(24.0,44.0); let y = r.y+(r.h-2.0*big-footer-8.0).max(0.0)/2.0;
          self::glyph_icon(p,icon,r.x+r.w/2.0,y+big/2.0,big,true,t.primary)?;
          self::glyph_text(p,&temp,Rect::new(r.x,y+big+2.0,r.w,big+4.0),TextStyle{size:big,weight:400.0},t.on_layer0,Align::Center,true)?;
          self::glyph_text(p,&line,Rect::new(r.x,y+big*2.0+8.0,r.w,20.0),SMALL,t.on_layer0,Align::Center,false)?;
          if plan.caption.is_none() { self::glyph_text(p,&w.place,Rect::new(r.x,y+big*2.0+28.0,r.w,20.0),SMALL,t.on_surface_variant,Align::Center,false)?; }
        }
        return Ok(());
      }
      // Keep icon, temperature and caption together even after a deliberate
      // resize. Pinning them to opposite edges created an empty giant card.
      let h = r.h.min(104.0); let width = r.w.min(320.0);
      let r = Rect::new(r.x+(r.w-width)/2.0,r.y+(r.h-h)/2.0,width,h);
      let big = (r.h - 44.0).clamp(24.0, 52.0);
      glyph_icon(p, icon, r.x + big / 2.0, r.y + big / 2.0 + 4.0, big, true, t.primary)?;
      let unit = if w.fahrenheit { "°F" } else { "°" };
      let temp = format!("{}{unit}", w.temp.round() as i32);
      glyph_text(p, &temp, Rect::new(r.x + big + 12.0, r.y, r.w - big - 12.0, big + 4.0), TextStyle { size: big * 0.8, weight: 400.0 }, t.on_layer0, Align::Left, true)?;
      let line = if w.high.is_finite() && w.low.is_finite() {
        format!("{} · ↑{}° ↓{}°", (d.tr)(text), w.high.round() as i32, w.low.round() as i32)
      } else {
        (d.tr)(text)
      };
      glyph_text(p, &line, Rect::new(r.x, r.bottom() - 40.0, r.w, 20.0), BODY, t.on_layer0, Align::Left, false)?;
      glyph_text(p, &w.place, Rect::new(r.x, r.bottom() - 20.0, r.w, 20.0), SMALL, t.on_surface_variant, Align::Left, false)?;
    }
    Some(Err(_)) => {
      glyph_icon(p, "cloud_off", r.x + 16.0, r.y + r.h / 2.0, 26.0, false, t.on_surface_variant)?;
      glyph_text(p, &(d.tr)("Hava durumu alınamadı"), Rect::new(r.x + 40.0, r.y, r.w - 40.0, r.h), BODY, t.on_surface_variant, Align::Left, false)?;
    }
    None => {
      glyph_icon(p, "partly_cloudy_day", r.x + 16.0, r.y + r.h / 2.0, 26.0, false, t.on_surface_variant)?;
      glyph_text(p, &(d.tr)("Yükleniyor…"), Rect::new(r.x + 40.0, r.y, r.w - 40.0, r.h), BODY, t.on_surface_variant, Align::Left, false)?;
    }
  }
  Ok(())
}

fn agenda(p: &mut Painter, t: &Theme, d: &Data, r: Rect, plan: &shape::Plan) -> anyhow::Result<()> {
  let badge = plan.lead.map(|lead| Rect::new(lead.x+(lead.w-52.0)/2.0,lead.y+(lead.h-52.0)/2.0,52.0,52.0)).unwrap_or(Rect::new(r.x,r.y,52.0,52.0));
  if !GLYPH_OUTLINE.with(|flag| flag.get()) { p.fill_round(badge, 16.0, t.primary_container)?; }
  glyph_text(p, &d.day_big, badge, TextStyle { size: 32.0, weight: 500.0 }, t.on_primary_container, Align::Center, true)?;
  let offset = if plan.lead.is_some() { 0.0 } else { 66.0 };
  glyph_text(p, &d.day_line, Rect::new(r.x + offset, r.y + 6.0, r.w - offset, 40.0), BODY, t.on_layer0, Align::Left, false)?;
  let header = if plan.lead.is_some() { 52.0 } else { 66.0 };
  p.fill(Rect::new(r.x, r.y + header, r.w, 1.0), t.on_layer0.alpha(0.10))?;
  let mut y = r.y + header+12.0;
  if d.todos.is_empty() {
    glyph_text(p, &(d.tr)("Yapılacak yok"), Rect::new(r.x, y, r.w, 20.0), SMALL, t.on_surface_variant, Align::Left, false)?;
    return Ok(());
  }
  for todo in d.todos {
    if y + 22.0 > r.bottom() {
      break;
    }
    glyph_icon(p, "radio_button_unchecked", r.x + 8.0, y + 11.0, 14.0, false, t.primary)?;
    glyph_text(p, todo, Rect::new(r.x + 26.0, y, r.w - 26.0, 22.0), BODY, t.on_layer0, Align::Left, false)?;
    y += 30.0;
  }
  Ok(())
}

#[allow(clippy::too_many_arguments)]
fn note(
  p: &mut Painter,
  t: &Theme,
  s: &Spec,
  d: &Data,
  r: Rect,
  editing: Option<&Editing>,
  note_layout: &mut Option<IDWriteTextLayout>,
  hits: &mut Vec<(Rect, Hit)>,
) -> anyhow::Result<()> {
  hits.push((r, Hit::Text));
  let text = editing.map_or(s.note.as_str(), |e| e.text);
  if text.is_empty() && editing.is_none() {
    glyph_text(p, &(d.tr)("Not yazmak için tıkla"), Rect::new(r.x, r.y, r.w, 20.0), BODY, t.on_surface_variant, Align::Left, false)?;
    *note_layout = None;
    return Ok(());
  }
  let layout = wrapped(p, text, r)?;
  unsafe {
    p.dc.PushAxisAlignedClip(&r.d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED);
  }
  if let Some(e) = editing {
    let (a, b) = e.selection;
    if a != b {
      let (ua, ub) = (utf16_at(text, a), utf16_at(text, b));
      let mut runs = [DWRITE_HIT_TEST_METRICS::default(); 32];
      let mut count = 0u32;
      if unsafe { layout.HitTestTextRange(ua, ub - ua, 0.0, 0.0, Some(&mut runs), &mut count) }.is_ok() {
        for m in runs.iter().take(count as usize) {
          p.fill(Rect::new(r.x + m.left, r.y + m.top, m.width, m.height), t.primary.alpha(0.35))?;
        }
      }
    }
  }
  let brush = p.brush(t.on_layer0)?;
  if GLYPH_OUTLINE.with(|flag| flag.get()) {
    let halo = p.brush(opposing(t.on_layer0))?;
    for (dx,dy) in OUTLINE_OFFSETS { unsafe { p.dc.DrawTextLayout(pt(r.x+dx,r.y+dy),&layout,&halo,D2D1_DRAW_TEXT_OPTIONS_NONE); } }
  }
  unsafe {
    p.dc.DrawTextLayout(pt(r.x, r.y), &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
  }
  if let Some(e) = editing {
    let (mut x, mut y) = (0f32, 0f32);
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    unsafe {
      let _ = layout.HitTestTextPosition(utf16_at(text, e.caret), false, &mut x, &mut y, &mut m);
    }
    let h = if m.height > 0.0 { m.height } else { BODY.size * 1.3 };
    if GLYPH_OUTLINE.with(|flag| flag.get()) { p.fill(Rect::new((r.x+x).round()-0.8,r.y+y-0.8,3.1,h+1.6),opposing(t.primary))?; }
    p.fill(Rect::new((r.x + x).round(), r.y + y, 1.5, h), t.primary)?;
  }
  unsafe { p.dc.PopAxisAlignedClip() };
  *note_layout = Some(layout);
  Ok(())
}

/// The note's text wrapped to `r` (also used to find where a click falls).
pub fn wrapped(p: &mut Painter, text: &str, r: Rect) -> anyhow::Result<IDWriteTextLayout> {
  use windows::Win32::Graphics::DirectWrite::{DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_WORD_WRAPPING_WRAP};
  let layout = p.layout(text, BODY, r.w, r.h, false)?;
  unsafe {
    layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
    layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
  }
  Ok(layout)
}

/// The UTF-16 position of the `i`-th character.
pub fn utf16_at(text: &str, i: usize) -> u32 {
  text.chars().take(i).map(char::len_utf16).sum::<usize>() as u32
}

/// The character index of a UTF-16 position.
pub fn char_at_utf16(text: &str, pos: u32) -> usize {
  let mut n = 0u32;
  for (i, c) in text.chars().enumerate() {
    if n >= pos {
      return i;
    }
    n += c.len_utf16() as u32;
  }
  text.chars().count()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  #[ignore = "offscreen Direct2D visual check; requires the installed font assets"]
  fn render_widget_gallery_without_showing_windows() -> anyhow::Result<()> {
    use super::super::super::{fonts::Fonts, gfx::Gfx, icons::Icons, view::{Res, DARK, LIGHT}};
    use windows::{Foundation::Numerics::Matrix3x2, Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED}};
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?; }
    struct Com;
    impl Drop for Com { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
    let _com = Com;
    let gfx = Gfx::new()?;
    let pack = std::path::PathBuf::from(std::env::var_os("LL_WIDGET_TEST_FONTS")
      .unwrap_or_else(|| r"C:\Program Files\LogicalLunge\ui\logical-lunge".into()));
    let mut fonts = Fonts::load(&gfx.dwrite, &pack)?;
    // Real DirectWrite collections and cache: changing one widget's face must
    // never leak into the next widget, menus or Material Symbols icons.
    let family = |format: &windows::Win32::Graphics::DirectWrite::IDWriteTextFormat| -> anyhow::Result<String> {
      let mut name = vec![0u16; unsafe { format.GetFontFamilyNameLength() } as usize+1];
      unsafe { format.GetFontFamilyName(&mut name)?; }
      Ok(String::from_utf16_lossy(&name[..name.len()-1]))
    };
    let normal = fonts.text(BODY)?; let icon_format = fonts.icon(24.0,false)?;
    for (style, expected) in [("pixel","Pixelify Sans"),("cartoon","Comic Sans MS"),("paper","Georgia"),("futuristic","Consolas")] {
      with_widget_font(style, || -> anyhow::Result<()> {
        let selected = fonts.text(BODY)?;
        assert_eq!(family(&selected)?,expected);
        assert_ne!(selected.as_raw(),normal.as_raw());
        assert_eq!(fonts.icon(24.0,false)?.as_raw(),icon_format.as_raw());
        let collection = unsafe { selected.GetFontCollection()? };
        let mut index = 0; let mut exists = windows::Win32::Foundation::BOOL(0);
        unsafe { collection.FindFamilyName(&windows::core::HSTRING::from(expected),&mut index,&mut exists)?; }
        assert!(exists.as_bool(),"{expected} did not resolve to an actual font");
        Ok(())
      })?;
      assert_eq!(fonts.text(BODY)?.as_raw(),normal.as_raw());
    }
    let mut res = Res::new(&gfx)?;
    let mut icons = Icons::default();
    let out = std::env::temp_dir().join("ll-widget-gallery");
    std::fs::create_dir_all(&out)?;
    let report = Ok(Report { place: "İstanbul".into(), temp: 23.0, high: 25.0, low: 17.0, code: 2, day: true, fahrenheit: false });
    let tr = |s: &str| s.to_owned();
    let d = Data {
      clock: Clock { h: 13, m: 24, s: 36, time: "13:24".into(), date: "3 Ekim Cumartesi".into() },
      media: Some(Media { title: "Açık pencereler".into(), artist: "Logical Lunge".into(), playing: true, progress: Some(0.42), art: None }),
      system: System { cpu: Some(32.0), ram: Some(64.0), gpu: Some(12.0), cpu_temp: Some(51.0), gpu_temp: Some(42.0) },
      weather: Some(&report), day_big: "3".into(), day_line: "Ekim\nCumartesi".into(),
      todos: &["Yerel sürümü dene".into(), "Notları düzenle".into()], tr: &tr,
    };
    for (name, theme) in [("dark", DARK), ("light", LIGHT)] {
      for small in [false, true] {
        let path = out.join(format!("{name}-{}.png", if small { "min" } else { "default" }));
        let mut requests = Vec::new();
        let mut paint_error = None;
        gfx.snapshot(760, 720, 1.0, &path, |dc| {
          let mut p = Painter { dc, gfx: &gfx, fonts: &mut fonts, res: &mut res, icons: &mut icons, requests: &mut requests };
          p.fill(Rect::new(0.0, 0.0, 760.0, 720.0), Rgba::hex(0x698778))?;
          // Light, dark and detailed wallpaper behind every card.
          for x in 0..19 {
            p.fill(Rect::new(x as f32 * 40.0, 0.0, 20.0, 720.0), if x % 2 == 0 { Rgba::hex(0xe7dabb) } else { Rgba::hex(0x273e4b) })?;
          }
          for (i, kind) in super::super::layout::KINDS.into_iter().enumerate() {
            let mut spec = Spec::new(i as u64 + 1, kind, "");
            if small { (spec.w, spec.h) = kind.min_size(); }
            spec.note = "Bugün\nÖnce küçük işleri bitir.\n\nBir fikir: sakin bir masaüstü.".into();
            let x = 24.0 + (i % 2) as f32 * 380.0;
            let y = 24.0 + (i / 2) as f32 * 208.0;
            unsafe { dc.SetTransform(&Matrix3x2::translation(x, y)); }
            let mut note_layout = None;
            match paint(&mut p, &theme, &spec, &d, true, None, &mut note_layout) {
              Ok(hits) => assert!(hits.iter().all(|(r, _)| r.x >= 0.0 && r.y >= 0.0 && r.right() <= spec.w && r.bottom() <= spec.h), "hit area outside {:?}", kind),
              Err(err) => { paint_error = Some(err); break; }
            }
          }
          Ok(())
        })?;
        if let Some(err) = paint_error { return Err(err); }
        println!("{}", path.display());
      }
    }
    // Appearance gallery and opacity checks use offscreen surfaces only.
    let path = out.join("appearances.png");
    let mut requests = Vec::new(); let mut paint_error = None;
    gfx.snapshot(760, 720, 1.0, &path, |dc| {
      let mut p = Painter { dc, gfx: &gfx, fonts: &mut fonts, res: &mut res, icons: &mut icons, requests: &mut requests };
      p.fill(Rect::new(0.0, 0.0, 760.0, 720.0), Rgba::hex(0x698778))?;
      for (i, (appearance, _)) in super::super::layout::APPEARANCES.iter().enumerate() {
        let x = 24.0 + (i % 2) as f32 * 380.0; let y = 12.0 + (i / 2) as f32 * 178.0;
        unsafe { dc.SetTransform(&Matrix3x2::translation(x, y)); }
        let mut spec = Spec::new(i as u64 + 1, Kind::Weather, ""); spec.w = 320.0; spec.appearance = (*appearance).into();
        let mut layout = None;
        if let Err(error) = paint(&mut p, &DARK, &spec, &d, false, None, &mut layout) { paint_error = Some(error); break; }
        glyph_text(&mut p, appearance, Rect::new(0.0, 138.0, 320.0, 20.0), BODY, Rgba::hex(0xffffff), Align::Left, false)
          .map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), e.to_string()))?;
      }
      Ok(())
    })?;
    if let Some(error) = paint_error { return Err(error); }
    println!("{}", path.display());

    for appearance in ["pixel","cartoon","paper","futuristic"] {
      let path = out.join(format!("fonts-{appearance}-all-kinds.png")); let mut requests = Vec::new();
      gfx.snapshot(1120,660,1.0,&path,|dc| {
        let mut p = Painter { dc,gfx:&gfx,fonts:&mut fonts,res:&mut res,icons:&mut icons,requests:&mut requests };
        p.fill(Rect::new(0.0,0.0,1120.0,660.0),Rgba::hex(0x698778))?;
        for (i,kind) in super::super::layout::KINDS.into_iter().enumerate() {
          let mut spec = Spec::new(i as u64+1,kind,""); spec.appearance = appearance.into();
          spec.note = "Çığ, öğle, şüphe, İstanbul 😀\nBir fikir: sakin bir masaüstü.".into();
          let editing = Editing { text:&spec.note,caret:5,selection:(2,5) };
          let x = 16.0+(i%3) as f32*370.0; let y = 20.0+(i/3) as f32*330.0;
          unsafe { dc.SetTransform(&Matrix3x2::translation(x,y)); }
          let mut note_layout = None;
          paint(&mut p,&DARK,&spec,&d,false,(kind == Kind::Note).then_some(&editing),&mut note_layout)
            .map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?;
          if let Some(layout) = note_layout {
            let mut name = vec![0u16;64];
            unsafe { layout.GetFontFamilyName(0,&mut name,None)?; }
            let end = name.iter().position(|c| *c == 0).unwrap();
            let expected = match appearance { "pixel" => "Pixelify Sans", "cartoon" => "Comic Sans MS", "paper" => "Georgia", _ => "Consolas" };
            assert_eq!(String::from_utf16_lossy(&name[..end]),expected,"editable note uses the appearance font");
            let mut cx=0.0; let mut cy=0.0; let mut hit=DWRITE_HIT_TEST_METRICS::default();
            unsafe { layout.HitTestTextPosition(utf16_at(&spec.note,5),false,&mut cx,&mut cy,&mut hit)?; }
            assert!(cx.is_finite() && cy.is_finite() && hit.height > 0.0);
          }
        }
        Ok(())
      })?;
      println!("{}",path.display());
    }

    for form in ["card","ticket","bubble"] {
      let mut spec = Spec::new(1,Kind::Weather,""); spec.shape=form.into(); spec.appearance="transparent".into();
      spec.w=545.0; spec.h=287.0;
      let path=out.join(format!("weather-oversized-{form}.png")); let mut requests=Vec::new();
      gfx.snapshot(545,287,1.0,&path,|dc| {
        let mut p=Painter { dc,gfx:&gfx,fonts:&mut fonts,res:&mut res,icons:&mut icons,requests:&mut requests };
        paint(&mut p,&DARK,&spec,&d,false,None,&mut None)
          .map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?;
        Ok(())
      })?;
      let pixels=read_pixels(&gfx,&path,545,287)?;
      let rows:Vec<_>=(0..287).filter(|y| pixels[y*545*4..(y+1)*545*4].chunks_exact(4).any(|p| p[3]>32)).collect();
      assert!(!rows.is_empty());
      let span=rows.last().unwrap()-rows[0];
      assert!(span>140 && span<240,"{form}: weather content must grow proportionately and stay grouped; span={span}");
    }

    let art = gfx.bitmap(&std::fs::read(out.join("dark-default.png"))?)?;
    let mut data = d;
    data.media = Some(Media { title: "Bitmap & emoji 😀".into(), artist: "Independent opacity".into(), playing: true, progress: Some(0.4), art: Some(&art) });
    let mut pixels = Vec::new();
    for (i, (background, content)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.0, 0.5)].into_iter().enumerate() {
      let mut spec = Spec::new(1, Kind::Media, ""); spec.background_opacity = background; spec.content_opacity = content;
      let path = out.join(format!("opacity-{i}.png")); let mut requests = Vec::new(); let mut error = None;
      gfx.snapshot(spec.w as u32, spec.h as u32, 1.0, &path, |dc| {
        let mut p = Painter { dc, gfx: &gfx, fonts: &mut fonts, res: &mut res, icons: &mut icons, requests: &mut requests };
        match paint(&mut p, &DARK, &spec, &data, false, None, &mut None) { Ok(_) => {}, Err(e) => error = Some(e) }
        Ok(())
      })?;
      if let Some(error) = error { return Err(error); }
      unsafe {
        use windows::Win32::{Foundation::GENERIC_READ, Graphics::Imaging::*};
        let decoder = gfx.wic.CreateDecoderFromFilename(&windows::core::HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        let converter = gfx.wic.CreateFormatConverter()?;
        converter.Initialize(&decoder.GetFrame(0)?, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
        let mut bytes = vec![0; spec.w as usize * spec.h as usize * 4];
        converter.CopyPixels(std::ptr::null(), spec.w as u32 * 4, &mut bytes)?;
        pixels.push(bytes);
      }
    }
    let alpha = |image: usize, x: usize, y: usize| pixels[image][(y * 336 + x) * 4 + 3];
    assert_eq!(alpha(0, 5, 50), 0, "zero background/content leaves clear pixels");
    assert!(alpha(1, 5, 50) > 200, "background remains when content is zero");
    assert_eq!(alpha(2, 5, 50), 0, "content does not turn on the card background");
    assert!(alpha(2, 40, 40) > 200, "album art remains when background is zero");
    assert_eq!(alpha(1, 40, 40), alpha(1, 100, 40), "zero content hides album art");
    assert!(alpha(4, 40, 40).abs_diff(alpha(2, 40, 40) / 2) <= 2, "content opacity composites bitmap alpha");
    let mut invisible = Spec::new(1, Kind::Media, ""); invisible.background_opacity = 0.0; invisible.content_opacity = 0.0;
    let mut requests = Vec::new(); let mut hits = Vec::new();
    gfx.snapshot(336, 112, 1.0, &out.join("zero-hover.png"), |dc| {
      let mut p = Painter { dc, gfx: &gfx, fonts: &mut fonts, res: &mut res, icons: &mut icons, requests: &mut requests };
      hits = paint(&mut p, &DARK, &invisible, &data, true, None, &mut None)
        .map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32), e.to_string()))?;
      Ok(())
    })?;
    assert!(hits.iter().any(|(_, hit)| *hit == Hit::Settings));
    // All 48 cells below use the production renderers and one standard
    // material, so their silhouettes and layout can be compared directly.
    for kind in super::super::layout::KINDS {
      let path = out.join(format!("shapes-{}.png",kind.id()));
      let mut requests = Vec::new(); let mut error = None;
      gfx.snapshot(1640,700,1.0,&path,|dc| {
        let mut p = Painter { dc,gfx: &gfx,fonts: &mut fonts,res: &mut res,icons: &mut icons,requests: &mut requests };
        p.fill(Rect::new(0.0,0.0,1640.0,700.0),Rgba::hex(0x72887e))?;
        glyph_text(&mut p,&format!("{} · same standard material / actual native renderer",kind.id()),Rect::new(20.0,8.0,1600.0,28.0),BODY,Rgba::hex(0xffffff),Align::Left,false)
          .map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?;
        for (i,(form,_)) in shape::SHAPES.iter().enumerate() {
          let x = 20.0+(i%4) as f32*410.0; let y = 50.0+(i/4) as f32*325.0;
          let mut spec = Spec::new(1,kind,""); spec.shape = (*form).into(); spec.note = "Bugün\nSüt ve ekmek al.\n\nBir fikir: sakin masaüstü. 😀".into();
          (_,_,spec.w,spec.h) = shape::clamp(kind,form,spec.rect(),380.0,268.0);
          let zoom = (380.0/spec.w).min(268.0/spec.h).min(1.0);
          unsafe { dc.SetTransform(&Matrix3x2 { M11: zoom, M12: 0.0, M21: 0.0, M22: zoom,
            M31: x+(380.0-spec.w*zoom)/2.0, M32: y+(268.0-spec.h*zoom)/2.0 }); }
          let editing = Editing { text: &spec.note,caret: 7,selection: (6,9) };
          if kind == Kind::Clock {
            let inner = shape::plan(form,spec.w,spec.h).inner;
            for time in ["13:24","23:59:59","11:59 PM"] { for large in [false,true] {
              let style = fitted_clock_style(&mut p,time,inner,20.0,large).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?;
              let width = p.measure_with(time,style,true).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?;
              assert!(width <= inner.w-1.0,"{form}: clock ellipsized {time} at {width}/{}",inner.w);
            } }
          }
          match paint(&mut p,&DARK,&spec,&data,false,(kind == Kind::Note).then_some(&editing),&mut None) {
            Ok(hits) => { let plan = shape::plan(form,spec.w,spec.h); assert!(hits.iter().all(|(r,_)| plan.contains_rect(*r)),"{kind:?}/{form} hits outside silhouette: {hits:?}"); },
            Err(e) => { error = Some(e); break; }
          }
          unsafe { dc.SetTransform(&Matrix3x2::identity()); }
          glyph_text(&mut p,form,Rect::new(x,y+278.0,380.0,22.0),BODY,Rgba::hex(0xffffff),Align::Center,false)
            .map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?;
        }
        Ok(())
      })?;
      if let Some(error) = error { return Err(error); }
      println!("{}",path.display());
    }
    // The actual path mask, rather than an inset rectangle, clips pixels.
    for form in ["circle","ticket","bubble","hexagon","polaroid","split"] {
      let mut spec = Spec::new(1,Kind::Note,""); spec.shape = form.into();
      (_,_,spec.w,spec.h) = shape::clamp(spec.kind,form,spec.rect(),380.0,268.0);
      spec.note = "A long note across the mask. 😀\nThe caret and selection stay in the safe content area.".repeat(4);
      let editing = Editing { text: &spec.note,caret: 7,selection: (2,12) };
      let path = out.join(format!("mask-{form}.png")); let mut requests = Vec::new();
      gfx.snapshot(spec.w as u32,spec.h as u32,1.0,&path,|dc| {
        let mut p = Painter { dc,gfx: &gfx,fonts: &mut fonts,res: &mut res,icons: &mut icons,requests: &mut requests };
        paint(&mut p,&DARK,&spec,&data,true,Some(&editing),&mut None).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?; Ok(())
      })?;
      let pixels = read_pixels(&gfx,&path,spec.w as u32,spec.h as u32)?; let plan = shape::plan(form,spec.w,spec.h);
      for y in (2..spec.h as usize-2).step_by(7) { for x in (2..spec.w as usize-2).step_by(7) {
        // Stay two pixels away from antialiased contour edges.
        if [-2.0,0.0,2.0].iter().all(|dy| [-2.0,0.0,2.0].iter().all(|dx| !plan.contains(x as f32+dx,y as f32+dy))) {
          assert_eq!(pixels[(y*spec.w as usize+x)*4+3],0,"{form} blocked an outside pixel ({x},{y})");
        }
      } }
    }
    // Glyph halos operate in all six renderers; no widget-sized fill or
    // border is introduced, and album art pixels are unchanged.
    let mut glass = Spec::new(1,Kind::Note,""); glass.shape = "circle".into(); glass.appearance = "glass".into();
    glass.w = 248.0; glass.h = 248.0; glass.content_opacity = 0.0;
    let glass_path = out.join("glass-circle.png"); let mut requests = Vec::new();
    gfx.snapshot(248,248,1.0,&glass_path,|dc| {
      let mut p = Painter { dc,gfx: &gfx,fonts: &mut fonts,res: &mut res,icons: &mut icons,requests: &mut requests };
      paint(&mut p,&DARK,&glass,&data,false,None,&mut None).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?; Ok(())
    })?;
    let glass_pixels = read_pixels(&gfx,&glass_path,248,248)?;
    assert!(glass_pixels[(124*248+124)*4+3].abs_diff(89) <= 2,"shaped glass background was composited more than once");
    let mut outline_images = Vec::new();
    for kind in super::super::layout::KINDS {
      let mut pair = Vec::new();
      for appearance in ["transparent","outline"] {
        let mut spec = Spec::new(1,kind,""); spec.appearance = appearance.into(); spec.note = "Unicode note 😀\nCaret / selection".into();
        let editing = Editing { text: &spec.note,caret: 7,selection: (0,0) };
        let path = out.join(format!("{appearance}-{}.png",kind.id())); let mut requests = Vec::new();
        gfx.snapshot(spec.w as u32,spec.h as u32,1.0,&path,|dc| {
          let mut p = Painter { dc,gfx: &gfx,fonts: &mut fonts,res: &mut res,icons: &mut icons,requests: &mut requests };
          paint(&mut p,&DARK,&spec,&data,false,(kind == Kind::Note).then_some(&editing),&mut None).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80004005u32 as i32),e.to_string()))?; Ok(())
        })?;
        let pixels = read_pixels(&gfx,&path,spec.w as u32,spec.h as u32)?;
        assert_eq!(pixels[(5*spec.w as usize+5)*4+3],0,"{kind:?}/{appearance} has a background"); pair.push(pixels);
      }
      assert_ne!(pair[0],pair[1],"{kind:?} glyph outline absent");
      if kind == Kind::Media { let pixel = (40*336+40)*4; assert_eq!(&pair[0][pixel..pixel+4],&pair[1][pixel..pixel+4],"outline changed artwork"); }
      outline_images.push(out.join(format!("outline-{}.png",kind.id())));
    }
    let path = out.join("glyph-outline-all-kinds.png"); let mut requests = Vec::new();
    gfx.snapshot(980,840,1.0,&path,|dc| {
      let mut p = Painter { dc,gfx: &gfx,fonts: &mut fonts,res: &mut res,icons: &mut icons,requests: &mut requests };
      for (i,kind) in super::super::layout::KINDS.into_iter().enumerate() {
        let x = 16.0+(i%2) as f32*490.0; let y = 20.0+(i/2) as f32*278.0;
        for stripe in 0..23 { p.fill(Rect::new(x+stripe as f32*20.0,y,20.0,250.0),if stripe%2 == 0 { Rgba::hex(0xd9d9cf) } else { Rgba::hex(0x303a45) })?; }
        let spec = Spec::new(1,kind,""); let bytes = std::fs::read(&outline_images[i]).map_err(|_| windows::core::Error::from_win32())?;
        let bitmap = gfx.bitmap(&bytes)?; let image: ID2D1Image = bitmap.cast()?;
        p.image_round(&image,spec.w,spec.h,Rect::new(x+16.0,y+16.0,spec.w,spec.h),0.0,1.0)?;
      }
      Ok(())
    })?;
    println!("{}",path.display());
    Ok(())
  }

  fn read_pixels(gfx: &super::super::super::gfx::Gfx,path: &std::path::Path,w: u32,h: u32) -> anyhow::Result<Vec<u8>> {
    use windows::Win32::{Foundation::GENERIC_READ,Graphics::Imaging::*};
    unsafe {
      let decoder = gfx.wic.CreateDecoderFromFilename(&windows::core::HSTRING::from(path.as_os_str()),None,GENERIC_READ,WICDecodeMetadataCacheOnDemand)?;
      let converter = gfx.wic.CreateFormatConverter()?;
      converter.Initialize(&decoder.GetFrame(0)?,&GUID_WICPixelFormat32bppPBGRA,WICBitmapDitherTypeNone,None,0.0,WICBitmapPaletteTypeMedianCut)?;
      let mut pixels = vec![0;w as usize*h as usize*4]; converter.CopyPixels(std::ptr::null(),w*4,&mut pixels)?; Ok(pixels)
    }
  }

  #[test]
  fn utf16_positions_count_surrogates() {
    let s = "a😀b";
    assert_eq!(utf16_at(s, 0), 0);
    assert_eq!(utf16_at(s, 2), 3);
    assert_eq!(char_at_utf16(s, 3), 2);
    assert_eq!(char_at_utf16(s, 99), 3);
  }
}
