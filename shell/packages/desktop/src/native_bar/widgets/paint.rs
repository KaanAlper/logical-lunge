//! Drawing the desktop widgets: a card in the theme's colours and each
//! kind's contents. Returns the clickable parts.

use windows::{
  core::Interface,
  Win32::Graphics::{
    Direct2D::{ID2D1Bitmap1, ID2D1Image, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT},
    DirectWrite::{IDWriteTextLayout, DWRITE_HIT_TEST_METRICS},
  },
};

use super::{
  layout::{ClockStyle, Kind, Spec},
  weather::{self, Report},
};
use super::super::{
  fonts::TextStyle,
  gfx::{pt, Rect, Rgba},
  view::{Align, Painter, Theme},
};

pub const RADIUS: f32 = 24.0;
/// the corner a press resizes from
pub const GRIP: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
  Prev,
  Play,
  Next,
  /// the note's text (editing)
  Text,
  Grip,
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
  // A dense material keeps text readable over bright or detailed wallpapers.
  // The inset edge separates the widget without reserving extra screen space.
  p.fill_round(card, RADIUS, Rgba(0, 0, 0, 0.18))?;
  p.fill_round(card.inset(1.0, 1.0), RADIUS - 1.0, t.layer0.alpha(0.97))?;
  p.stroke_round(card.inset(1.0, 1.0), RADIUS - 1.0, t.on_layer0.alpha(if hover { 0.20 } else { 0.10 }), 1.0)?;
  if s.kind == Kind::Note {
    p.fill_round(Rect::new(8.0, 18.0, 3.0, s.h - 36.0), 1.5, t.primary.alpha(0.6))?;
  }
  let inner = card.inset(16.0, 14.0);
  let mut hits = Vec::new();
  match s.kind {
    Kind::Clock => clock(p, t, s, d, inner)?,
    Kind::Media => media(p, t, d, inner, &mut hits)?,
    Kind::System => system(p, t, s, d, inner)?,
    Kind::Weather => weather_card(p, t, d, inner)?,
    Kind::Agenda => agenda(p, t, d, inner)?,
    Kind::Note => note(p, t, s, d, inner, editing, note_layout, &mut hits)?,
  }
  let grip = Rect::new(s.w - GRIP - 4.0, s.h - GRIP - 4.0, GRIP, GRIP);
  if hover {
    // three dots along the corner
    for (dx, dy) in [(12.0, 4.0), (8.0, 8.0), (4.0, 12.0), (12.0, 8.0), (8.0, 12.0), (12.0, 12.0)] {
      p.fill_circle(grip.x + dx, grip.y + dy, 1.3, t.on_surface_variant.alpha(0.7))?;
    }
  }
  hits.push((grip, Hit::Grip));
  Ok(hits)
}

fn clock(p: &mut Painter, t: &Theme, s: &Spec, d: &Data, r: Rect) -> anyhow::Result<()> {
  let c = &d.clock;
  match s.clock {
    ClockStyle::Analog => {
      let size = r.w.min(r.h);
      let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
      let rad = size / 2.0;
      p.fill_circle(cx, cy, rad, t.surface_container)?;
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
      let max = if large { r.h - date_h } else { (r.h - date_h) * 0.8 };
      // the time fits the width too (a narrow widget)
      let mut size = max.max(12.0);
      let probe = TextStyle { size, weight: if large { 300.0 } else { 400.0 } };
      let w = p.measure_with(&c.time, probe, true)?;
      if w > r.w {
        size *= r.w / w;
      }
      let style_t = TextStyle { size, weight: probe.weight };
      let block = size * 1.25 + date_h;
      let top = r.y + (r.h - block) / 2.0;
      p.text(&c.time, Rect::new(r.x, top, r.w, size * 1.25), style_t, t.on_layer0, Align::Center, true)?;
      if s.date {
        p.text(&c.date, Rect::new(r.x, top + size * 1.25, r.w, date_h), SMALL, t.on_surface_variant, Align::Center, false)?;
      }
    }
  }
  if s.clock == ClockStyle::Analog && s.date && r.h > r.w + 24.0 {
    p.text(&c.date, Rect::new(r.x, r.bottom() - 18.0, r.w, 18.0), SMALL, t.on_surface_variant, Align::Center, false)?;
  }
  Ok(())
}

fn button(p: &mut Painter, t: &Theme, r: Rect, icon: &str, primary: bool) -> anyhow::Result<()> {
  if primary {
    p.fill_circle(r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, t.primary)?;
  }
  p.icon(icon, r.x + r.w / 2.0, r.y + r.h / 2.0, 22.0, true, if primary { t.on_primary } else { t.on_layer0 })
}

fn media(p: &mut Painter, t: &Theme, d: &Data, r: Rect, hits: &mut Vec<(Rect, Hit)>) -> anyhow::Result<()> {
  let Some(m) = &d.media else {
    p.icon("music_off", r.x + 18.0, r.y + r.h / 2.0, 26.0, false, t.on_surface_variant)?;
    p.text(&(d.tr)("Çalan bir şey yok"), Rect::new(r.x + 42.0, r.y, r.w - 42.0, r.h), BODY, t.on_surface_variant, Align::Left, false)?;
    return Ok(());
  };
  let side = r.h.min(r.w * 0.32);
  let cover = Rect::new(r.x, r.y, side, side);
  match m.art {
    Some(bmp) => {
      let size = unsafe { bmp.GetSize() };
      let img: ID2D1Image = bmp.cast()?;
      p.image_round(&img, size.width, size.height, cover, 16.0, 1.0)?;
    }
    None => {
      p.fill_round(cover, 16.0, t.primary_container)?;
      p.icon("music_note", cover.x + side / 2.0, cover.y + side / 2.0, side * 0.4, false, t.on_primary_container)?;
    }
  }
  let x = cover.right() + 14.0;
  let w = (r.right() - x).max(10.0);
  p.text(&m.title, Rect::new(x, r.y, w, 20.0), TextStyle { size: 15.0, weight: 600.0 }, t.on_layer0, Align::Left, false)?;
  p.text(&m.artist, Rect::new(x, r.y + 20.0, w, 18.0), SMALL, t.on_surface_variant, Align::Left, false)?;
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
    let y = top + row_h * i as f32;
    let frac = value.unwrap_or(0.0).clamp(0.0, 100.0) / 100.0;
    let pct = value.map_or("–".to_string(), |v| format!("{}%", v.round() as i32));
    p.text(label, Rect::new(r.x, y, 40.0, 18.0), SMALL, t.on_layer0, Align::Left, false)?;
    if let Some(c) = temp {
      p.text(&format!("{}°", c.round() as i32), Rect::new(r.x + 42.0, y, 40.0, 18.0), SMALL, t.on_surface_variant, Align::Left, true)?;
    }
    p.text(&pct, Rect::new(r.right() - 48.0, y, 48.0, 18.0), TextStyle { size: 13.0, weight: 600.0 }, t.on_layer0, Align::Center, true)?;
    let track = Rect::new(r.x, y + row_h - 6.0, r.w, 4.0);
    p.fill_round(track, 2.0, t.on_layer0.alpha(0.10))?;
    if value.is_some() && frac > 0.0 {
      p.fill_round(Rect::new(track.x, track.y, track.w * frac, track.h), 2.0, t.primary)?;
    }
  }
  Ok(())
}

fn weather_card(p: &mut Painter, t: &Theme, d: &Data, r: Rect) -> anyhow::Result<()> {
  match d.weather {
    Some(Ok(w)) => {
      let (icon, text) = weather::describe(w.code, w.day);
      let big = (r.h - 44.0).clamp(24.0, 52.0);
      p.icon(icon, r.x + big / 2.0, r.y + big / 2.0 + 4.0, big, true, t.primary)?;
      let unit = if w.fahrenheit { "°F" } else { "°" };
      let temp = format!("{}{unit}", w.temp.round() as i32);
      p.text(&temp, Rect::new(r.x + big + 12.0, r.y, r.w - big - 12.0, big + 4.0), TextStyle { size: big * 0.8, weight: 400.0 }, t.on_layer0, Align::Left, true)?;
      let line = if w.high.is_finite() && w.low.is_finite() {
        format!("{} · ↑{}° ↓{}°", (d.tr)(text), w.high.round() as i32, w.low.round() as i32)
      } else {
        (d.tr)(text)
      };
      p.text(&line, Rect::new(r.x, r.bottom() - 40.0, r.w, 20.0), BODY, t.on_layer0, Align::Left, false)?;
      p.text(&w.place, Rect::new(r.x, r.bottom() - 20.0, r.w, 20.0), SMALL, t.on_surface_variant, Align::Left, false)?;
    }
    Some(Err(_)) => {
      p.icon("cloud_off", r.x + 16.0, r.y + r.h / 2.0, 26.0, false, t.on_surface_variant)?;
      p.text(&(d.tr)("Hava durumu alınamadı"), Rect::new(r.x + 40.0, r.y, r.w - 40.0, r.h), BODY, t.on_surface_variant, Align::Left, false)?;
    }
    None => {
      p.icon("partly_cloudy_day", r.x + 16.0, r.y + r.h / 2.0, 26.0, false, t.on_surface_variant)?;
      p.text(&(d.tr)("Yükleniyor…"), Rect::new(r.x + 40.0, r.y, r.w - 40.0, r.h), BODY, t.on_surface_variant, Align::Left, false)?;
    }
  }
  Ok(())
}

fn agenda(p: &mut Painter, t: &Theme, d: &Data, r: Rect) -> anyhow::Result<()> {
  p.fill_round(Rect::new(r.x, r.y, 52.0, 52.0), 16.0, t.primary_container)?;
  p.text(&d.day_big, Rect::new(r.x, r.y, 52.0, 52.0), TextStyle { size: 32.0, weight: 500.0 }, t.on_primary_container, Align::Center, true)?;
  p.text(&d.day_line, Rect::new(r.x + 66.0, r.y + 6.0, r.w - 66.0, 40.0), BODY, t.on_layer0, Align::Left, false)?;
  p.fill(Rect::new(r.x, r.y + 66.0, r.w, 1.0), t.on_layer0.alpha(0.10))?;
  let mut y = r.y + 78.0;
  if d.todos.is_empty() {
    p.text(&(d.tr)("Yapılacak yok"), Rect::new(r.x, y, r.w, 20.0), SMALL, t.on_surface_variant, Align::Left, false)?;
    return Ok(());
  }
  for todo in d.todos {
    if y + 22.0 > r.bottom() {
      break;
    }
    p.icon("radio_button_unchecked", r.x + 8.0, y + 11.0, 14.0, false, t.primary)?;
    p.text(todo, Rect::new(r.x + 26.0, y, r.w - 26.0, 22.0), BODY, t.on_layer0, Align::Left, false)?;
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
    p.text(&(d.tr)("Not yazmak için tıkla"), Rect::new(r.x, r.y, r.w, 20.0), BODY, t.on_surface_variant, Align::Left, false)?;
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
    Ok(())
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
