//! Popups under the bar (ii StyledPopup): resources, media, the tray panel,
//! tooltips and the dragged tray icon. Each is a small topmost window that
//! never takes the keyboard, drawn into one DirectComposition surface.
//! Opening slides down and fades in, closing goes back up (styles.css
//! `popDown` 240 ms / `popUp` 170 ms); both run in the compositor.

use std::mem::ManuallyDrop;

use windows::{
  core::{Interface, Result, HSTRING},
  Win32::{
    Foundation::HWND,
    Graphics::{
      Direct2D::{
        Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BORDER_MODE_HARD, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_PIXEL_FORMAT, D2D_SIZE_U},
        ID2D1Bitmap1, ID2D1Image, CLSID_D2D1ColorMatrix, CLSID_D2D1GaussianBlur,
        D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
        D2D1_COLORMATRIX_PROP_COLOR_MATRIX,
        D2D1_GAUSSIANBLUR_PROP_BORDER_MODE, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION,
        D2D1_INTERPOLATION_MODE_LINEAR, D2D1_PROPERTY_TYPE_ENUM, D2D1_PROPERTY_TYPE_FLOAT,
        D2D1_PROPERTY_TYPE_MATRIX_5X4,
      },
      DirectComposition::{
        IDCompositionEffectGroup, IDCompositionRectangleClip, IDCompositionTarget, IDCompositionVisual2,
      },
      Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
      CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE,
      SWP_SHOWWINDOW, SW_HIDE, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW,
      WS_EX_TOPMOST, WS_POPUP,
    },
  },
};

use super::{
  anim::{Animated, POP_IN, POP_OUT},
  fonts::TextStyle,
  gfx::{self, Gfx, Rect, Rgba},
  model::Model,
  view::{style, Align, Painter, Theme},
  Layer, CLASS,
};

/// Room around a popup's box for its shadow (DIPs).
pub const PAD: f32 = 14.0;
/// Gap between the bar and a popup (`top: 44px` under a 40 px bar).
pub const GAP: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PopKind {
  Res,
  Media,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PopHit {
  Prev,
  Next,
  Play,
  /// the progress bar (its rect, to map a click to a position)
  Seek(Rect),
  TrayIcon(String),
}

pub struct Hit {
  pub rect: Rect,
  pub kind: PopHit,
}

pub fn hit_at(hits: &[Hit], x: f32, y: f32) -> Option<&Hit> {
  hits.iter().rev().find(|h| h.rect.contains(x, y))
}

/// `tools\temps\lunge-temps.exe --read`
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Temps {
  pub cpu: Option<f32>,
  pub gpu: Option<f32>,
  pub gpu_hot: Option<f32>,
  pub gpu_load: Option<f32>,
}

impl Temps {
  pub fn parse(json: &str) -> Option<Self> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    let f = |k: &str| v[k].as_f64().map(|x| x as f32);
    Some(Self { cpu: f("cpu"), gpu: f("gpu"), gpu_hot: f("gpuHot"), gpu_load: f("gpuLoad") })
  }
}

/// How a window appears: popups slide out of the bar, tooltips and the drag
/// ghost only fade.
#[derive(Clone, Copy, PartialEq)]
pub enum Motion {
  Slide,
  Fade,
  None,
}

pub struct PopWin {
  pub hwnd: HWND,
  pub scale: f32,
  motion: Motion,
  _target: IDCompositionTarget,
  root: IDCompositionVisual2,
  fx: IDCompositionEffectGroup,
  layer: Layer,
  clip: IDCompositionRectangleClip,
  /// surface size in physical pixels
  size: (u32, u32),
  off: Animated,
  bottom: Animated,
  opacity: Animated,
  pub shown: bool,
  pub closing: bool,
  /// window position (screen pixels)
  pub origin: (i32, i32),
  /// clickable areas, in DIPs from the window's top-left
  pub hits: Vec<Hit>,
}

impl PopWin {
  pub fn new(gfx: &Gfx, title: &str, scale: f32, motion: Motion) -> anyhow::Result<Self> {
    unsafe {
      let hwnd = CreateWindowExW(
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
        CLASS,
        &HSTRING::from(title),
        WS_POPUP,
        0,
        0,
        1,
        1,
        None,
        None,
        GetModuleHandleW(None)?,
        None,
      )?;
      let made = (|| -> Result<(IDCompositionTarget, IDCompositionVisual2, IDCompositionEffectGroup, Layer, IDCompositionRectangleClip)> {
        let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
        let root = gfx.dcomp.CreateVisual()?;
        let fx = gfx.dcomp.CreateEffectGroup()?;
        root.SetEffect(&fx)?;
        let layer = Layer::new(gfx, 1, 1)?;
        let clip = gfx.dcomp.CreateRectangleClip()?;
        layer.visual.SetClip(&clip)?;
        root.AddVisual(&layer.visual, false, None)?;
        target.SetRoot(&root)?;
        Ok((target, root, fx, layer, clip))
      })();
      let (target, root, fx, layer, clip) = match made {
        Ok(v) => v,
        Err(err) => {
          let _ = DestroyWindow(hwnd);
          return Err(err.into());
        }
      };
      Ok(Self {
        hwnd,
        scale,
        motion,
        _target: target,
        root,
        fx,
        layer,
        clip,
        size: (1, 1),
        off: Animated::new(0.0),
        bottom: Animated::new(0.0),
        opacity: Animated::new(1.0),
        shown: false,
        closing: false,
        origin: (0, 0),
        hits: Vec::new(),
      })
    }
  }

  /// Makes the surface `w` x `h` DIPs (the whole window, shadow included).
  pub fn resize(&mut self, gfx: &Gfx, w: f32, h: f32) -> Result<()> {
    let px = ((w * self.scale).ceil().max(1.0) as u32, (h * self.scale).ceil().max(1.0) as u32);
    if px != self.size {
      unsafe {
        self.layer.surface = gfx.surface(px.0, px.1)?;
        self.layer.visual.SetContent(&self.layer.surface)?;
        self.clip.SetLeft2(0.0)?;
        self.clip.SetTop2(0.0)?;
        self.clip.SetRight2(px.0 as f32)?;
        if !self.shown || self.closing || self.motion != Motion::Slide {
          self.clip.SetBottom2(px.1 as f32)?;
        } else {
          // open: the new height at once (content switched, not re-opened)
          self.bottom.set(px.1 as f32);
          self.clip.SetBottom2(px.1 as f32)?;
        }
      }
      self.size = px;
    }
    Ok(())
  }

  pub fn draw<F>(&self, f: F) -> Result<()>
  where
    F: FnOnce(&windows::Win32::Graphics::Direct2D::ID2D1DeviceContext) -> Result<()>,
  {
    gfx::draw_surface(&self.layer.surface, self.scale, f)
  }

  /// Places the window (screen pixels) and shows it; animates in unless it
  /// is already open (a content switch moves it without re-opening).
  pub fn show_at(&mut self, gfx: &Gfx, x: i32, y: i32) -> Result<()> {
    let reopen = !self.shown || self.closing;
    self.origin = (x, y);
    unsafe {
      if reopen {
        let s = self.scale;
        let h = self.size.1 as f32;
        match self.motion {
          Motion::Slide => {
            if !self.shown {
              self.off.set(-14.0 * s);
              self.bottom.set(PAD * s);
              self.opacity.set(0.0);
              self.root.SetOffsetY2(-14.0 * s)?;
              self.clip.SetBottom2(PAD * s)?;
              self.fx.SetOpacity2(0.0)?;
            }
            if let Some(a) = self.off.to(&gfx.dcomp, 0.0, 240.0, POP_IN)? {
              self.root.SetOffsetY(&a)?;
            }
            if let Some(a) = self.bottom.to(&gfx.dcomp, h, 240.0, POP_IN)? {
              self.clip.SetBottom(&a)?;
            }
            // `popDown`: fully opaque at 60 %
            if let Some(a) = self.opacity.to(&gfx.dcomp, 1.0, 144.0, POP_IN)? {
              self.fx.SetOpacity(&a)?;
            }
          }
          Motion::Fade => {
            if !self.shown {
              self.opacity.set(0.0);
              self.fx.SetOpacity2(0.0)?;
            }
            if let Some(a) = self.opacity.to(&gfx.dcomp, 1.0, 120.0, POP_IN)? {
              self.fx.SetOpacity(&a)?;
            }
          }
          Motion::None => {
            self.opacity.set(1.0);
            self.fx.SetOpacity2(1.0)?;
          }
        }
      }
      gfx.dcomp.Commit()?;
      let _ = SetWindowPos(
        self.hwnd,
        HWND_TOPMOST,
        x,
        y,
        self.size.0 as i32,
        self.size.1 as i32,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
      );
    }
    self.shown = true;
    self.closing = false;
    Ok(())
  }

  /// Starts closing; the caller hides the window when the animation is
  /// over (`close_ms`).
  pub fn close(&mut self, gfx: &Gfx) -> Result<()> {
    if !self.shown || self.closing {
      return Ok(());
    }
    self.closing = true;
    let s = self.scale;
    unsafe {
      match self.motion {
        Motion::Slide => {
          if let Some(a) = self.off.to(&gfx.dcomp, -12.0 * s, 170.0, POP_OUT)? {
            self.root.SetOffsetY(&a)?;
          }
          if let Some(a) = self.bottom.to(&gfx.dcomp, PAD * s, 170.0, POP_OUT)? {
            self.clip.SetBottom(&a)?;
          }
          if let Some(a) = self.opacity.to(&gfx.dcomp, 0.0, 170.0, POP_OUT)? {
            self.fx.SetOpacity(&a)?;
          }
        }
        Motion::Fade => {
          if let Some(a) = self.opacity.to(&gfx.dcomp, 0.0, 100.0, POP_OUT)? {
            self.fx.SetOpacity(&a)?;
          }
        }
        Motion::None => {}
      }
      gfx.dcomp.Commit()?;
    }
    Ok(())
  }

  pub fn close_ms(&self) -> u32 {
    match self.motion {
      Motion::Slide => 180,
      Motion::Fade => 110,
      Motion::None => 0,
    }
  }

  pub fn hide(&mut self) {
    unsafe {
      let _ = ShowWindow(self.hwnd, SW_HIDE);
    }
    self.shown = false;
    self.closing = false;
    self.hits.clear();
  }

  pub fn contains_screen(&self, x: i32, y: i32) -> bool {
    self.shown
      && !self.closing
      && x >= self.origin.0
      && y >= self.origin.1
      && x < self.origin.0 + self.size.0 as i32
      && y < self.origin.1 + self.size.1 as i32
  }
}

impl Drop for PopWin {
  fn drop(&mut self) {
    unsafe {
      let _ = DestroyWindow(self.hwnd);
    }
  }
}

// ------------------------------------------------------------------ drawing

/// The popup box (styles.css `.res-pop`, `.media-pop`, `.tray-popup`):
/// `box-shadow: 0 4px 14px rgba(0 0 0 / 35%)`, `colLayer0`, 1 px border.
fn frame_box(p: &mut Painter, t: &Theme, r: Rect, radius: f32) -> anyhow::Result<()> {
  for k in 1..=7 {
    let g = k as f32 * 1.8;
    p.fill_round(Rect::new(r.x - g, r.y + 4.0 - g, r.w + 2.0 * g, r.h + 2.0 * g), radius + g, Rgba(0, 0, 0, 0.045))?;
  }
  p.fill_round(r, radius, t.layer0)?;
  Ok(())
}

fn frame_border(p: &mut Painter, t: &Theme, r: Rect, radius: f32) -> anyhow::Result<()> {
  p.stroke_round(r, radius, t.border, 1.0)?;
  Ok(())
}

// ---- resources (ii ResourcesPopup.qml + our temperatures)

struct Row {
  icon: &'static str,
  label: String,
  value: String,
  hot: bool,
}

struct Col {
  icon: &'static str,
  head: String,
  rows: Vec<Row>,
}

fn gb(b: u64) -> String {
  format!("{:.1} GB", b as f64 / 1_073_741_824.0)
}

fn deg(v: Option<f32>, hot: f32) -> (String, bool) {
  match v {
    Some(v) => (format!("{}°C", v.round() as i32), v >= hot),
    None => ("—".into(), false),
  }
}

fn res_cols(m: &Model, temps: Option<&Temps>) -> Vec<Col> {
  let row = |icon, label: &str, value: String, hot| Row { icon, label: format!("{}:", m.tr(label)), value, hot };
  let mut cols = Vec::new();
  let mem = m.memory.as_ref();
  cols.push(Col {
    icon: "memory",
    head: "RAM".into(),
    rows: vec![
      row("clock_loader_60", "Kullanılan", mem.map_or("—".into(), |m| gb(m.used_memory)), false),
      row("check_circle", "Boş", mem.map_or("—".into(), |m| gb(m.free_memory)), false),
      row("empty_dashboard", "Toplam", mem.map_or("—".into(), |m| gb(m.total_memory)), false),
    ],
  });
  if let Some(mem) = mem.filter(|m| m.total_swap > 0) {
    cols.push(Col {
      icon: "swap_horiz",
      head: "Swap".into(),
      rows: vec![
        row("clock_loader_60", "Kullanılan", gb(mem.used_swap), false),
        row("check_circle", "Boş", gb(mem.free_swap), false),
        row("empty_dashboard", "Toplam", gb(mem.total_swap), false),
      ],
    });
  }
  let cpu = m.cpu.as_ref();
  let mut cpu_rows = vec![row("bolt", "Yük", cpu.map_or("—".into(), |c| format!("{}%", c.usage.round() as i32)), false)];
  if let Some(c) = cpu.filter(|c| c.frequency > 0) {
    cpu_rows.push(row("speed", "Frekans", format!("{:.2} GHz", c.frequency as f64 / 1000.0), false));
  }
  cols.push(Col { icon: "planner_review", head: "CPU".into(), rows: cpu_rows });
  let t = temps.copied().unwrap_or_default();
  let (cv, ch) = deg(t.cpu, 85.0);
  let (gv, gh) = deg(t.gpu, 80.0);
  let mut rows = vec![row("planner_review", "CPU", cv, ch), row("developer_board", "GPU", gv, gh)];
  if t.gpu_hot.is_some() {
    let (hv, hh) = deg(t.gpu_hot, 95.0);
    rows.push(row("local_fire_department", "Sıcak nokta", hv, hh));
  }
  if let Some(l) = t.gpu_load {
    rows.push(row("bolt", "GPU yükü", format!("{}%", l.round() as i32), false));
  }
  cols.push(Col { icon: "device_thermostat", head: m.tr("Sıcaklık"), rows });
  cols
}

const HEAD: TextStyle = style(13.0);
const ROW: TextStyle = style(12.0);
const HOT: TextStyle = TextStyle { size: 12.0, weight: 600.0 };
const HEAD_H: f32 = 17.0;
const ROW_H: f32 = 16.0;

fn col_width(p: &mut Painter, c: &Col) -> anyhow::Result<f32> {
  let mut w = 15.0 + 5.0 + p.measure(&c.head, HEAD)?;
  for r in &c.rows {
    let vw = p.measure_with(&r.value, if r.hot { HOT } else { ROW }, true)?;
    w = w.max(13.0 + 4.0 + p.measure(&r.label, ROW)? + 4.0 + vw);
  }
  Ok(w.ceil())
}

/// Box size (DIPs, without `PAD`).
pub fn res_size(p: &mut Painter, m: &Model, temps: Option<&Temps>) -> anyhow::Result<(f32, f32)> {
  let cols = res_cols(m, temps);
  let mut w = 28.0 + 18.0 * (cols.len() as f32 - 1.0);
  let mut rows = 0;
  for c in &cols {
    w += col_width(p, c)?;
    rows = rows.max(c.rows.len());
  }
  Ok((w, 20.0 + HEAD_H + 3.0 + rows as f32 * (5.0 + ROW_H)))
}

pub fn paint_res(p: &mut Painter, m: &Model, t: &Theme, temps: Option<&Temps>, size: (f32, f32)) -> anyhow::Result<()> {
  let r = Rect::new(PAD, PAD, size.0, size.1);
  frame_box(p, t, r, 12.0)?;
  let mut x = r.x + 14.0;
  for c in res_cols(m, temps) {
    let cw = col_width(p, &c)?;
    let mut y = r.y + 10.0;
    p.icon(c.icon, x + 7.5, y + HEAD_H / 2.0, 15.0, false, t.subtext)?;
    p.text(&c.head, Rect::new(x + 20.0, y, cw - 20.0 + 1.0, HEAD_H), HEAD, t.on_layer0, Align::Left, false)?;
    y += HEAD_H + 3.0;
    for row in &c.rows {
      y += 5.0;
      p.icon(row.icon, x + 6.5, y + ROW_H / 2.0, 13.0, false, t.subtext)?;
      let lx = x + 17.0;
      let lw = p.text(&row.label, Rect::new(lx, y, cw, ROW_H), ROW, t.on_layer1, Align::Left, false)?;
      let (st, color) = if row.hot { (HOT, t.error) } else { (ROW, t.on_layer1) };
      p.text(&row.value, Rect::new(lx + lw + 4.0, y, cw, ROW_H), st, color, Align::Left, true)?;
      y += ROW_H;
    }
    x += cw + 18.0;
  }
  frame_border(p, t, r, 12.0)?;
  Ok(())
}

// ---- media (ii media popup)

pub const MEDIA_W: f32 = 360.0;
pub const MEDIA_H: f32 = 110.0;

pub fn fmt_time(secs: f64) -> String {
  let s = if secs.is_finite() && secs > 0.0 { secs as u64 } else { 0 };
  format!("{}:{:02}", s / 60, s % 60)
}

pub struct MediaView<'a> {
  pub art: Option<&'a ID2D1Bitmap1>,
  /// the art blurred and darkened, `MEDIA_W` x `MEDIA_H` (`.mp-bg`)
  pub bg: Option<&'a ID2D1Bitmap1>,
  /// position now (seconds), end (seconds), playing
  pub pos: f64,
  pub end: f64,
  pub playing: bool,
  pub hover: Option<&'a PopHit>,
}

pub fn paint_media(p: &mut Painter, m: &Model, t: &Theme, v: &MediaView) -> anyhow::Result<Vec<Hit>> {
  let mut hits = Vec::new();
  let r = Rect::new(PAD, PAD, MEDIA_W, MEDIA_H);
  frame_box(p, t, r, 12.0)?;
  let session = m.media.as_ref().and_then(|x| x.current_session.as_ref());
  let title = session.and_then(|s| s.title.clone()).filter(|s| !s.is_empty());
  let Some(title) = title else {
    p.text(&m.tr("Medya yok"), Rect::new(r.x + 10.0, r.y + 10.0, r.w - 20.0, 19.0), style(15.0), t.on_layer0, Align::Left, false)?;
    frame_border(p, t, r, 12.0)?;
    return Ok(hits);
  };
  if let Some(bg) = v.bg {
    // 96 DPI bitmap of the popup's pixel size: drawn 1:1 into the box
    let size = unsafe { bg.GetSize() };
    let img: ID2D1Image = bg.cast()?;
    p.image_round(&img, size.width, size.height, r, 12.0, 1.0)?;
  }
  // art 90 x 90, radius 12
  let art = Rect::new(r.x + 10.0, r.y + 10.0, 90.0, 90.0);
  match v.art {
    Some(bmp) => {
      let size = unsafe { bmp.GetSize() };
      let img: ID2D1Image = bmp.cast()?;
      p.image_round(&img, size.width, size.height, art, 12.0, 1.0)?;
    }
    None => {
      p.fill_round(art, 12.0, t.sec_container)?;
      p.icon("music_note", art.x + 45.0, art.y + 45.0, 36.0, true, t.on_sec_container)?;
    }
  }
  let white = |a: f32| Rgba(255, 255, 255, a);
  // play button: 36 x 36 on the right, vertically centred
  let play = Rect::new(r.right() - 10.0 - 36.0, r.y + (MEDIA_H - 36.0) / 2.0, 36.0, 36.0);
  let play_hot = v.hover == Some(&PopHit::Play);
  p.fill_round(play, 18.0, white(if play_hot { 0.20 } else { 0.12 }))?;
  p.icon(if v.playing { "pause" } else { "play_arrow" }, play.x + 18.0, play.y + 18.0, 22.0, true, t.on_layer0)?;
  hits.push(Hit { rect: play, kind: PopHit::Play });

  let ix = art.right() + 12.0;
  let iw = play.x - 12.0 - ix;
  p.text(&title, Rect::new(ix, r.y + 10.0, iw, 19.0), style(15.0), t.on_layer0, Align::Left, false)?;
  let artist = session.and_then(|s| s.artist.clone()).unwrap_or_default();
  p.text(&artist, Rect::new(ix, r.y + 31.0, iw, 14.0), style(11.0), t.subtext, Align::Left, false)?;
  let time = format!("{} / {}", fmt_time(v.pos), fmt_time(v.end));
  p.text(&time, Rect::new(ix, r.y + 55.0, iw, 16.0), style(12.0), t.subtext, Align::Left, true)?;

  // controls: prev, progress, next (bottom of the info column)
  let cy = r.bottom() - 10.0 - 11.0;
  let prev = Rect::new(ix, cy - 11.0, 22.0, 22.0);
  let next = Rect::new(ix + iw - 22.0, cy - 11.0, 22.0, 22.0);
  for (rect, kind, icon) in [(prev, PopHit::Prev, "skip_previous"), (next, PopHit::Next, "skip_next")] {
    if v.hover == Some(&kind) {
      p.fill_round(rect, 11.0, white(0.10))?;
    }
    p.icon(icon, rect.x + 11.0, rect.y + 11.0, 18.0, true, t.on_layer0)?;
    hits.push(Hit { rect, kind });
  }
  let bar = Rect::new(prev.right() + 6.0, cy - 1.5, next.x - 6.0 - prev.right() - 6.0, 3.0);
  let frac = if v.end > 0.0 { (v.pos / v.end).clamp(0.0, 1.0) as f32 } else { 0.0 };
  p.fill_round(bar, 1.5, white(0.20))?;
  p.fill_round(Rect::new(bar.x, bar.y, bar.w * frac, bar.h), 1.5, t.primary)?;
  p.fill_round(Rect::new(bar.x + bar.w * frac - 1.5, cy - 7.0, 3.0, 14.0), 1.5, t.primary)?;
  // `.mp-bar::before`: a taller area to click
  hits.push(Hit { rect: Rect::new(bar.x, bar.y - 7.0, bar.w, bar.h + 14.0), kind: PopHit::Seek(bar) });
  frame_border(p, t, r, 12.0)?;
  Ok(hits)
}

/// `.mp-bg`: the art covering the box plus 20 px on each side, blurred
/// 24 px, darkened (brightness .35, saturate 1.4), opacity .9. Rendered once
/// per song (the popup ticks every second; the blur is not redone).
pub fn blur_art(gfx: &Gfx, art: &ID2D1Bitmap1, scale: f32) -> Result<ID2D1Bitmap1> {
  unsafe {
    let dc = &gfx.dc;
    let (w, h) = ((MEDIA_W * scale).round() as u32, (MEDIA_H * scale).round() as u32);
    let props = D2D1_BITMAP_PROPERTIES1 {
      pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
      dpiX: 96.0,
      dpiY: 96.0,
      bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
      colorContext: ManuallyDrop::new(None),
    };
    let target = dc.CreateBitmap(D2D_SIZE_U { width: w.max(1), height: h.max(1) }, None, 0, &props)?;
    let prev = dc.GetTarget().ok();
    dc.SetTarget(&target);
    dc.SetDpi(96.0, 96.0);
    dc.BeginDraw();
    dc.Clear(Some(&Rgba(0, 0, 0, 0.0).into()));

    // cover (w + 40s) x (h + 40s), centred
    let size = art.GetSize();
    let (bw, bh) = (size.width.max(1.0), size.height.max(1.0));
    let (cw, ch) = (w as f32 + 40.0 * scale, h as f32 + 40.0 * scale);
    let k = (cw / bw).max(ch / bh);
    let scaled = dc.CreateEffect(&windows::Win32::Graphics::Direct2D::CLSID_D2D12DAffineTransform)?;
    scaled.SetInput(0, art, true);
    let m = windows::Foundation::Numerics::Matrix3x2 {
      M11: k,
      M12: 0.0,
      M21: 0.0,
      M22: k,
      M31: (w as f32 - bw * k) / 2.0,
      M32: (h as f32 - bh * k) / 2.0,
    };
    let mut mb = Vec::with_capacity(24);
    for v in [m.M11, m.M12, m.M21, m.M22, m.M31, m.M32] {
      mb.extend_from_slice(&v.to_le_bytes());
    }
    scaled.SetValue(
      windows::Win32::Graphics::Direct2D::D2D1_2DAFFINETRANSFORM_PROP_TRANSFORM_MATRIX.0 as u32,
      windows::Win32::Graphics::Direct2D::D2D1_PROPERTY_TYPE_MATRIX_3X2,
      &mb,
    )?;

    let blur = dc.CreateEffect(&CLSID_D2D1GaussianBlur)?;
    blur.SetInput(0, &scaled.GetOutput()?, true);
    // CSS blur(24px): the standard deviation, in the target's pixels
    blur.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &(24.0 * scale).to_le_bytes())?;
    blur.SetValue(D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &D2D1_BORDER_MODE_HARD.0.to_le_bytes())?;

    // brightness(.35) after saturate(1.4), as a 5x4 colour matrix
    let s = 1.4f32;
    let (lr, lg, lb) = (0.2126f32, 0.7152f32, 0.0722f32);
    let b = 0.35f32;
    let row = |r: f32, g: f32, bb: f32| [r * b, g * b, bb * b];
    let rr = row(lr * (1.0 - s) + s, lg * (1.0 - s), lb * (1.0 - s));
    let gg = row(lr * (1.0 - s), lg * (1.0 - s) + s, lb * (1.0 - s));
    let bbb = row(lr * (1.0 - s), lg * (1.0 - s), lb * (1.0 - s) + s);
    // D2D matrix is column-major per output: m[input][output]
    let matrix: [f32; 20] = [
      rr[0], gg[0], bbb[0], 0.0,
      rr[1], gg[1], bbb[1], 0.0,
      rr[2], gg[2], bbb[2], 0.0,
      0.0, 0.0, 0.0, 0.9,
      0.0, 0.0, 0.0, 0.0,
    ];
    let mut cb = Vec::with_capacity(80);
    for v in matrix {
      cb.extend_from_slice(&v.to_le_bytes());
    }
    let color = dc.CreateEffect(&CLSID_D2D1ColorMatrix)?;
    color.SetInput(0, &blur.GetOutput()?, true);
    color.SetValue(D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32, D2D1_PROPERTY_TYPE_MATRIX_5X4, &cb)?;
    let out = color.GetOutput()?;
    dc.DrawImage(&out, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
    let end = dc.EndDraw(None, None);
    dc.SetTarget(prev.as_ref());
    end?;
    Ok(target)
  }
}

// ---- tray panel (ii SysTray.qml overflow)

pub const TRAY_COLS: usize = 6;

pub fn tray_size(n: usize) -> (f32, f32) {
  let rows = n.div_ceil(TRAY_COLS).max(1);
  (TRAY_COLS as f32 * 30.0 + (TRAY_COLS as f32 - 1.0) * 2.0 + 18.0, rows as f32 * 32.0 + 18.0)
}

pub struct TrayView<'a> {
  pub hover: Option<&'a str>,
  /// a dragged icon is over the panel
  pub drop_zone: bool,
  /// pin key of the icon being dragged (drawn faint)
  pub dragging: Option<&'a str>,
}

pub fn paint_tray(p: &mut Painter, m: &Model, t: &Theme, v: &TrayView) -> anyhow::Result<Vec<Hit>> {
  let mut hits = Vec::new();
  let icons = m.unpinned_icons();
  let size = tray_size(icons.len());
  let r = Rect::new(PAD, PAD, size.0, size.1);
  frame_box(p, t, r, 17.0)?;
  if icons.is_empty() {
    p.text(&m.tr("Buraya bırak"), Rect::new(r.x, r.y, r.w, r.h), style(12.0), t.subtext, Align::Center, false)?;
  }
  for (i, ic) in icons.iter().enumerate() {
    let (col, row) = (i % TRAY_COLS, i / TRAY_COLS);
    let cell = Rect::new(r.x + 9.0 + col as f32 * 32.0, r.y + 9.0 + row as f32 * 32.0, 30.0, 30.0);
    if v.hover == Some(ic.id.as_str()) {
      p.fill_round(cell, 15.0, t.layer1_hover)?;
    }
    if let Some(bmp) = p.icons.tray(p.gfx, &ic.icon_hash, &ic.icon_bytes) {
      // `.tray-item.dragging { opacity: 0.35 }`
      let faint = v.dragging == Some(super::model::pin_key(ic).as_str());
      let size = unsafe { bmp.GetSize() };
      let img: ID2D1Image = bmp.cast()?;
      p.image_round(&img, size.width, size.height, Rect::new(cell.x + 6.0, cell.y + 6.0, 18.0, 18.0), 0.0, if faint { 0.35 } else { 1.0 })?;
    }
    hits.push(Hit { rect: cell, kind: PopHit::TrayIcon(ic.id.clone()) });
  }
  if v.drop_zone {
    p.stroke_round(r.inset(4.0, 4.0), 13.0, t.primary, 1.0)?;
  }
  frame_border(p, t, r, 17.0)?;
  Ok(hits)
}

// ---- tooltip (ii StyledToolTip)

const TIP: TextStyle = style(12.0);

pub fn tip_size(p: &mut Painter, text: &str) -> anyhow::Result<(f32, f32)> {
  Ok((p.measure(text, TIP)?.min(360.0).ceil() + 18.0, 24.0))
}

pub fn paint_tip(p: &mut Painter, t: &Theme, text: &str, size: (f32, f32)) -> anyhow::Result<()> {
  let r = Rect::new(PAD, PAD, size.0, size.1);
  for k in 1..=4 {
    let g = k as f32 * 1.5;
    p.fill_round(Rect::new(r.x - g, r.y + 2.0 - g, r.w + 2.0 * g, r.h + 2.0 * g), 7.0 + g, Rgba(0, 0, 0, 0.05))?;
  }
  p.fill_round(r, 7.0, t.tip_bg)?;
  p.text(text, Rect::new(r.x + 9.0, r.y + 4.0, r.w - 18.0, r.h - 8.0), TIP, t.tip_fg, Align::Left, false)?;
  Ok(())
}

// ---- drag ghost (`.tray-ghost`: 18 px with a drop shadow)

pub const GHOST: f32 = 18.0;

pub fn paint_ghost(p: &mut Painter, bmp: Option<&ID2D1Bitmap1>) -> anyhow::Result<()> {
  let r = Rect::new(PAD, PAD, GHOST, GHOST);
  p.fill_circle(r.x + GHOST / 2.0, r.y + GHOST / 2.0 + 2.0, GHOST / 2.0 + 2.0, Rgba(0, 0, 0, 0.25))?;
  if let Some(bmp) = bmp {
    p.image(bmp, r);
  }
  Ok(())
}
