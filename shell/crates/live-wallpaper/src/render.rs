//! The video side, on its own thread: one Direct3D 11 device, one Media
//! Foundation engine per video (decoded once, on the GPU, even when it
//! shows on several monitors) and one swap chain per monitor window. A
//! frame is copied into the swap chains only when the engine has a new
//! one, at the display's pace; an engine decodes only while one of its
//! windows can show a frame, and while none can the thread just waits.
//!
//! Failures are not final: a window whose drawing failed is tried again
//! (later each time), a video that could not be opened is opened again
//! with the next target list, and when the graphics device is lost (a
//! driver update, a timeout recovery) the thread ends and the UI starts a
//! new one.

use std::{
  collections::HashMap,
  path::{Path, PathBuf},
  sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError},
    Arc,
  },
  time::{Duration, Instant},
};

use windows::{
  core::{Interface, BSTR},
  Win32::{
    Foundation::{E_FAIL, HMODULE, HWND, LPARAM, RECT, S_OK, WPARAM},
    Graphics::{
      Direct3D::D3D_DRIVER_TYPE_HARDWARE,
      Direct3D11::*,
      Dxgi::{Common::*, *},
    },
    Media::MediaFoundation::*,
    System::Com::{
      CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER,
      COINIT_MULTITHREADED,
    },
    UI::WindowsAndMessaging::PostMessageW,
  },
};

use crate::{fit, log};

pub struct Target {
  /// the UI's name for the window (window handles are reused)
  pub id: u64,
  pub hwnd: isize,
  pub width: u32,
  pub height: u32,
  pub file: PathBuf,
}

pub enum Cmd {
  /// the monitor windows and their videos (replaces the previous ones)
  Targets(Vec<Target>),
  /// per target, in the same order: covered by a fullscreen app, locked,
  /// display off, on battery
  Paused(Vec<bool>),
  /// let go of every window (they are about to be destroyed); answered
  /// when done
  Release(SyncSender<()>),
  Quit,
}

/// To the UI thread's window when a target shows its first frame (lparam:
/// the target's id).
pub const WM_APP_FIRST_FRAME: u32 = 0x8000 + 10;
/// To the UI thread's window when this thread ended without being asked.
pub const WM_APP_RENDER_DIED: u32 = 0x8000 + 11;

const RETRY_FIRST: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct State {
  pub ready: AtomicBool,
  pub failed: AtomicBool,
}

#[windows::core::implement(IMFMediaEngineNotify)]
struct Notify {
  state: Arc<State>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
  fn EventNotify(
    &self,
    event: u32,
    _param1: usize,
    _param2: u32,
  ) -> windows::core::Result<()> {
    match MF_MEDIA_ENGINE_EVENT(event as i32) {
      MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA
      | MF_MEDIA_ENGINE_EVENT_CANPLAY
      | MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY => {
        self.state.ready.store(true, Ordering::Release)
      }
      MF_MEDIA_ENGINE_EVENT_ERROR => {
        self.state.failed.store(true, Ordering::Release)
      }
      _ => {}
    }
    Ok(())
  }
}

pub struct Gpu {
  pub device: ID3D11Device,
  pub context: ID3D11DeviceContext,
  manager: IMFDXGIDeviceManager,
  factory: IMFMediaEngineClassFactory,
  adapter: IDXGIAdapter,
  dxgi: IDXGIFactory2,
  /// the first monitor's output: its vertical blank paces the drawing
  output: Option<IDXGIOutput>,
}

impl Gpu {
  /// Media Foundation must be started on this thread (MFStartup) before.
  pub fn new() -> windows::core::Result<Gpu> {
    unsafe {
      let mut device = None;
      let mut context = None;
      D3D11CreateDevice(
        None,
        D3D_DRIVER_TYPE_HARDWARE,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT
          | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        None,
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
      )?;
      let device: ID3D11Device =
        device.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
      let context: ID3D11DeviceContext =
        context.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
      // the engine decodes on its own threads with this device
      if let Ok(mt) = context.cast::<ID3D11Multithread>() {
        let _ = mt.SetMultithreadProtected(true);
      }
      let mut token = 0u32;
      let mut manager = None;
      MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
      let manager =
        manager.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
      manager.ResetDevice(&device, token)?;
      let factory: IMFMediaEngineClassFactory = CoCreateInstance(
        &CLSID_MFMediaEngineClassFactory,
        None,
        CLSCTX_INPROC_SERVER,
      )?;
      let adapter = device.cast::<IDXGIDevice>()?.GetAdapter()?;
      let output = adapter.EnumOutputs(0).ok();
      let dxgi: IDXGIFactory2 = adapter.GetParent()?;
      Ok(Gpu {
        device,
        context,
        manager,
        factory,
        adapter,
        dxgi,
        output,
      })
    }
  }

  /// Monitors changed: the output that paces the drawing may be another.
  fn refresh_output(&mut self) {
    self.output = unsafe { self.adapter.EnumOutputs(0) }.ok();
  }

  /// The device is gone (removed or reset): nothing drawn with it shows.
  fn lost(&self) -> bool {
    unsafe { self.device.GetDeviceRemovedReason() }.is_err()
  }

  /// A muted engine for `file`, its frames handed out as BGRA textures
  /// (frame server mode).
  pub fn open(
    &self,
    file: &Path,
    looping: bool,
    autoplay: bool,
  ) -> windows::core::Result<Engine> {
    unsafe {
      let state = Arc::new(State::default());
      let notify: IMFMediaEngineNotify = Notify {
        state: state.clone(),
      }
      .into();
      let mut attributes = None;
      MFCreateAttributes(&mut attributes, 3)?;
      let attributes =
        attributes.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
      attributes
        .SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &self.manager)?;
      attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
      attributes.SetUINT32(
        &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
        DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
      )?;
      // made an Engine at once: it is shut down (Drop) whichever step
      // below fails
      let engine = Engine {
        engine: self.factory.CreateInstance(0, &attributes)?,
        _notify: notify,
        state,
        paused: !autoplay,
      };
      engine.engine.SetMuted(true)?;
      engine.engine.SetLoop(looping)?;
      engine.engine.SetAutoPlay(autoplay)?;
      engine
        .engine
        .SetSource(&BSTR::from(file.to_string_lossy().as_ref()))?;
      Ok(engine)
    }
  }

  /// The swap chain of a monitor window: bit-block transfer, which a
  /// layered child window under the desktop icons accepts (flip-model
  /// chains do not work there).
  fn chain(
    &self,
    hwnd: isize,
    width: u32,
    height: u32,
  ) -> windows::core::Result<IDXGISwapChain1> {
    let desc = DXGI_SWAP_CHAIN_DESC1 {
      Width: width,
      Height: height,
      Format: DXGI_FORMAT_B8G8R8A8_UNORM,
      SampleDesc: DXGI_SAMPLE_DESC {
        Count: 1,
        Quality: 0,
      },
      BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
      BufferCount: 1,
      Scaling: DXGI_SCALING_STRETCH,
      SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
      AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
      ..Default::default()
    };
    unsafe {
      let chain = self.dxgi.CreateSwapChainForHwnd(
        &self.device,
        HWND(hwnd as _),
        &desc,
        None,
        None,
      )?;
      // DXGI leaves the window alone: no Alt+Enter, no watching its
      // messages (the UI thread owns it and may be waiting for this one)
      let _ = self.dxgi.MakeWindowAssociation(
        HWND(hwnd as _),
        DXGI_MWA_NO_WINDOW_CHANGES | DXGI_MWA_NO_ALT_ENTER,
      );
      Ok(chain)
    }
  }
}

pub struct Engine {
  pub engine: IMFMediaEngine,
  _notify: IMFMediaEngineNotify,
  pub state: Arc<State>,
  paused: bool,
}

impl Engine {
  /// The engine has a frame newer than the last one handed out
  /// (OnVideoStreamTick's S_OK; the wrapper would turn its "no new
  /// frame" S_FALSE into success too).
  pub fn new_frame(&self) -> bool {
    let mut pts = 0i64;
    unsafe {
      (Interface::vtable(&self.engine).OnVideoStreamTick)(
        Interface::as_raw(&self.engine),
        &mut pts,
      ) == S_OK
    }
  }

  pub fn size(&self) -> Option<(u32, u32)> {
    let (mut w, mut h) = (0u32, 0u32);
    unsafe {
      self
        .engine
        .GetNativeVideoSize(Some(&mut w), Some(&mut h))
        .ok()?
    };
    (w > 0 && h > 0).then_some((w, h))
  }

  /// The current frame into `texture` (width x height), filling it.
  pub fn transfer(
    &self,
    texture: &ID3D11Texture2D,
    width: u32,
    height: u32,
  ) -> windows::core::Result<()> {
    let (vw, vh) = self
      .size()
      .ok_or_else(|| windows::core::Error::from(E_FAIL))?;
    let (left, top, right, bottom) = fit::cover(vw, vh, width, height);
    let src = MFVideoNormalizedRect {
      left,
      top,
      right,
      bottom,
    };
    let dst = RECT {
      left: 0,
      top: 0,
      right: width as i32,
      bottom: height as i32,
    };
    let border = MFARGB {
      rgbBlue: 0,
      rgbGreen: 0,
      rgbRed: 0,
      rgbAlpha: 255,
    };
    unsafe {
      self.engine.TransferVideoFrame(
        texture,
        Some(&src),
        &dst,
        Some(&border),
      )
    }
  }

  fn set_paused(&mut self, paused: bool) {
    if paused == self.paused {
      return;
    }
    self.paused = paused;
    let _ = unsafe {
      if paused {
        self.engine.Pause()
      } else {
        self.engine.Play()
      }
    };
  }
}

impl Drop for Engine {
  fn drop(&mut self) {
    let _ = unsafe { self.engine.Shutdown() };
  }
}

struct Surface {
  id: u64,
  hwnd: isize,
  width: u32,
  height: u32,
  file: PathBuf,
  chain: Option<IDXGISwapChain1>,
  paused: bool,
  shown: bool,
  /// its video cannot be played (opening or decoding failed): until the
  /// next target list
  dead: bool,
  /// drawing failed: tried again at this time, waiting longer each time
  retry_at: Option<Instant>,
  retry_wait: Duration,
}

impl Surface {
  fn new(t: Target) -> Surface {
    Surface {
      id: t.id,
      hwnd: t.hwnd,
      width: t.width,
      height: t.height,
      file: t.file,
      chain: None,
      paused: false,
      shown: false,
      dead: false,
      retry_at: None,
      retry_wait: RETRY_FIRST,
    }
  }

  /// Shows no frame now.
  fn idle(&self, now: Instant) -> bool {
    self.paused || self.dead || self.retry_at.is_some_and(|t| t > now)
  }

  fn failed(&mut self, what: &str, err: &windows::core::Error) {
    log::line(&format!(
      "{what} (window {}): {err:?}; again in {} s",
      self.hwnd,
      self.retry_wait.as_secs()
    ));
    self.chain = None;
    self.retry_at = Some(Instant::now() + self.retry_wait);
    self.retry_wait = (self.retry_wait * 2).min(RETRY_MAX);
  }
}

/// Runs until asked to quit (true), or until it cannot go on (false): no
/// Media Foundation, no device, the device lost.
pub fn run(rx: Receiver<Cmd>, ui: isize) -> bool {
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    if let Err(err) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
      log::line(&format!("Media Foundation did not start: {err:?}"));
      return false;
    }
  }
  let asked = match Gpu::new() {
    Ok(mut gpu) => render(&mut gpu, &rx, ui),
    Err(err) => {
      log::line(&format!("no Direct3D device: {err:?}"));
      false
    }
  };
  unsafe {
    let _ = MFShutdown();
  }
  asked
}

fn render(gpu: &mut Gpu, rx: &Receiver<Cmd>, ui: isize) -> bool {
  let mut engines: HashMap<PathBuf, Engine> = HashMap::new();
  let mut surfaces: Vec<Surface> = Vec::new();
  let mut quick_waits = 0u32;
  let asked = loop {
    let now = Instant::now();
    let playing = surfaces.iter().any(|s| !s.idle(now));
    // a window waiting to try again wakes the thread at that time
    let retry = surfaces
      .iter()
      .filter(|s| !s.paused && !s.dead)
      .filter_map(|s| s.retry_at)
      .min();
    let cmd = if playing {
      match rx.try_recv() {
        Ok(c) => Some(c),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => break true,
      }
    } else if let Some(at) = retry {
      match rx.recv_timeout(at.saturating_duration_since(now)) {
        Ok(c) => Some(c),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => break true,
      }
    } else {
      match rx.recv() {
        Ok(c) => Some(c),
        Err(_) => break true,
      }
    };
    if let Some(cmd) = cmd {
      match cmd {
        Cmd::Quit => break true,
        Cmd::Release(done) => {
          surfaces.clear();
          let _ = done.send(());
        }
        Cmd::Targets(targets) => {
          surfaces = targets.into_iter().map(Surface::new).collect();
          gpu.refresh_output();
          // a video that failed is opened again (the file may have been
          // replaced)
          engines.retain(|file, e| {
            !e.state.failed.load(Ordering::Acquire)
              && surfaces.iter().any(|s| &s.file == file)
          });
          open_engines(gpu, &mut engines, &mut surfaces);
        }
        Cmd::Paused(paused) => {
          for (s, p) in surfaces.iter_mut().zip(paused) {
            s.paused = p;
          }
        }
      }
      sync_engines(&mut engines, &surfaces, Instant::now());
      // every waiting command first, then the frames
      continue;
    }
    // once per refresh (not DwmFlush: with nothing else changing on screen
    // it returns at once, and the loop would spin between the frames
    // of a 24 fps video)
    let started = Instant::now();
    let waited = gpu
      .output
      .as_ref()
      .is_some_and(|o| unsafe { o.WaitForVBlank() }.is_ok());
    // an output that went away or off may return at once: no spinning
    quick_waits =
      if waited && started.elapsed() < Duration::from_micros(500) {
        quick_waits + 1
      } else {
        0
      };
    if !waited || quick_waits >= 3 {
      std::thread::sleep(Duration::from_millis(16));
    }
    let now = Instant::now();
    let mut lost = false;
    for (file, engine) in &engines {
      if engine.state.failed.load(Ordering::Acquire) {
        for s in surfaces.iter_mut().filter(|s| &s.file == file && !s.dead)
        {
          log::line(&format!("cannot play {}", file.display()));
          s.dead = true;
        }
        continue;
      }
      if !engine.new_frame() {
        continue;
      }
      for s in surfaces
        .iter_mut()
        .filter(|s| &s.file == file && !s.idle(now))
      {
        if !draw(gpu, engine, s, ui) && gpu.lost() {
          lost = true;
        }
      }
    }
    if lost {
      // a driver update or a timeout recovery: the UI starts a new thread
      // with a new device for the same windows
      log::line("the graphics device was lost; starting again");
      break false;
    }
    sync_engines(&mut engines, &surfaces, now);
  };
  // engines shut down when dropped, before MFShutdown
  drop(engines);
  asked
}

fn open_engines(
  gpu: &Gpu,
  engines: &mut HashMap<PathBuf, Engine>,
  surfaces: &mut [Surface],
) {
  for i in 0..surfaces.len() {
    let file = surfaces[i].file.clone();
    if engines.contains_key(&file) || surfaces[i].dead {
      continue;
    }
    match gpu.open(&file, true, true) {
      Ok(e) => {
        engines.insert(file, e);
      }
      Err(err) => {
        log::line(&format!("cannot open {}: {err:?}", file.display()));
        for s in surfaces.iter_mut().filter(|s| s.file == file) {
          s.dead = true;
        }
      }
    }
  }
}

/// An engine decodes only while one of its windows can show a frame.
fn sync_engines(
  engines: &mut HashMap<PathBuf, Engine>,
  surfaces: &[Surface],
  now: Instant,
) {
  for (file, e) in engines.iter_mut() {
    e.set_paused(
      surfaces
        .iter()
        .filter(|s| &s.file == file)
        .all(|s| s.idle(now)),
    );
  }
}

/// The engine's current frame into the window; false when that failed.
fn draw(gpu: &Gpu, engine: &Engine, s: &mut Surface, ui: isize) -> bool {
  if s.chain.is_none() {
    match gpu.chain(s.hwnd, s.width, s.height) {
      Ok(c) => s.chain = Some(c),
      Err(err) => {
        s.failed("no swap chain", &err);
        return false;
      }
    }
  }
  let Some(chain) = &s.chain else { return false };
  let result = unsafe { chain.GetBuffer::<ID3D11Texture2D>(0) }
    .and_then(|texture| engine.transfer(&texture, s.width, s.height))
    .and_then(|_| unsafe { chain.Present(0, DXGI_PRESENT(0)) }.ok());
  match result {
    Ok(()) => {
      s.retry_at = None;
      s.retry_wait = RETRY_FIRST;
      if !s.shown {
        s.shown = true;
        unsafe {
          let _ = PostMessageW(
            HWND(ui as _),
            WM_APP_FIRST_FRAME,
            WPARAM(0),
            LPARAM(s.id as isize),
          );
        }
      }
      true
    }
    Err(err) => {
      s.failed("drawing failed", &err);
      false
    }
  }
}
