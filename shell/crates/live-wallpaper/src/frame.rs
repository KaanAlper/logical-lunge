//! `lunge-wallpaper --frame <video> <png>`: one frame of a video (a tenth
//! of the way in, at most a second) as a PNG. The core sets it as the
//! static wallpaper under the live one: the theme colours follow the
//! video, and the desktop shows the same picture before the video starts
//! or if it stops.

use std::{
  path::Path,
  sync::atomic::Ordering,
  time::{Duration, Instant},
};

use windows::{
  core::{Error, HSTRING},
  Win32::{
    Foundation::{E_FAIL, GENERIC_WRITE, RECT},
    Graphics::{
      Direct3D11::*,
      Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
      Imaging::*,
    },
    Media::MediaFoundation::{
      MFShutdown, MFStartup, MFSTARTUP_FULL, MF_VERSION,
    },
    System::Com::{
      CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER,
      COINIT_MULTITHREADED,
    },
  },
};

use crate::render::Gpu;

fn wait(until: impl Fn() -> bool, limit: Duration) -> bool {
  let start = Instant::now();
  while !until() {
    if start.elapsed() > limit {
      return false;
    }
    std::thread::sleep(Duration::from_millis(10));
  }
  true
}

pub fn save(video: &Path, png: &Path) -> windows::core::Result<()> {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
  }
  // into a file of its own first: a stopped (or a second) run never leaves
  // a half-written picture that would be taken for the frame
  let tmp = png.with_extension(format!("{}.tmp", std::process::id()));
  let result = grab(video, &tmp).and_then(|_| {
    std::fs::rename(&tmp, png).map_err(|_| Error::from(E_FAIL))
  });
  if result.is_err() {
    let _ = std::fs::remove_file(&tmp);
  }
  unsafe {
    let _ = MFShutdown();
  }
  result
}

fn grab(video: &Path, png: &Path) -> windows::core::Result<()> {
  let gpu = Gpu::new()?;
  let engine = gpu.open(video, false, false)?;
  if !wait(
    || {
      engine.state.ready.load(Ordering::Acquire)
        || engine.state.failed.load(Ordering::Acquire)
    },
    Duration::from_secs(10),
  ) || engine.state.failed.load(Ordering::Acquire)
  {
    return Err(Error::from(E_FAIL));
  }
  let (w, h) = engine.size().ok_or_else(|| Error::from(E_FAIL))?;
  unsafe {
    let duration = engine.engine.GetDuration();
    let at = if duration.is_finite() && duration > 0.0 {
      (duration / 10.0).min(1.0)
    } else {
      0.0
    };
    let _ = engine.engine.SetCurrentTime(at);
    engine.engine.Play()?;
  }
  // the seek ends first: a frame before it is the one at the start
  wait(
    || unsafe { !engine.engine.IsSeeking().as_bool() },
    Duration::from_secs(10),
  );
  if !wait(|| engine.new_frame(), Duration::from_secs(10)) {
    return Err(Error::from(E_FAIL));
  }
  unsafe {
    let _ = engine.engine.Pause();
  }

  // the frame into a GPU texture, then into memory the CPU can read
  let mut desc = D3D11_TEXTURE2D_DESC {
    Width: w,
    Height: h,
    MipLevels: 1,
    ArraySize: 1,
    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
    SampleDesc: DXGI_SAMPLE_DESC {
      Count: 1,
      Quality: 0,
    },
    Usage: D3D11_USAGE_DEFAULT,
    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
    ..Default::default()
  };
  let mut target = None;
  unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut target))? };
  let target = target.ok_or_else(|| Error::from(E_FAIL))?;
  unsafe {
    let dst = RECT {
      left: 0,
      top: 0,
      right: w as i32,
      bottom: h as i32,
    };
    engine
      .engine
      .TransferVideoFrame(&target, None, &dst, None)?;
  }
  desc.Usage = D3D11_USAGE_STAGING;
  desc.BindFlags = 0;
  desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
  let mut staging = None;
  unsafe {
    gpu
      .device
      .CreateTexture2D(&desc, None, Some(&mut staging))?
  };
  let staging = staging.ok_or_else(|| Error::from(E_FAIL))?;
  let mut pixels = vec![0u8; (w * h * 4) as usize];
  unsafe {
    gpu.context.CopyResource(&staging, &target);
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    gpu
      .context
      .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    let row = (w * 4) as usize;
    for y in 0..h as usize {
      let from = std::slice::from_raw_parts(
        (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
        row,
      );
      pixels[y * row..(y + 1) * row].copy_from_slice(from);
    }
    gpu.context.Unmap(&staging, 0);
  }
  // opaque, whatever the decoder left in the alpha byte
  for alpha in pixels.iter_mut().skip(3).step_by(4) {
    *alpha = 255;
  }
  write_png(png, w, h, &pixels)
}

fn write_png(
  path: &Path,
  w: u32,
  h: u32,
  pixels: &[u8],
) -> windows::core::Result<()> {
  unsafe {
    let wic: IWICImagingFactory = CoCreateInstance(
      &CLSID_WICImagingFactory,
      None,
      CLSCTX_INPROC_SERVER,
    )?;
    let stream = wic.CreateStream()?;
    stream.InitializeFromFilename(
      &HSTRING::from(path.as_os_str()),
      GENERIC_WRITE.0,
    )?;
    let encoder =
      wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
    encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
    let mut frame = None;
    encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
    let frame = frame.ok_or_else(|| Error::from(E_FAIL))?;
    frame.Initialize(None)?;
    frame.SetSize(w, h)?;
    let mut format = GUID_WICPixelFormat32bppBGRA;
    frame.SetPixelFormat(&mut format)?;
    frame.WritePixels(h, w * 4, pixels)?;
    frame.Commit()?;
    encoder.Commit()?;
  }
  Ok(())
}
