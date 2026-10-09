//! Copies of the videos at their monitors' size. A 4K video on a 1080p
//! monitor costs the GPU's video decoder four times the pixels it shows
//! (29% of the decoder against 9% for its 1080p copy, measured on an RTX
//! 4080), and a laptop pays that in battery and heat for as long as the
//! wallpaper plays. So a video bigger than every monitor it fills is copied
//! once, on the GPU (DXVA decode, video processor, the hardware H.264
//! encoder: a 19 s 4K60 video in about 6 s), at the smallest size that still
//! fills them; SSIM 0.99 against a careful downscale of the original.
//!
//! The copies live in `cache\wallpaper` with an index (`copies.json`):
//! for each source (path, length, modification time) and the monitor sizes
//! it fills, the copy's name, or "" when the video is small enough already.
//! The player reads it when it builds its windows, so a known video plays
//! from its copy from the start; an unknown one plays as it is while its
//! copy is made (in the background, at low priority, on mains power only),
//! then the windows are built again on the copy. A source that changed or
//! is no longer set loses its copy.

use std::{
  collections::BTreeMap,
  path::{Path, PathBuf},
};

/// A copy is made only when it is at most this part of the video's pixels.
const WORTH: f64 = 0.8;
/// Bits per pixel per frame the copy is encoded with (at most the source's
/// own bitrate): 1080p60 gets ~10 Mbit/s.
const BITS_PER_PIXEL: f64 = 0.08;

/// The size the copy of a `video` filling every one of `screens` is made
/// at, or None when the video is not much bigger than what they need. Each
/// screen shows the middle of the video at its aspect ratio (fit::cover),
/// so the copy is scaled by the largest factor any screen needs; even
/// sides, as H.264 wants.
pub fn target_size(video: (u32, u32), screens: &[(u32, u32)]) -> Option<(u32, u32)> {
  let (vw, vh) = (video.0 as f64, video.1 as f64);
  if vw < 2.0 || vh < 2.0 || screens.is_empty() {
    return None;
  }
  let scale = screens
    .iter()
    .map(|&(sw, sh)| (sw as f64 / vw).max(sh as f64 / vh))
    .fold(0.0_f64, f64::max);
  if scale * scale > WORTH {
    return None;
  }
  let even = |x: f64| ((x / 2.0).ceil() as u32 * 2).max(2);
  Some((even(vw * scale), even(vh * scale)))
}

/// The copy's bitrate: BITS_PER_PIXEL at its size and frame rate, never
/// above the source's (0: unknown).
pub fn bitrate(size: (u32, u32), fps: f64, source: u32) -> u32 {
  let want = (BITS_PER_PIXEL * size.0 as f64 * size.1 as f64 * fps.max(1.0)) as u32;
  let want = want.max(1_000_000);
  if source > 0 { want.min(source) } else { want }
}

/// What the index knows about a source at some monitor sizes.
#[derive(Clone, Debug, PartialEq)]
pub enum Known {
  /// play this copy
  Copy(PathBuf),
  /// the video itself is the right size (or its copy could not be made)
  Original,
}

/// The index's key: the source as it is now (a replaced file is another
/// key) and the sizes it fills.
pub fn key(source: &Path, screens: &[(u32, u32)]) -> Option<String> {
  let meta = std::fs::metadata(source).ok()?;
  let modified = meta
    .modified()
    .ok()?
    .duration_since(std::time::UNIX_EPOCH)
    .ok()?
    .as_secs();
  let mut sizes: Vec<String> = screens.iter().map(|(w, h)| format!("{w}x{h}")).collect();
  sizes.sort();
  sizes.dedup();
  Some(format!(
    "{}|{}|{}|{}",
    source.to_string_lossy().to_lowercase(),
    meta.len(),
    modified,
    sizes.join(",")
  ))
}

/// A file name for the copy of `key` (FNV-1a of the key: stable across
/// runs, unlike std's hasher).
pub fn file_name(key: &str, size: (u32, u32)) -> String {
  let mut h: u64 = 0xcbf29ce484222325;
  for b in key.bytes() {
    h ^= b as u64;
    h = h.wrapping_mul(0x100000001b3);
  }
  format!("{h:016x}-{}x{}.mp4", size.0, size.1)
}

pub fn dir() -> PathBuf {
  crate::config::data_dir().join("cache").join("wallpaper")
}

/// key -> copy file name ("" for the original)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index(pub BTreeMap<String, String>);

impl Index {
  pub fn parse(text: &str) -> Index {
    let map = serde_json::from_str::<BTreeMap<String, String>>(text.trim_start_matches('\u{feff}')).unwrap_or_default();
    Index(map)
  }

  pub fn load() -> Index {
    std::fs::read_to_string(dir().join("copies.json"))
      .map(|t| Index::parse(&t))
      .unwrap_or_default()
  }

  pub fn save(&self) {
    let _ = std::fs::create_dir_all(dir());
    let tmp = dir().join("copies.json.tmp");
    if let Ok(text) = serde_json::to_string_pretty(&self.0) {
      if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, dir().join("copies.json"));
      }
    }
  }

  /// What to play for `key`; None when unknown (or its copy is gone).
  pub fn known(&self, key: &str, in_dir: &Path) -> Option<Known> {
    match self.0.get(key)?.as_str() {
      "" => Some(Known::Original),
      name => {
        let path = in_dir.join(name);
        path.is_file().then_some(Known::Copy(path))
      }
    }
  }

  /// Keeps only the entries of `keys`; returns the copy files no entry
  /// names any more (to delete).
  pub fn keep_only(&mut self, keys: &[String], files: &[String]) -> Vec<String> {
    // Copies of a video still set stay, whatever size they were made for: a
    // game switching the display mode made a request at the game's size, and
    // the copy for the desktop's size was deleted (the next start decoded
    // the full original again and copied it once more).
    let sources: Vec<&str> = keys.iter().map(|k| source_part(k)).collect();
    self.0.retain(|k, _| sources.contains(&source_part(k)));
    files
      .iter()
      .filter(|f| f.ends_with(".mp4") && !self.0.values().any(|v| v == *f))
      .cloned()
      .collect()
  }
}

/// The part of a key naming the video (path, length, modified): the key
/// without the sizes.
fn source_part(key: &str) -> &str {
  key.rsplit_once('|').map_or(key, |(source, _)| source)
}

/// A copy to make: the source and the sizes it fills, under `key`.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
  pub key: String,
  pub source: PathBuf,
  pub screens: Vec<(u32, u32)>,
}

#[cfg(windows)]
pub use worker::{request, set_hold, set_on_battery, stop};

#[cfg(windows)]
mod worker {
  use super::*;
  use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
    Mutex,
  };
  use std::time::Duration;
  use windows::{
    core::{Interface, HSTRING},
    Win32::{
      Foundation::HMODULE,
      Graphics::{Direct3D::D3D_DRIVER_TYPE_HARDWARE, Direct3D11::*},
      Media::MediaFoundation::*,
      System::{
        Com::{CoInitializeEx, COINIT_MULTITHREADED},
        Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN},
      },
    },
  };

  /// Waits this long after the windows are built: the desktop starting
  /// (or a monitor change) comes first.
  const SETTLE: Duration = Duration::from_secs(20);
  const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

  /// copies to make, and every key now set
  type Batch = (Vec<Job>, Vec<String>);
  static QUEUE: Mutex<Option<Sender<Batch>>> = Mutex::new(None);
  static STOP: AtomicBool = AtomicBool::new(false);
  static ON_BATTERY: AtomicBool = AtomicBool::new(false);
  static HOLD: AtomicBool = AtomicBool::new(false);

  pub fn set_on_battery(on: bool) {
    ON_BATTERY.store(on, Ordering::Release);
  }

  /// A fullscreen app (a game) covers a monitor, or the session is locked:
  /// no copy is made meanwhile (decoding and encoding a video on the GPU
  /// in the middle of a game); a copy being made is dropped and made later.
  pub fn set_hold(on: bool) {
    HOLD.store(on, Ordering::Release);
  }

  /// The player is closing: a copy being made is dropped.
  pub fn stop() {
    STOP.store(true, Ordering::Release);
  }

  /// Makes the `jobs`' copies, then keeps only the copies of `keys` (every
  /// source and size now set). `made` runs on the worker's thread after a
  /// copy is made.
  pub fn request(jobs: Vec<Job>, keys: Vec<String>, made: fn()) {
    let mut q = match QUEUE.lock() {
      Ok(q) => q,
      Err(_) => return,
    };
    if q.is_none() {
      let (tx, rx) = mpsc::channel();
      if std::thread::Builder::new()
        .name("video-copies".into())
        .spawn(move || run(rx, made))
        .is_err()
      {
        return;
      }
      *q = Some(tx);
    }
    if let Some(tx) = q.as_ref() {
      let _ = tx.send((jobs, keys));
    }
  }

  fn run(rx: Receiver<Batch>, made: fn()) {
    unsafe {
      let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
      let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    while let Ok(mut batch) = rx.recv() {
      // a newer request (the windows were built again) replaces this one
      std::thread::sleep(SETTLE);
      // a game in front: wait it out (newer requests still replace this one)
      while HOLD.load(Ordering::Acquire) && !STOP.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_secs(2));
      }
      while let Ok(newer) = rx.try_recv() {
        batch = newer;
      }
      let (jobs, keys) = batch;
      let mut index = Index::load();
      for job in jobs {
        if STOP.load(Ordering::Acquire) {
          return;
        }
        if ON_BATTERY.load(Ordering::Acquire) || index.0.contains_key(&job.key) {
          continue;
        }
        match make(&job) {
          Ok(Some(name)) => {
            crate::log::line(&format!("{} plays from its copy {name}", job.source.display()));
            index.0.insert(job.key.clone(), name);
            index.save();
            made();
          }
          Ok(None) => {
            index.0.insert(job.key.clone(), String::new());
            index.save();
          }
          // stopped, on battery: tried again with the next request
          Err(None) => {}
          Err(Some(err)) => {
            crate::log::line(&format!("no copy of {}: {err}", job.source.display()));
            index.0.insert(job.key.clone(), String::new());
            index.save();
          }
        }
      }
      // copies of videos no longer set (or replaced) go
      let files: Vec<String> = std::fs::read_dir(dir())
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
      for f in index.keep_only(&keys, &files) {
        let _ = std::fs::remove_file(dir().join(f));
      }
      index.save();
    }
  }

  /// The copy's file name; None when the video needs none; Err(None) when
  /// it was stopped (try again later), Err(Some) when it failed.
  pub(super) fn make(job: &Job) -> Result<Option<String>, Option<String>> {
    unsafe {
      MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| Some(format!("Media Foundation: {e}")))?;
      let r = transcode(job);
      let _ = MFShutdown();
      r
    }
  }

  unsafe fn transcode(job: &Job) -> Result<Option<String>, Option<String>> {
    let fail = |what: &str, e: windows::core::Error| Some(format!("{what}: {e}"));
    let mut device = None;
    D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(),
      D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None)
      .map_err(|e| fail("no Direct3D device", e))?;
    let device: ID3D11Device = device.ok_or(None)?;
    if let Ok(mt) = device.GetImmediateContext().and_then(|c| c.cast::<ID3D11Multithread>()) {
      let _ = mt.SetMultithreadProtected(true);
    }
    let (mut token, mut manager) = (0u32, None);
    MFCreateDXGIDeviceManager(&mut token, &mut manager).map_err(|e| fail("device manager", e))?;
    let manager: IMFDXGIDeviceManager = manager.ok_or(None)?;
    manager.ResetDevice(&device, token).map_err(|e| fail("device manager", e))?;

    // reader: decoded and scaled on the GPU
    let mut attrs = None;
    MFCreateAttributes(&mut attrs, 3).map_err(|e| fail("attributes", e))?;
    let attrs: IMFAttributes = attrs.ok_or(None)?;
    attrs.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &manager).map_err(|e| fail("reader", e))?;
    attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1).map_err(|e| fail("reader", e))?;
    attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1).map_err(|e| fail("reader", e))?;
    let reader = MFCreateSourceReaderFromURL(&HSTRING::from(job.source.as_os_str()), &attrs).map_err(|e| fail("cannot read the video", e))?;
    reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).map_err(|e| fail("reader", e))?;
    reader.SetStreamSelection(VIDEO, true).map_err(|e| fail("no video stream", e))?;
    let native = reader.GetNativeMediaType(VIDEO, 0).map_err(|e| fail("no video stream", e))?;
    let frame = native.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|e| fail("no frame size", e))?;
    let rate = native.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or((30u64 << 32) | 1);
    let source_bitrate = native.GetUINT32(&MF_MT_AVG_BITRATE).unwrap_or(0);
    let Some(size) = target_size(((frame >> 32) as u32, frame as u32), &job.screens) else {
      return Ok(None);
    };
    let fps = (rate >> 32) as f64 / (rate as u32).max(1) as f64;
    let pack = |a: u32, b: u32| (a as u64) << 32 | b as u64;

    let decoded = MFCreateMediaType().map_err(|e| fail("media type", e))?;
    decoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| fail("media type", e))?;
    decoded.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).map_err(|e| fail("media type", e))?;
    decoded.SetUINT64(&MF_MT_FRAME_SIZE, pack(size.0, size.1)).map_err(|e| fail("media type", e))?;
    decoded.SetUINT64(&MF_MT_FRAME_RATE, rate).map_err(|e| fail("media type", e))?;
    decoded.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(|e| fail("media type", e))?;
    decoded.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1)).map_err(|e| fail("media type", e))?;
    reader.SetCurrentMediaType(VIDEO, None, &decoded).map_err(|e| fail("cannot scale the video", e))?;
    let decoded = reader.GetCurrentMediaType(VIDEO).map_err(|e| fail("cannot scale the video", e))?;

    // writer: the hardware H.264 encoder where there is one
    let name = file_name(&job.key, size);
    let _ = std::fs::create_dir_all(dir());
    let part = dir().join(format!("{name}.part.mp4"));
    let _ = std::fs::remove_file(&part);
    let mut wattrs = None;
    MFCreateAttributes(&mut wattrs, 2).map_err(|e| fail("attributes", e))?;
    let wattrs: IMFAttributes = wattrs.ok_or(None)?;
    wattrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1).map_err(|e| fail("writer", e))?;
    wattrs.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &manager).map_err(|e| fail("writer", e))?;
    let writer = MFCreateSinkWriterFromURL(&HSTRING::from(part.as_os_str()), None, &wattrs).map_err(|e| fail("cannot write the copy", e))?;
    let encoded = MFCreateMediaType().map_err(|e| fail("media type", e))?;
    encoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| fail("media type", e))?;
    encoded.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).map_err(|e| fail("media type", e))?;
    encoded.SetUINT32(&MF_MT_AVG_BITRATE, bitrate(size, fps, source_bitrate)).map_err(|e| fail("media type", e))?;
    encoded.SetUINT64(&MF_MT_FRAME_SIZE, pack(size.0, size.1)).map_err(|e| fail("media type", e))?;
    encoded.SetUINT64(&MF_MT_FRAME_RATE, rate).map_err(|e| fail("media type", e))?;
    encoded.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(|e| fail("media type", e))?;
    encoded.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1)).map_err(|e| fail("media type", e))?;
    encoded.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32).map_err(|e| fail("media type", e))?;
    let stream = writer.AddStream(&encoded).map_err(|e| fail("no H.264 encoder", e))?;
    writer.SetInputMediaType(stream, &decoded, None).map_err(|e| fail("no H.264 encoder", e))?;
    writer.BeginWriting().map_err(|e| fail("cannot write the copy", e))?;

    let mut frames = 0u32;
    let result = loop {
      if STOP.load(Ordering::Acquire) || ON_BATTERY.load(Ordering::Acquire) || HOLD.load(Ordering::Acquire) {
        break Err(None);
      }
      let (mut flags, mut sample) = (0u32, None);
      if let Err(e) = reader.ReadSample(VIDEO, 0, None, Some(&mut flags), None, Some(&mut sample)) {
        break Err(fail("decoding", e));
      }
      if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
        break if frames > 0 { writer.Finalize().map_err(|e| fail("finishing the copy", e)) } else { Err(Some("no frames".into())) };
      }
      if let Some(s) = sample {
        if let Err(e) = writer.WriteSample(stream, &s) {
          break Err(fail("encoding", e));
        }
        frames += 1;
      }
    };
    drop(writer);
    if let Err(e) = result {
      let _ = std::fs::remove_file(&part);
      return Err(e);
    }
    // the copy must open at its size before it replaces the video
    let check = MFCreateSourceReaderFromURL(&HSTRING::from(part.as_os_str()), None)
      .and_then(|r| r.GetNativeMediaType(VIDEO, 0))
      .and_then(|t| t.GetUINT64(&MF_MT_FRAME_SIZE));
    if check.ok() != Some(pack(size.0, size.1)) {
      let _ = std::fs::remove_file(&part);
      return Err(Some("the copy does not open at its size".into()));
    }
    std::fs::rename(&part, dir().join(&name)).map_err(|e| Some(format!("keeping the copy: {e}")))?;
    crate::log::line(&format!("copied {} at {}x{}: {frames} frames", job.source.display(), size.0, size.1));
    Ok(Some(name))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn copies_only_what_is_much_bigger_than_its_screens() {
    // the cases of this machine
    assert_eq!(target_size((3840, 2160), &[(1920, 1080)]), Some((1920, 1080)));
    assert_eq!(target_size((3840, 2160), &[(1280, 1024)]), Some((1822, 1024)));
    // two monitors on one video: the bigger need wins
    assert_eq!(target_size((3840, 2160), &[(1280, 1024), (1920, 1080)]), Some((1920, 1080)));
    // already the screen's size, or close to it
    assert_eq!(target_size((1920, 1080), &[(1920, 1080)]), None);
    assert_eq!(target_size((2560, 1440), &[(1920, 1080), (2560, 1440)]), None);
    assert_eq!(target_size((3840, 2160), &[(3840, 2160)]), None);
    assert_eq!(target_size((0, 0), &[(1920, 1080)]), None);
    assert_eq!(target_size((3840, 2160), &[]), None);
  }

  /// Every size from a sliver to 8K against every screen of a list: the
  /// copy is never smaller than what a screen shows, never larger than the
  /// video, has even sides, and is made only when it saves a fifth.
  #[test]
  fn target_size_rule() {
    let screens = [(1280, 720), (1280, 1024), (1366, 768), (1920, 1080), (2560, 1080), (2560, 1440), (3440, 1440), (3840, 2160)];
    for vw in (64..=7680).step_by(331) {
      for vh in (64..=4320).step_by(277) {
        for n in 1..=2 {
          let set: Vec<(u32, u32)> = screens.iter().copied().cycle().skip(vw as usize % 8).take(n).collect();
          match target_size((vw, vh), &set) {
            Some((w, h)) => {
              assert!(w % 2 == 0 && h % 2 == 0, "{vw}x{vh} -> {w}x{h}");
              assert!(w <= vw + 1 && h <= vh + 1, "{vw}x{vh} -> {w}x{h}");
              assert!((w as f64 * h as f64) <= WORTH * vw as f64 * vh as f64 + 4.0 * (vw + vh) as f64, "{vw}x{vh} -> {w}x{h}");
              for &(sw, sh) in &set {
                // the screen's cut of the copy covers it
                let s = (sw as f64 / w as f64).max(sh as f64 / h as f64);
                assert!(s <= 1.0 + 1e-9, "{vw}x{vh} -> {w}x{h} too small for {sw}x{sh}");
              }
            }
            None => {
              let s = set.iter().map(|&(sw, sh)| (sw as f64 / vw as f64).max(sh as f64 / vh as f64)).fold(0.0, f64::max);
              assert!(s * s > WORTH, "{vw}x{vh} on {set:?} should be copied");
            }
          }
        }
      }
    }
  }

  #[test]
  fn bitrate_follows_size_and_source() {
    assert_eq!(bitrate((1920, 1080), 60.0, 9_845_475), 9_845_475);
    assert_eq!(bitrate((1920, 1080), 60.0, 50_000_000), 9_953_280);
    assert_eq!(bitrate((1920, 1080), 30.0, 0), 4_976_640);
    assert_eq!(bitrate((64, 64), 30.0, 0), 1_000_000);
  }

  #[test]
  fn names_are_stable_and_distinct() {
    let a = file_name("c:\\v\\a.mp4|1|2|1920x1080", (1920, 1080));
    assert_eq!(a, file_name("c:\\v\\a.mp4|1|2|1920x1080", (1920, 1080)));
    assert_ne!(a, file_name("c:\\v\\a.mp4|1|3|1920x1080", (1920, 1080)));
    assert!(a.ends_with("-1920x1080.mp4"));
  }

  #[test]
  fn index_keeps_what_is_set_and_drops_the_rest() {
    let mut index = Index::parse(r#"{"a|1|2|1920x1080":"aaaa-1920x1080.mp4","b|1|2|1920x1080":"bbbb-1920x1080.mp4","c|1|2|1920x1080":""}"#);
    let files = vec!["aaaa-1920x1080.mp4".to_string(), "bbbb-1920x1080.mp4".to_string(), "stale-1280x720.mp4".to_string(), "copies.json".to_string(), "x.part.mp4".to_string()];
    let gone = index.keep_only(&["a|1|2|1920x1080".to_string(), "c|1|2|1920x1080".to_string()], &files);
    assert_eq!(gone, vec!["bbbb-1920x1080.mp4", "stale-1280x720.mp4", "x.part.mp4"]);
    assert_eq!(index.0.len(), 2);
    assert_eq!(Index::parse("not json"), Index::default());
  }

  #[test]
  fn index_keeps_another_sizes_copy_of_a_video_still_set() {
    let mut index = Index::parse(r#"{"a|1|2|1920x1080":"aaaa-1920x1080.mp4"}"#);
    let files = vec!["aaaa-1920x1080.mp4".to_string()];
    // a game switched the display to 1280x720: the desktop's copy stays
    let gone = index.keep_only(&["a|1|2|1280x720".to_string()], &files);
    assert!(gone.is_empty());
    assert_eq!(index.0.len(), 1);
  }

  /// A real video (LL_TEST_VIDEO, 4K) copied for a 1080p screen, as the
  /// player does it: `cargo test -p lunge-wallpaper -- --ignored copies`
  #[cfg(windows)]
  #[test]
  #[ignore]
  fn copies_a_real_video() {
    let Some(video) = std::env::var_os("LL_TEST_VIDEO").map(PathBuf::from) else { return };
    let screens = vec![(1920, 1080)];
    let job = Job { key: key(&video, &screens).unwrap(), source: video, screens };
    let started = std::time::Instant::now();
    let name = worker::make(&job).expect("copy").expect("a 4K video is copied for 1080p");
    let path = dir().join(&name);
    assert!(name.ends_with("-1920x1080.mp4"), "{name}");
    assert!(std::fs::metadata(&path).unwrap().len() > 0);
    println!("{} in {:.1} s", path.display(), started.elapsed().as_secs_f64());
    let _ = std::fs::remove_file(path);
  }

  #[test]
  fn known_needs_the_copy_file() {
    let dir = std::env::temp_dir().join(format!("ll-copies-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("here.mp4"), b"x").unwrap();
    let index = Index::parse(r#"{"k1":"here.mp4","k2":"gone.mp4","k3":""}"#);
    assert_eq!(index.known("k1", &dir), Some(Known::Copy(dir.join("here.mp4"))));
    assert_eq!(index.known("k2", &dir), None, "a deleted copy is made again");
    assert_eq!(index.known("k3", &dir), Some(Known::Original));
    assert_eq!(index.known("k4", &dir), None);
    let _ = std::fs::remove_dir_all(&dir);
  }
}
