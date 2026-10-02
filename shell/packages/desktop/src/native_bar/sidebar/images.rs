//! Pictures of the right panel: wallpaper and live wallpaper thumbnails (from
//! the web through the Windows cache, or library files), the store's moving
//! previews (GIF), screen saver and notification icons (data URLs).
//!
//! Decoding and scaling run on a worker thread with its own WIC factory;
//! the UI thread only turns the pixels into Direct2D bitmaps.

use std::{
  collections::{HashMap, VecDeque},
  sync::{Condvar, Mutex},
  time::Instant,
};

use windows::{
  core::{HSTRING, PCWSTR},
  Win32::{
    Graphics::{
      Direct2D::{Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U}, ID2D1Bitmap1, D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1},
      Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
      Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapDecoder, IWICBitmapSource, IWICImagingFactory,
        WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
      },
    },
    System::Com::{CoCreateInstance, CoInitializeEx, Urlmon::URLDownloadToCacheFileW, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
  },
};

use super::super::{gfx::Gfx, icons::data_url_bytes, send, Msg};
use super::Ev;

/// Decoded pixels (premultiplied BGRA, rows of `w * 4` bytes).
pub(in crate::native_bar) struct Pixels {
  pub w: u32,
  pub h: u32,
  pub data: Vec<u8>,
}

pub(super) enum Img {
  Loading,
  Failed,
  Still(ID2D1Bitmap1),
  /// frames with their delays (ms)
  Moving(Vec<(ID2D1Bitmap1, u32)>),
}

/// Bitmaps by key (a URL, a file path, or a data URL's hash), newest last.
#[derive(Default)]
pub(super) struct Images {
  map: HashMap<String, Img>,
  order: VecDeque<String>,
}

/// pictures kept (thumbnails are small; a gallery shows a few dozen)
const KEEP: usize = 160;

impl Images {
  /// The bitmap for `key`, asked for once when it is missing.
  pub fn get(&mut self, key: &str, max_w: u32, max_h: u32) -> Option<&Img> {
    if !self.map.contains_key(key) {
      self.map.insert(key.to_string(), Img::Loading);
      self.order.push_back(key.to_string());
      let k = key.to_string();
      std::thread::spawn(move || {
        let _slot = Slot::take();
        let px = load(&k, max_w, max_h);
        send(Msg::Sidebar(Ev::Image(k, px.map(|p| vec![(p, 0)]))));
      });
      self.trim();
    }
    self.map.get(key)
  }

  /// A moving preview (all frames), asked for once.
  pub fn get_moving(&mut self, key: &str, max_w: u32, max_h: u32) -> Option<&Img> {
    let k = format!("gif:{key}");
    if !self.map.contains_key(&k) {
      self.map.insert(k.clone(), Img::Loading);
      self.order.push_back(k.clone());
      let url = key.to_string();
      std::thread::spawn(move || {
        let _slot = Slot::take();
        let frames = load_frames(&url, max_w, max_h);
        send(Msg::Sidebar(Ev::Image(k, frames)));
      });
      self.trim();
    }
    self.map.get(&format!("gif:{key}"))
  }

  pub fn forget(&mut self, key: &str) {
    self.map.remove(key);
    self.order.retain(|k| k != key);
  }

  fn trim(&mut self) {
    while self.order.len() > KEEP {
      if let Some(old) = self.order.pop_front() {
        self.map.remove(&old);
      }
    }
  }

  /// A worker's result: bitmaps on the UI thread's device.
  pub fn arrived(&mut self, gfx: &Gfx, key: String, frames: Option<Vec<(Pixels, u32)>>) {
    if !self.map.contains_key(&key) {
      return; // forgotten meanwhile
    }
    let img = match frames {
      Some(f) if !f.is_empty() => {
        let mut out = Vec::new();
        for (px, delay) in f {
          if let Some(b) = bitmap(gfx, &px) {
            out.push((b, delay));
          }
        }
        match out.len() {
          0 => Img::Failed,
          1 if !key.starts_with("gif:") => Img::Still(out.remove(0).0),
          _ => Img::Moving(out),
        }
      }
      _ => Img::Failed,
    };
    self.map.insert(key, img);
  }

  /// The device went away: every bitmap goes with it.
  pub fn clear(&mut self) {
    self.map.clear();
    self.order.clear();
  }
}

/// The frame of a moving picture to show now (frames loop).
pub(super) fn frame_now(frames: &[(ID2D1Bitmap1, u32)], since: Instant) -> Option<&ID2D1Bitmap1> {
  let total: u32 = frames.iter().map(|(_, d)| (*d).max(20)).sum();
  if total == 0 {
    return frames.first().map(|(b, _)| b);
  }
  let mut t = (since.elapsed().as_millis() % total as u128) as u32;
  for (b, d) in frames {
    let d = (*d).max(20);
    if t < d {
      return Some(b);
    }
    t -= d;
  }
  frames.last().map(|(b, _)| b)
}

fn bitmap(gfx: &Gfx, px: &Pixels) -> Option<ID2D1Bitmap1> {
  let props = D2D1_BITMAP_PROPERTIES1 {
    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
    dpiX: 96.0,
    dpiY: 96.0,
    bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
    colorContext: std::mem::ManuallyDrop::new(None),
  };
  unsafe {
    gfx
      .dc
      .CreateBitmap(D2D_SIZE_U { width: px.w, height: px.h }, Some(px.data.as_ptr().cast()), px.w * 4, &props)
      .ok()
  }
}

// ------------------------------------------------------------ worker side

/// Pictures decoded at once (a store category asks for a hundred covers;
/// the rest wait their turn).
const AT_ONCE: usize = 6;
static RUNNING: Mutex<usize> = Mutex::new(0);
static FREED: Condvar = Condvar::new();

struct Slot;

impl Slot {
  fn take() -> Slot {
    let mut n = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    while *n >= AT_ONCE {
      n = FREED.wait(n).unwrap_or_else(|e| e.into_inner());
    }
    *n += 1;
    Slot
  }
}

impl Drop for Slot {
  fn drop(&mut self) {
    let mut n = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    *n = n.saturating_sub(1);
    FREED.notify_one();
  }
}

fn factory() -> Option<IWICImagingFactory> {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()
  }
}

/// A local path for `key`: a file as it is, a web picture through the
/// Windows download cache (as the notification cards do), a data URL
/// decoded to bytes.
enum Source {
  File(String),
  Bytes(Vec<u8>),
}

fn source(key: &str) -> Option<Source> {
  if key.starts_with("data:") {
    return data_url_bytes(key).map(Source::Bytes);
  }
  if key.starts_with("https://") || key.starts_with("http://") {
    let mut buf = [0u16; 1024];
    unsafe {
      URLDownloadToCacheFileW(None, &HSTRING::from(key), &mut buf, 0, None).ok()?;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    return Some(Source::File(String::from_utf16_lossy(&buf[..end])));
  }
  Some(Source::File(key.to_string()))
}

fn decoder(wic: &IWICImagingFactory, src: &Source) -> Option<IWICBitmapDecoder> {
  unsafe {
    match src {
      Source::File(path) => wic
        .CreateDecoderFromFilename(&HSTRING::from(path.as_str()), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand)
        .ok(),
      Source::Bytes(bytes) => {
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()
      }
    }
  }
}

/// Fits `w` x `h` inside `max_w` x `max_h` (never larger than it is).
fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
  let k = (max_w as f32 / w.max(1) as f32).min(max_h as f32 / h.max(1) as f32).min(1.0);
  (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1))
}

/// Scales a source to fit and copies its premultiplied pixels.
fn pixels(wic: &IWICImagingFactory, src: &IWICBitmapSource, max_w: u32, max_h: u32) -> Option<Pixels> {
  unsafe {
    let (mut w, mut h) = (0u32, 0u32);
    src.GetSize(&mut w, &mut h).ok()?;
    let (tw, th) = fit(w, h, max_w, max_h);
    let scaler = wic.CreateBitmapScaler().ok()?;
    scaler.Initialize(src, tw, th, WICBitmapInterpolationModeFant).ok()?;
    let conv = wic.CreateFormatConverter().ok()?;
    conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
    let mut data = vec![0u8; (tw * th * 4) as usize];
    conv.CopyPixels(std::ptr::null(), tw * 4, &mut data).ok()?;
    Some(Pixels { w: tw, h: th, data })
  }
}

/// The first frame, scaled to fit.
pub(super) fn load(key: &str, max_w: u32, max_h: u32) -> Option<Pixels> {
  let wic = factory()?;
  let src = source(key)?;
  let dec = decoder(&wic, &src)?;
  unsafe {
    let frame = dec.GetFrame(0).ok()?;
    let conv = wic.CreateFormatConverter().ok()?;
    conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
    let source: IWICBitmapSource = windows::core::Interface::cast(&conv).ok()?;
    pixels(&wic, &source, max_w, max_h)
  }
}

fn meta_u16(reader: &windows::Win32::Graphics::Imaging::IWICMetadataQueryReader, name: &str) -> Option<u16> {
  let mut v = windows::core::PROPVARIANT::default();
  let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
  unsafe {
    reader.GetMetadataByName(PCWSTR(wide.as_ptr()), &mut v).ok()?;
  }
  u16::try_from(&v).ok()
}

/// Every frame of a GIF composed onto its canvas (frame offsets, "restore
/// to background" and "restore to previous"), scaled to fit. At most 90
/// frames; a still picture gives one.
pub(super) fn load_frames(key: &str, max_w: u32, max_h: u32) -> Option<Vec<(Pixels, u32)>> {
  let wic = factory()?;
  let src = source(key)?;
  let dec = decoder(&wic, &src)?;
  unsafe {
    let count = dec.GetFrameCount().ok()?.min(90);
    let global = dec.GetMetadataQueryReader().ok();
    let cw = global.as_ref().and_then(|r| meta_u16(r, "/logscrdesc/Width")).map(u32::from);
    let ch = global.as_ref().and_then(|r| meta_u16(r, "/logscrdesc/Height")).map(u32::from);
    let first = dec.GetFrame(0).ok()?;
    let (mut fw, mut fh) = (0u32, 0u32);
    first.GetSize(&mut fw, &mut fh).ok()?;
    let (cw, ch) = (cw.unwrap_or(fw).max(1), ch.unwrap_or(fh).max(1));
    let mut canvas = vec![0u8; (cw * ch * 4) as usize];
    let mut out = Vec::new();
    for i in 0..count {
      let Ok(frame) = dec.GetFrame(i) else { break };
      let reader = frame.GetMetadataQueryReader().ok();
      let left = reader.as_ref().and_then(|r| meta_u16(r, "/imgdesc/Left")).unwrap_or(0) as u32;
      let top = reader.as_ref().and_then(|r| meta_u16(r, "/imgdesc/Top")).unwrap_or(0) as u32;
      let delay = reader.as_ref().and_then(|r| meta_u16(r, "/grctlext/Delay")).unwrap_or(10) as u32 * 10;
      let disposal = reader.as_ref().and_then(|r| meta_u16(r, "/grctlext/Disposal")).unwrap_or(0);
      let conv = wic.CreateFormatConverter().ok()?;
      conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
      let (mut w, mut h) = (0u32, 0u32);
      conv.GetSize(&mut w, &mut h).ok()?;
      let mut px = vec![0u8; (w * h * 4) as usize];
      conv.CopyPixels(std::ptr::null(), w * 4, &mut px).ok()?;
      let previous = (disposal == 3).then(|| canvas.clone());
      compose(&mut canvas, cw, ch, &px, w, h, left, top);
      let bmp = wic.CreateBitmapFromMemory(cw, ch, &GUID_WICPixelFormat32bppPBGRA, cw * 4, &canvas).ok()?;
      let source: IWICBitmapSource = windows::core::Interface::cast(&bmp).ok()?;
      if let Some(p) = pixels(&wic, &source, max_w, max_h) {
        out.push((p, delay));
      }
      match disposal {
        2 => clear(&mut canvas, cw, ch, left, top, w, h),
        3 => {
          if let Some(prev) = previous {
            canvas = prev;
          }
        }
        _ => {}
      }
    }
    (!out.is_empty()).then_some(out)
  }
}

/// Source-over of premultiplied pixels at (left, top).
#[allow(clippy::too_many_arguments)]
fn compose(canvas: &mut [u8], cw: u32, ch: u32, px: &[u8], w: u32, h: u32, left: u32, top: u32) {
  for y in 0..h {
    let cy = top + y;
    if cy >= ch {
      break;
    }
    for x in 0..w {
      let cx = left + x;
      if cx >= cw {
        break;
      }
      let s = ((y * w + x) * 4) as usize;
      let d = ((cy * cw + cx) * 4) as usize;
      let a = px[s + 3] as u32;
      if a == 255 {
        canvas[d..d + 4].copy_from_slice(&px[s..s + 4]);
      } else if a > 0 {
        for k in 0..4 {
          canvas[d + k] = (px[s + k] as u32 + canvas[d + k] as u32 * (255 - a) / 255) as u8;
        }
      }
    }
  }
}

fn clear(canvas: &mut [u8], cw: u32, ch: u32, left: u32, top: u32, w: u32, h: u32) {
  for y in top..(top + h).min(ch) {
    for x in left..(left + w).min(cw) {
      let d = ((y * cw + x) * 4) as usize;
      canvas[d..d + 4].fill(0);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pictures_fit_without_growing() {
    assert_eq!(fit(1920, 1080, 320, 200), (320, 180));
    assert_eq!(fit(100, 50, 320, 200), (100, 50));
    assert_eq!(fit(1080, 1920, 320, 200), (113, 200));
  }

  #[test]
  fn composing_blends_over_the_canvas() {
    let mut canvas = vec![0u8, 0, 200, 255]; // one red pixel
    compose(&mut canvas, 1, 1, &[0, 50, 0, 128], 1, 1, 0, 0); // half green, premultiplied
    assert_eq!(canvas[3], 255);
    assert_eq!(canvas[1], 50);
    assert_eq!(canvas[2], 99);
  }
}
