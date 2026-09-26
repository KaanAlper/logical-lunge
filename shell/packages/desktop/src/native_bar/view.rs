//! Layout and drawing of one bar: `ui/bar.html` + `ui/styles.css` with the
//! same sizes (ii Appearance.qml), in DIPs.

use std::collections::HashMap;

use windows::{
  core::{Interface, Result},
  Win32::Graphics::{
    Direct2D::{
      Common::{D2D_SIZE_F, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN},
      ID2D1DeviceContext, ID2D1Factory, ID2D1SolidColorBrush, ID2D1StrokeStyle,
      D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND,
      D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_STROKE_STYLE_PROPERTIES,
      D2D1_SWEEP_DIRECTION_CLOCKWISE,
    },
    DirectWrite::{
      IDWriteTextLayout, IDWriteTypography, DWRITE_FONT_FEATURE,
      DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE,
    },
  },
};

use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::{
  ID2D1Bitmap1, ID2D1Image, CLSID_D2D1Saturation, D2D1_BRUSH_PROPERTIES, D2D1_EXTEND_MODE_CLAMP,
  D2D1_IMAGE_BRUSH_PROPERTIES, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
  D2D1_INTERPOLATION_MODE_LINEAR, D2D1_PROPERTY_TYPE_FLOAT, D2D1_SATURATION_PROP_SATURATION,
};

use super::{
  fonts::{Fonts, TextStyle},
  gfx::{ellipse, pt, Gfx, Rect, Rgba},
  icons::Icons,
  model::Model,
};

pub const BAR_H: f32 = 40.0;
pub const SHOWN: usize = 10; // bar.workspaces.shown
const WS: f32 = 26.0; // workspaceButtonWidth
const MARGIN: f32 = 2.0; // activeWorkspaceMargin
const SIDE_W: [f32; 3] = [360.0, 280.0, 190.0]; // barCenterSideModuleWidth / Shortened / HellaShortened
const SCREEN_ROUNDING: f32 = 23.0;
const GROUP_H: f32 = 32.0;
const GROUP_R: f32 = 12.0;

/// ii thresholds 1200 / 1000, raised a little for Windows' wider tray + title.
pub fn shorten_level(width: f32) -> usize {
  if width <= 1100.0 {
    2
  } else if width <= 1440.0 {
    1
  } else {
    0
  }
}

#[derive(Clone, Copy)]
pub struct Theme {
  pub primary: Rgba,
  pub on_primary: Rgba,
  pub sec_container: Rgba,
  pub on_sec_container: Rgba,
  pub error: Rgba,
  pub layer0: Rgba,
  pub layer1: Rgba,
  pub layer1_hover: Rgba,
  pub on_layer0: Rgba,
  pub on_layer1: Rgba,
  pub subtext: Rgba,
  pub inactive: Rgba,
  pub occupied: Rgba,
}

/// Material You dark, purple seed (styles.css `:root`).
pub const DARK: Theme = Theme {
  primary: Rgba::hex(0xd0bcff),
  on_primary: Rgba::hex(0x381e72),
  sec_container: Rgba::hex(0x4a4458),
  on_sec_container: Rgba::hex(0xe8def8),
  error: Rgba::hex(0xffb4ab),
  layer0: Rgba::hex(0x141218),
  layer1: Rgba::hex(0x1d1b20),
  layer1_hover: Rgba::hex(0x36323b),
  on_layer0: Rgba::hex(0xe6e0e9),
  on_layer1: Rgba::hex(0xe6e0e9),
  subtext: Rgba::hex(0x938f99),
  inactive: Rgba::hex(0x8a8591),
  occupied: Rgba(74, 68, 88, 0.6),
};

/// styles.css `:root[data-theme="light"]`.
pub const LIGHT: Theme = Theme {
  primary: Rgba::hex(0x6750a4),
  on_primary: Rgba::hex(0xffffff),
  sec_container: Rgba::hex(0xe8def8),
  on_sec_container: Rgba::hex(0x1d192b),
  error: Rgba::hex(0xb3261e),
  layer0: Rgba::hex(0xfef7ff),
  layer1: Rgba::hex(0xf3edf7),
  layer1_hover: Rgba::hex(0xe6e0e9),
  on_layer0: Rgba::hex(0x1d1b20),
  on_layer1: Rgba::hex(0x1d1b20),
  subtext: Rgba::hex(0x79747e),
  inactive: Rgba::hex(0x9e98a3),
  occupied: Rgba(232, 222, 248, 0.9),
};

#[derive(Clone, Debug, PartialEq)]
pub enum HitKind {
  Search,
  ActiveWindow,
  Paused,
  Mode(String),
  Workspace(u32),
  Resources,
  Media,
  Snip,
  Osk,
  Theme,
  Indicators,
  TrayMore,
  TrayIcon(String),
}

pub struct Hit {
  pub rect: Rect,
  pub kind: HitKind,
}

/// What one paint produced: clickable areas and the wheel zones.
#[derive(Default)]
pub struct Frame {
  pub hits: Vec<Hit>,
  pub left_zone: Rect,
  pub right_zone: Rect,
  pub ws_track: Rect,
  /// First workspace shown (pages of 10).
  pub ws_base: u32,
}

impl Frame {
  pub fn hit(&self, x: f32, y: f32) -> Option<&Hit> {
    self.hits.iter().rev().find(|h| h.rect.contains(x, y))
  }
}

/// Device-level resources reused by every paint.
pub struct Res {
  round: ID2D1StrokeStyle,
  tabular: IDWriteTypography,
  brushes: HashMap<(u8, u8, u8, u16), ID2D1SolidColorBrush>,
}

impl Res {
  pub fn new(gfx: &Gfx) -> Result<Self> {
    unsafe {
      let factory: ID2D1Factory = gfx.dc.GetFactory()?;
      let round = factory.CreateStrokeStyle(
        &D2D1_STROKE_STYLE_PROPERTIES {
          startCap: D2D1_CAP_STYLE_ROUND,
          endCap: D2D1_CAP_STYLE_ROUND,
          ..Default::default()
        },
        None,
      )?;
      let tabular = gfx.dwrite.CreateTypography()?;
      tabular.AddFontFeature(DWRITE_FONT_FEATURE {
        nameTag: DWRITE_FONT_FEATURE_TAG_TABULAR_FIGURES,
        parameter: 1,
      })?;
      Ok(Self { round, tabular, brushes: HashMap::new() })
    }
  }
}

pub struct Painter<'a> {
  pub dc: &'a ID2D1DeviceContext,
  pub gfx: &'a Gfx,
  pub fonts: &'a mut Fonts,
  pub res: &'a mut Res,
  pub icons: &'a mut Icons,
  /// windows whose own icon should be asked from the core
  pub requests: &'a mut Vec<i64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
  Left,
  Center,
}

impl Painter<'_> {
  fn brush(&mut self, c: Rgba) -> Result<ID2D1SolidColorBrush> {
    let key = (c.0, c.1, c.2, (c.3 * 1000.0) as u16);
    if let Some(b) = self.res.brushes.get(&key) {
      return Ok(b.clone());
    }
    let b = self.gfx.brush(c)?;
    self.res.brushes.insert(key, b.clone());
    Ok(b)
  }

  fn fill(&mut self, r: Rect, c: Rgba) -> Result<()> {
    let b = self.brush(c)?;
    unsafe { self.dc.FillRectangle(&r.d2d(), &b) };
    Ok(())
  }

  fn fill_round(&mut self, r: Rect, radius: f32, c: Rgba) -> Result<()> {
    let b = self.brush(c)?;
    unsafe { self.dc.FillRoundedRectangle(&r.rounded(radius.min(r.h / 2.0).min(r.w / 2.0)), &b) };
    Ok(())
  }

  fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) -> Result<()> {
    let b = self.brush(c)?;
    unsafe { self.dc.FillEllipse(&ellipse(cx, cy, r), &b) };
    Ok(())
  }

  fn layout(&mut self, s: &str, style: TextStyle, max_w: f32, h: f32, tabular: bool) -> anyhow::Result<IDWriteTextLayout> {
    let format = self.fonts.text(style)?;
    let wide: Vec<u16> = s.encode_utf16().collect();
    unsafe {
      let layout = self.gfx.dwrite.CreateTextLayout(&wide, &format, max_w.max(1.0), h)?;
      if tabular {
        layout.SetTypography(
          &self.res.tabular,
          DWRITE_TEXT_RANGE { startPosition: 0, length: wide.len() as u32 },
        )?;
      }
      Ok(layout)
    }
  }

  fn width_of(layout: &IDWriteTextLayout) -> f32 {
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe {
      let _ = layout.GetMetrics(&mut m);
    }
    m.widthIncludingTrailingWhitespace
  }

  /// Measures single-line text.
  fn measure(&mut self, s: &str, style: TextStyle) -> anyhow::Result<f32> {
    self.measure_with(s, style, false)
  }

  /// Measures with tabular figures (as drawn for numbers that tick).
  fn measure_with(&mut self, s: &str, style: TextStyle, tabular: bool) -> anyhow::Result<f32> {
    let l = self.layout(s, style, 10000.0, 100.0, tabular)?;
    Ok(Self::width_of(&l))
  }

  /// Draws text vertically centred in `r` (ellipsis when it does not fit).
  fn text(&mut self, s: &str, r: Rect, style: TextStyle, c: Rgba, align: Align, tabular: bool) -> anyhow::Result<f32> {
    let layout = self.layout(s, style, r.w, r.h, tabular)?;
    let w = Self::width_of(&layout).min(r.w);
    let x = if align == Align::Center { r.x + (r.w - w) / 2.0 } else { r.x };
    let b = self.brush(c)?;
    unsafe { self.dc.DrawTextLayout(pt(x, r.y), &layout, &b, D2D1_DRAW_TEXT_OPTIONS_NONE) };
    Ok(w)
  }

  /// Material Symbols icon (a ligature) centred on (cx, cy).
  fn icon(&mut self, name: &str, cx: f32, cy: f32, size: f32, fill: bool, c: Rgba) -> anyhow::Result<()> {
    let format = self.fonts.icon(size, fill)?;
    let wide: Vec<u16> = name.encode_utf16().collect();
    let box_ = size * 1.5;
    unsafe {
      let layout = self.gfx.dwrite.CreateTextLayout(&wide, &format, box_ * 4.0, box_)?;
      let w = Self::width_of(&layout);
      let b = self.brush(c)?;
      self.dc.DrawTextLayout(pt(cx - w / 2.0, cy - box_ / 2.0), &layout, &b, D2D1_DRAW_TEXT_OPTIONS_NONE);
    }
    Ok(())
  }

  /// A bitmap scaled into `r` (tray icons).
  fn image(&mut self, bmp: &ID2D1Bitmap1, r: Rect) {
    unsafe {
      self.dc.DrawBitmap(bmp, Some(&r.d2d()), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
    }
  }

  /// A bitmap cut to a circle (`object-fit: cover`), optionally desaturated
  /// (`filter: saturate(...)`).
  fn image_circle(&mut self, bmp: &ID2D1Bitmap1, cx: f32, cy: f32, d: f32, saturation: Option<f32>) -> Result<()> {
    unsafe {
      let size = bmp.GetSize();
      let (bw, bh) = (size.width.max(1.0), size.height.max(1.0));
      let k = d / bw.min(bh);
      let image: ID2D1Image = match saturation {
        Some(sat) => {
          let fx = self.dc.CreateEffect(&CLSID_D2D1Saturation)?;
          fx.SetInput(0, bmp, true);
          fx.SetValue(D2D1_SATURATION_PROP_SATURATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &sat.to_le_bytes())?;
          fx.GetOutput()?
        }
        None => bmp.cast()?,
      };
      let brush = self.dc.CreateImageBrush(
        &image,
        &D2D1_IMAGE_BRUSH_PROPERTIES {
          sourceRectangle: Rect::new(0.0, 0.0, bw, bh).d2d(),
          extendModeX: D2D1_EXTEND_MODE_CLAMP,
          extendModeY: D2D1_EXTEND_MODE_CLAMP,
          interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
        },
        Some(&D2D1_BRUSH_PROPERTIES {
          opacity: 1.0,
          transform: Matrix3x2 {
            M11: k,
            M12: 0.0,
            M21: 0.0,
            M22: k,
            M31: cx - bw * k / 2.0,
            M32: cy - bh * k / 2.0,
          },
        }),
      )?;
      self.dc.FillEllipse(&ellipse(cx, cy, d / 2.0), &brush);
    }
    Ok(())
  }

  /// Circular progress (ii CircularProgress): track + value arc from 12 o'clock.
  fn ring(&mut self, cx: f32, cy: f32, r: f32, stroke: f32, frac: f32, track: Rgba, value: Rgba) -> Result<()> {
    let tb = self.brush(track)?;
    let vb = self.brush(value)?;
    unsafe {
      self.dc.DrawEllipse(&ellipse(cx, cy, r), &tb, stroke, None);
      let frac = frac.clamp(0.0, 1.0);
      if frac >= 0.999 {
        self.dc.DrawEllipse(&ellipse(cx, cy, r), &vb, stroke, None);
      } else if frac > 0.001 {
        let factory: ID2D1Factory = self.dc.GetFactory()?;
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(pt(cx, cy - r), D2D1_FIGURE_BEGIN_HOLLOW);
        let a = frac * std::f32::consts::TAU;
        sink.AddArc(&D2D1_ARC_SEGMENT {
          point: pt(cx + r * a.sin(), cy - r * a.cos()),
          size: D2D_SIZE_F { width: r, height: r },
          rotationAngle: 0.0,
          sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
          arcSize: if frac > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
        });
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        self.dc.DrawGeometry(&path.cast::<windows::Win32::Graphics::Direct2D::ID2D1Geometry>()?, &vb, stroke, &self.res.round);
      }
    }
    Ok(())
  }
}

const fn style(size: f32) -> TextStyle {
  TextStyle { size, weight: 450.0 }
}

/// Paints the whole bar (`w` DIPs wide) and returns its hit areas.
pub fn paint(p: &mut Painter, m: &Model, t: &Theme, w: f32, hover: Option<&HitKind>, hover_left: bool, hover_right: bool) -> anyhow::Result<Frame> {
  let mut f = Frame::default();
  let level = shorten_level(w);
  let side_w = SIDE_W[level];
  let track_w = SHOWN as f32 * WS;
  let middle_w = side_w * 2.0 + (track_w + 6.0) + 8.0;
  let side_space = ((w - middle_w) / 2.0).max(0.0);
  let hovered = |k: &HitKind| hover == Some(k);

  p.fill(Rect::new(0.0, 0.0, w, BAR_H), t.layer0)?;

  // ---------------- left: search, active window, modes ----------------
  f.left_zone = Rect::new(0.0, 0.0, side_space, BAR_H);
  let mut x = SCREEN_ROUNDING;
  let search = Rect::new(x, 4.0, 32.0, 32.0);
  if hovered(&HitKind::Search) {
    p.fill_round(search, 16.0, t.layer1_hover)?;
  }
  p.icon("search", search.x + 16.0, 20.0, 21.0, false, t.on_layer0)?;
  f.hits.push(Hit { rect: search, kind: HitKind::Search });
  x += 32.0;

  // pills (paused, binding modes) come after the active window
  let pill_style = style(13.0);
  let mut pills: Vec<(HitKind, String, bool)> = Vec::new();
  if m.wm.paused {
    pills.push((HitKind::Paused, m.tr("Duraklatıldı"), true));
  }
  for (name, display) in &m.wm.binding_modes {
    pills.push((HitKind::Mode(name.clone()), display.clone(), false));
  }
  let mut pill_ws = Vec::new();
  for (_, label, _) in &pills {
    pill_ws.push(p.measure(label, pill_style)? + 20.0);
  }
  let pills_total: f32 = pill_ws.iter().map(|w| w + 8.0).sum();

  if level == 0 {
    x += 10.0;
    let aw = (side_space - x - SCREEN_ROUNDING - pills_total).max(0.0);
    let area = Rect::new(x, 4.0, aw, 32.0);
    let (cls, title) = match &m.wm.focused_window {
      Some((process, title)) => (
        process.trim_end_matches(".exe").trim_end_matches(".EXE").to_lowercase(),
        title.clone(),
      ),
      None => (
        m.tr("Masaüstü"),
        format!("Workspace {}", m.wm.focused_workspace().map(|w| w.name.as_str()).unwrap_or("1")),
      ),
    };
    if aw > 8.0 {
      p.text(&cls, Rect::new(x, 4.5, aw, 14.0), style(12.0), t.subtext, Align::Left, false)?;
      p.text(&title, Rect::new(x, 16.5, aw, 19.0), style(15.0), t.on_layer0, Align::Left, false)?;
    }
    f.hits.push(Hit { rect: area, kind: HitKind::ActiveWindow });
    x += aw;
  }
  for ((kind, label, paused), pw) in pills.into_iter().zip(pill_ws) {
    x += 8.0;
    let r = Rect::new(x, 7.0, pw, 26.0);
    let (bg, fg) = if paused { (Rgba::hex(0x93000a), Rgba::hex(0xffdad6)) } else { (t.primary, t.on_primary) };
    p.fill_round(r, 13.0, bg)?;
    p.text(&label, r, pill_style, fg, Align::Center, false)?;
    f.hits.push(Hit { rect: r, kind });
    x += pw;
  }
  scroll_hint(p, t, 4.0, "light_mode", hover_left)?;

  // ---------------- middle ----------------
  let mut x = side_space;
  let gy = (BAR_H - GROUP_H) / 2.0;

  // left side group: resources + media
  let group = Rect::new(x, gy, side_w, GROUP_H);
  p.fill_round(group, GROUP_R, t.layer1)?;
  let mut cx = x + 5.0 + 4.0;
  let res_start = cx;
  let mut resources: Vec<(&str, f32, f32)> = Vec::new();
  if let Some(mem) = &m.memory {
    resources.push(("memory", mem.usage, 95.0));
    if mem.total_swap > 0 {
      resources.push(("swap_horiz", mem.used_swap as f32 / mem.total_swap as f32 * 100.0, 85.0));
    }
  }
  if let Some(cpu) = &m.cpu {
    resources.push(("planner_review", cpu.usage, 90.0));
  }
  for (i, (icon, pct, warn)) in resources.iter().enumerate() {
    if i > 0 {
      cx += 6.0;
    }
    let pct = pct.round();
    let value = if pct >= *warn { t.error } else { t.on_sec_container };
    p.ring(cx + 10.0, 20.0, 8.5, 2.0, pct / 100.0, t.sec_container, value)?;
    p.icon(icon, cx + 10.0, 20.0, 13.0, true, t.on_sec_container)?;
    cx += 22.0;
    let label = format!("{}", pct as i32);
    let tw = p.measure_with(&label, style(15.0), true)?.max(26.0);
    p.text(&label, Rect::new(cx, gy, tw, GROUP_H), style(15.0), t.on_layer1, Align::Center, true)?;
    cx += tw;
  }
  cx += 4.0;
  if !resources.is_empty() {
    f.hits.push(Hit { rect: Rect::new(res_start - 4.0, gy, cx - res_start + 4.0, GROUP_H), kind: HitKind::Resources });
  }
  if level < 2 {
    cx += 4.0;
    let media_r = Rect::new(cx, gy, group.right() - 5.0 - cx, GROUP_H);
    media(p, m, t, media_r)?;
    if m.media_title().is_some() {
      f.hits.push(Hit { rect: media_r, kind: HitKind::Media });
    }
  }
  x += side_w + 4.0;

  // workspaces
  let ws_group = Rect::new(x, gy, track_w + 6.0, GROUP_H);
  p.fill_round(ws_group, GROUP_R, t.layer1)?;
  let track = Rect::new(x + 3.0, gy + (GROUP_H - WS) / 2.0, track_w, WS);
  f.ws_track = track;
  workspaces(p, m, t, track, hover, &mut f)?;
  x += track_w + 6.0 + 4.0;

  // right side group: clock, utils, battery
  let group = Rect::new(x, gy, side_w, GROUP_H);
  p.fill_round(group, GROUP_R, t.layer1)?;
  let mut right_edge = group.right() - 5.0;
  if level < 2 {
    if let Some(bat) = &m.battery {
      right_edge -= 4.0;
      let r = Rect::new(right_edge - 38.0, 11.0, 38.0, 18.0);
      battery(p, t, r, bat.charge_percent, bat.is_charging)?;
      right_edge -= 38.0 + 4.0 + 4.0;
    }
  }
  if level == 0 {
    let buttons = [
      (HitKind::Theme, if m.light { "dark_mode" } else { "light_mode" }),
      (HitKind::Osk, "keyboard"),
      (HitKind::Snip, "screenshot_region"),
    ];
    for (kind, icon) in buttons {
      let r = Rect::new(right_edge - 26.0, 7.0, 26.0, 26.0);
      let bg = if hovered(&kind) { lighten(t.sec_container, 1.2) } else { t.sec_container };
      p.fill_round(r, 13.0, bg)?;
      p.icon(icon, r.x + 13.0, 20.0, 16.0, false, t.on_sec_container)?;
      f.hits.push(Hit { rect: r, kind });
      right_edge -= 26.0 + 4.0;
    }
  }
  // clock: centred in what is left (flex: 1)
  let clock = Rect::new(group.x + 5.0, gy, (right_edge - group.x - 5.0).max(0.0), GROUP_H);
  let tw = p.measure_with(&m.time, style(17.0), true)?;
  let mut parts_w = tw;
  let (sep_w, date_w) = if level < 2 {
    (p.measure("•", style(15.0))?, p.measure(&m.date, style(15.0))?)
  } else {
    (0.0, 0.0)
  };
  if level < 2 {
    parts_w += 4.0 + sep_w + 4.0 + date_w;
  }
  let mut tx = clock.x + ((clock.w - parts_w) / 2.0).max(0.0);
  p.text(&m.time, Rect::new(tx, gy, tw + 1.0, GROUP_H), style(17.0), t.on_layer1, Align::Left, true)?;
  if level < 2 {
    tx += tw + 4.0;
    p.text("•", Rect::new(tx, gy, sep_w + 1.0, GROUP_H), style(15.0), t.on_layer1, Align::Left, false)?;
    tx += sep_w + 4.0;
    let dw = date_w.min(clock.right() - tx).max(0.0);
    p.text(&m.date, Rect::new(tx, gy, dw + 1.0, GROUP_H), style(15.0), t.on_layer1, Align::Left, false)?;
  }

  // ---------------- right: indicators, tray ----------------
  f.right_zone = Rect::new(w - side_space, 0.0, side_space, BAR_H);
  let mut icons: Vec<&str> = Vec::new();
  if m.volume_muted() {
    icons.push("volume_off");
  }
  if m.mic_muted() {
    icons.push("mic_off");
  }
  icons.push(m.network_icon());
  let iw = icons.len() as f32 * 19.0 + (icons.len() as f32 - 1.0) * 15.0 + 20.0;
  let ind = Rect::new(w - SCREEN_ROUNDING - iw, 5.0, iw, 30.0);
  if hovered(&HitKind::Indicators) {
    p.fill_round(ind, 15.0, t.layer1_hover)?;
  }
  let mut ix = ind.x + 10.0;
  for icon in icons {
    p.icon(icon, ix + 9.5, 20.0, 19.0, false, t.on_layer0)?;
    ix += 19.0 + 15.0;
  }
  f.hits.push(Hit { rect: ind, kind: HitKind::Indicators });

  // ii SysTray.qml: pinned icons in the bar, the rest under the arrow
  if level == 0 && m.tray_count() > 0 {
    let pinned = m.pinned_icons();
    let items_w = pinned.len() as f32 * 26.0 + (pinned.len().max(1) as f32 - 1.0) * 2.0;
    let mut tx = ind.x - 5.0 - if pinned.is_empty() { 0.0 } else { items_w };
    for ic in &pinned {
      let r = Rect::new(tx, 7.0, 26.0, 26.0);
      let kind = HitKind::TrayIcon(ic.id.clone());
      if hovered(&kind) {
        p.fill_round(r, 13.0, t.layer1_hover)?;
      }
      if let Some(bmp) = p.icons.tray(p.gfx, &ic.icon_hash, &ic.icon_bytes) {
        p.image(&bmp, Rect::new(r.x + 5.0, r.y + 5.0, 16.0, 16.0));
      }
      f.hits.push(Hit { rect: r, kind });
      tx += 28.0;
    }
    let more_x = ind.x - 5.0 - if pinned.is_empty() { 0.0 } else { items_w + 2.0 } - 26.0;
    let more = Rect::new(more_x, 7.0, 26.0, 26.0);
    if hovered(&HitKind::TrayMore) {
      p.fill_round(more, 13.0, t.layer1_hover)?;
    }
    p.icon("expand_more", more.x + 13.0, 20.0, 20.0, false, t.on_layer0)?;
    f.hits.push(Hit { rect: more, kind: HitKind::TrayMore });
  }
  scroll_hint(p, t, w - 4.0 - 14.0, "volume_up", hover_right)?;

  Ok(f)
}

/// ii ScrollHint.qml: three stacked icons, faint until the edge zone is hovered.
fn scroll_hint(p: &mut Painter, t: &Theme, x: f32, icon: &str, hot: bool) -> anyhow::Result<()> {
  let c = t.subtext.alpha(if hot { 1.0 } else { 0.45 });
  let cx = x + 7.0;
  let top = (BAR_H - 32.0) / 2.0;
  p.icon("keyboard_arrow_up", cx, top + 7.0, 14.0, false, c)?;
  p.icon(icon, cx, top + 16.0, 14.0, false, c)?;
  p.icon("keyboard_arrow_down", cx, top + 25.0, 14.0, false, c)?;
  Ok(())
}

/// ii Media.qml: progress ring with play state + "title • artist".
fn media(p: &mut Painter, m: &Model, t: &Theme, r: Rect) -> anyhow::Result<()> {
  let x = r.x + 4.0;
  let cy = BAR_H / 2.0;
  match m.media_title() {
    None => {
      p.icon("music_note", x + 11.0, cy, 14.0, true, t.on_sec_container)?;
      p.text(&m.tr("Medya yok"), Rect::new(x + 28.0, r.y, (r.w - 36.0).max(0.0), r.h), style(15.0), t.subtext, Align::Left, false)?;
    }
    Some((text, progress, playing)) => {
      p.ring(x + 11.0, cy, 9.5, 2.0, progress, t.sec_container, t.on_sec_container)?;
      p.icon(if playing { "pause" } else { "music_note" }, x + 11.0, cy, 14.0, true, t.on_sec_container)?;
      p.text(&text, Rect::new(x + 28.0, r.y, (r.w - 36.0).max(0.0), r.h), style(15.0), t.on_layer1, Align::Left, false)?;
    }
  }
  Ok(())
}

/// ii Workspaces.qml: merged occupied background, active pill, app icons / dots.
fn workspaces(p: &mut Painter, m: &Model, t: &Theme, track: Rect, hover: Option<&HitKind>, f: &mut Frame) -> anyhow::Result<()> {
  let current: u32 = m.wm.focused_workspace().and_then(|w| w.name.parse().ok()).unwrap_or(1).max(1);
  let base = (current - 1) / SHOWN as u32 * SHOWN as u32;
  f.ws_base = base;
  let idx = (current - base - 1) as usize;
  let occupied: Vec<bool> = (0..SHOWN)
    .map(|i| {
      let n = (base + i as u32 + 1).to_string();
      m.wm.all_workspaces().any(|w| w.name == n && w.biggest.is_some())
    })
    .collect();

  // occupied runs
  let mut i = 0;
  while i < SHOWN {
    if !occupied[i] {
      i += 1;
      continue;
    }
    let start = i;
    while i + 1 < SHOWN && occupied[i + 1] {
      i += 1;
    }
    let r = Rect::new(track.x + start as f32 * WS, track.y, (i - start + 1) as f32 * WS, WS);
    p.fill_round(r, WS / 2.0, t.occupied)?;
    i += 1;
  }
  if m.wm.connected {
    let r = Rect::new(track.x + idx as f32 * WS + MARGIN, track.y + MARGIN, WS - 2.0 * MARGIN, WS - 2.0 * MARGIN);
    p.fill_round(r, r.h / 2.0, t.primary)?;
  }
  for i in 0..SHOWN {
    let n = base + i as u32 + 1;
    let cell = Rect::new(track.x + i as f32 * WS, track.y, WS, WS);
    let (cx, cy) = (cell.x + WS / 2.0, cell.y + WS / 2.0);
    if hover == Some(&HitKind::Workspace(n)) {
      p.fill_round(cell.inset(2.0, 2.0), WS, t.primary.alpha(0.10))?;
    }
    // ii showAppIcons: the biggest window's icon (workspaceIconSize 26 * 0.69)
    let big = m.wm.all_workspaces().find(|w| w.name == n.to_string()).and_then(|w| w.biggest.as_ref());
    let mut drawn = false;
    if let Some(win) = big {
      let (bmp, ask) = p.icons.for_window(p.gfx, &win.process, win.handle);
      if let Some(h) = ask {
        p.requests.push(h);
      }
      if let Some(bmp) = bmp {
        let active = i == idx;
        let d = if active { 18.0 * 1.05 } else { 18.0 };
        p.image_circle(&bmp, cx, cy, d, if active { None } else { Some(0.7) })?;
        drawn = true;
      }
    }
    if !drawn {
      let c = if i == idx {
        t.on_primary
      } else if occupied[i] {
        t.on_sec_container
      } else {
        t.inactive
      };
      p.fill_circle(cx, cy, 4.7 / 2.0, c)?;
    }
    f.hits.push(Hit { rect: cell, kind: HitKind::Workspace(n) });
  }
  Ok(())
}

/// ii BatteryIndicator.qml (ClippedProgressBar).
fn battery(p: &mut Painter, t: &Theme, r: Rect, percent: f32, charging: bool) -> anyhow::Result<()> {
  let pct = percent.round().clamp(0.0, 100.0);
  let low = pct <= 20.0 && !charging;
  p.fill_round(r, r.h / 2.0, t.sec_container)?;
  unsafe {
    p.dc.PushAxisAlignedClip(&Rect::new(r.x, r.y, r.w * pct / 100.0, r.h).d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
  }
  p.fill_round(r, r.h / 2.0, if low { t.error } else { t.on_sec_container })?;
  unsafe { p.dc.PopAxisAlignedClip() };
  let label = format!("{}", pct as i32);
  let color = if pct > 55.0 { Rgba::hex(0x1d1b20) } else { Rgba::hex(0xe8def8) };
  let st = TextStyle { size: 11.0, weight: 600.0 };
  let lw = p.measure_with(&label, st, true)?;
  let total = lw + if charging && pct < 100.0 { 10.0 } else { 0.0 };
  let mut x = r.x + (r.w - total) / 2.0;
  if charging && pct < 100.0 {
    p.icon("bolt", x + 5.0, r.y + r.h / 2.0, 12.0, true, color)?;
    x += 10.0;
  }
  p.text(&label, Rect::new(x, r.y, lw + 1.0, r.h), st, color, Align::Left, true)?;
  Ok(())
}

fn lighten(c: Rgba, k: f32) -> Rgba {
  let f = |v: u8| ((v as f32 * k).min(255.0)) as u8;
  Rgba(f(c.0), f(c.1), f(c.2), c.3)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OsdKind {
  Volume,
  Mic,
  Brightness,
  Gamma,
}

pub const OSD_W: f32 = 200.0;
pub const OSD_H: f32 = 48.0;
/// room around the pill for its shadow
pub const OSD_PAD: f32 = 12.0;

/// ii OsdValueIndicator.qml (styles.css `.osd`), drawn at (OSD_PAD, OSD_PAD).
pub fn paint_osd(p: &mut Painter, m: &Model, t: &Theme, kind: OsdKind, value: i32) -> anyhow::Result<()> {
  let r = Rect::new(OSD_PAD, OSD_PAD, OSD_W, OSD_H);
  // box-shadow: 0 2px 10px rgba(0 0 0 / 35%) -- a few soft layers
  for k in 1..=6 {
    let g = k as f32 * 1.6;
    p.fill_round(Rect::new(r.x - g, r.y + 2.0 - g, r.w + 2.0 * g, r.h + 2.0 * g), r.h / 2.0 + g, Rgba(0, 0, 0, 0.05))?;
  }
  p.fill_round(r, r.h / 2.0, t.layer0)?;
  let v = value.clamp(0, 100);
  let (icon, label) = match kind {
    OsdKind::Volume => (if v == 0 { "volume_off" } else { "volume_up" }, "Ses"),
    OsdKind::Mic => (if v == 0 { "mic_off" } else { "mic" }, "Mikrofon"),
    OsdKind::Gamma => ("brightness_4", "Gama"),
    OsdKind::Brightness => ("light_mode", "Parlaklık"),
  };
  let size = if kind == OsdKind::Brightness { 20.0 + 10.0 * v as f32 / 100.0 } else { 30.0 };
  p.icon(icon, r.x + 10.0 + 15.0, r.y + r.h / 2.0, size, false, t.on_layer0)?;
  // body: padding 9 20 9 10, icon 30 + gap 10
  let bx = r.x + 10.0 + 30.0 + 10.0;
  let bw = r.right() - 20.0 - bx;
  let row = Rect::new(bx + 2.0, r.y + 9.0, bw - 4.0, 19.0);
  p.text(&m.tr(label), row, style(15.0), t.on_layer0, Align::Left, false)?;
  let num = v.to_string();
  let nw = p.measure_with(&num, style(15.0), true)?;
  p.text(&num, Rect::new(row.right() - nw, row.y, nw + 1.0, row.h), style(15.0), t.on_layer0, Align::Left, true)?;
  let bar = Rect::new(bx, row.bottom() + 5.0, bw, 4.0);
  p.fill_round(bar, 2.0, t.sec_container)?;
  p.fill_round(Rect::new(bar.x, bar.y, bar.w * v as f32 / 100.0, bar.h), 2.0, t.primary)?;
  p.fill_circle(bar.right() - 2.0, bar.y + 2.0, 2.0, t.primary)?;
  Ok(())
}
