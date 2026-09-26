//! Native bar (docs/native-bar.md): the ii bar drawn with Direct2D into
//! DirectComposition surfaces, without a WebView.
//!
//! Runs on its own thread with a Win32 message loop. Data comes from the
//! shell's providers (forwarded from main.rs) and from the window manager's
//! IPC; actions go to the providers, the window manager and the core.

mod anim;
mod brightness;
mod core_api;
mod fonts;
mod gfx;
mod icons;
mod model;
mod view;
mod wm;

use std::{
  cell::RefCell,
  collections::HashMap,
  path::PathBuf,
  sync::{
    atomic::{AtomicIsize, Ordering},
    Arc, OnceLock,
  },
  time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crossbeam::channel::{unbounded, Receiver, Sender};
use serde_json::json;
use windows::{
  core::{w, HSTRING},
  Win32::{
    Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      DirectComposition::{
        IDCompositionRectangleClip, IDCompositionSurface, IDCompositionTarget, IDCompositionVisual2,
      },
      Gdi::{EnumDisplayMonitors, GetMonitorInfoW, ScreenToClient, HDC, HMONITOR, MONITORINFOEXW},
    },
    System::{
      Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
      LibraryLoader::GetModuleHandleW,
    },
    UI::{
      Controls::WM_MOUSELEAVE,
      HiDpi::{
        GetDpiForMonitor, SetThreadDpiAwarenessContext,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
      },
      Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT},
      WindowsAndMessaging::*,
    },
  },
};

use self::{
  anim::{Animated, OUT_SINE},
  brightness::{Display, Step},
  fonts::Fonts,
  gfx::{Gfx, Rgba},
  icons::Icons,
  model::{pin_key, Model},
  view::{HitKind, OsdKind, Painter, Res},
};
use crate::providers::{
  AudioFunction, MediaControlArgs, MediaFunction, ProviderConfig,
  ProviderEmission, ProviderFunction, ProviderManager, ProviderOutput, SetMuteArgs,
  SetVolumeArgs, SystrayFunction, SystrayIconArgs,
};

const HASH_PREFIX: &str = "native-bar:";
const WM_APP_WAKE: u32 = WM_APP + 1;
const WM_APP_REBUILD: u32 = WM_APP + 2;
const TIMER_CLOCK: usize = 1;
const TIMER_REBUILD: usize = 2;
const TIMER_OSD: usize = 3;
const TIMER_ALIVE: usize = 4;
const TIMER_RECOVER: usize = 5;
/// demo only (`LL_NATIVE_BAR_CYCLE=1`): fakes workspace switches to measure the pill animation
const TIMER_CYCLE: usize = 6;
/// The core finds the bar by this title (slides, focus guard, taskbar fallback, splash).
const TITLE: &str = "Logical Lunge · bar";
/// ii: the first four tray icons are pinned until the user moves them.
const DEFAULT_PINNED: usize = 4;

enum Msg {
  Provider(ProviderEmission),
  Wm(wm::WmState),
  Apps(Vec<icons::App>),
  WinIcon(i64, Option<Vec<u8>>),
  Display(brightness::Update),
}

static SENDER: OnceLock<Sender<Msg>> = OnceLock::new();
static WAKE: AtomicIsize = AtomicIsize::new(0);

fn send(msg: Msg) {
  let Some(tx) = SENDER.get() else { return };
  let _ = tx.send(msg);
  let h = WAKE.load(Ordering::Acquire);
  if h != 0 {
    unsafe {
      let _ = PostMessageW(HWND(h as _), WM_APP_WAKE, WPARAM(0), LPARAM(0));
    }
  }
}

/// Every provider emission passes through here (main.rs); ours go to the bar.
pub fn forward(emission: &ProviderEmission) {
  if emission.config_hash.starts_with(HASH_PREFIX) {
    send(Msg::Provider(emission.clone()));
  }
}

pub struct Options {
  /// `ui/logical-lunge`: fonts, i18n.json, prefs.json.
  pub pack_dir: PathBuf,
  /// Test run next to the web bar: own title, just below it, topmost.
  pub demo: bool,
}

pub fn start(manager: Arc<ProviderManager>, opts: Options) -> anyhow::Result<()> {
  let (tx, rx) = unbounded();
  SENDER.set(tx).map_err(|_| anyhow::anyhow!("native bar already started"))?;
  let wm_cmd = wm::spawn(|state| send(Msg::Wm(state)));
  let rt = tokio::runtime::Handle::current();

  // the same providers (and intervals) as ui/bar.html
  let creator = manager.clone();
  rt.spawn(async move {
    let configs = [
      ("cpu", json!({ "type": "cpu", "refreshInterval": 3000 })),
      ("memory", json!({ "type": "memory", "refreshInterval": 3000 })),
      ("battery", json!({ "type": "battery", "refreshInterval": 60000 })),
      ("network", json!({ "type": "network", "refreshInterval": 5000 })),
      ("audio", json!({ "type": "audio" })),
      ("media", json!({ "type": "media" })),
      ("systray", json!({ "type": "systray" })),
    ];
    for (name, config) in configs {
      match serde_json::from_value::<ProviderConfig>(config) {
        Ok(config) => {
          if let Err(err) = creator.create(format!("{}{}", HASH_PREFIX, name), config).await {
            tracing::warn!("Native bar: provider {}: {:?}", name, err);
          }
        }
        Err(err) => tracing::warn!("Native bar: provider config {}: {}", name, err),
      }
    }
  });

  // the app list (icons for the workspace dots); the core may still be starting
  std::thread::spawn(|| {
    for i in 1..=6u64 {
      if let Some((200, body)) = core_api::post("/apps.json") {
        if let Ok(apps) = serde_json::from_slice::<Vec<icons::App>>(&body) {
          send(Msg::Apps(apps));
          return;
        }
      }
      std::thread::sleep(Duration::from_secs(i));
    }
  });

  std::thread::Builder::new().name("native-bar".into()).spawn(move || {
    if let Err(err) = ui_thread(rx, wm_cmd, manager, rt, opts) {
      tracing::error!("Native bar stopped: {:?}", err);
    }
  })?;
  Ok(())
}

/// A DirectComposition visual with its own surface.
struct Layer {
  visual: IDCompositionVisual2,
  surface: IDCompositionSurface,
}

impl Layer {
  fn new(gfx: &Gfx, w: u32, h: u32) -> windows::core::Result<Self> {
    unsafe {
      let visual = gfx.dcomp.CreateVisual()?;
      let surface = gfx.surface(w, h)?;
      visual.SetContent(&surface)?;
      Ok(Self { visual, surface })
    }
  }
}

struct Bar {
  hwnd: HWND,
  /// `\\.\DISPLAY1`
  device: String,
  monitor: RECT,
  scale: f32,
  width: f32,
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  /// everything but the workspace pill and icons
  bg: Layer,
  /// the active workspace pill: a primary-coloured strip cut by `pill_clip`,
  /// whose edges the compositor animates (ii: leading edge 100 ms, trailing 300 ms)
  pill: Layer,
  pill_clip: IDCompositionRectangleClip,
  pill_left: Animated,
  pill_right: Animated,
  pill_color: Option<Rgba>,
  pill_idx: Option<usize>,
  /// workspace icons / dots above the pill
  fg: Layer,
  /// this bar's "alive" id for the core's watchdog
  alive_id: String,
  frame: view::Frame,
  hover: Option<HitKind>,
  hover_left: bool,
  hover_right: bool,
  tracking: bool,
}

struct OsdWin {
  hwnd: HWND,
  scale: f32,
  _target: IDCompositionTarget,
  layer: Layer,
}

struct Ui {
  gfx: Gfx,
  fonts: Fonts,
  res: Res,
  icons: Icons,
  model: Model,
  bars: Vec<Bar>,
  osd: Option<OsdWin>,
  displays: HashMap<String, Display>,
  msg_hwnd: HWND,
  rx: Receiver<Msg>,
  wm_cmd: wm::CommandTx,
  manager: Arc<ProviderManager>,
  rt: tokio::runtime::Handle,
  demo: bool,
  pins_file: PathBuf,
  last_wheel: Instant,
  last_ws_wheel: Instant,
  /// device of the bar last scrolled for volume, and when
  volume_wheel: Option<(String, Instant)>,
  last_volume: Option<(u32, bool)>,
  last_mic: Option<bool>,
  /// what the bars showed last (skip repaints that would draw the same)
  last_key: u64,
  /// device loss: recovery attempts in a row
  recover_tries: u32,
}

thread_local! {
  static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

const CLASS: windows::core::PCWSTR = w!("LungeNativeBar");

fn state_dir() -> PathBuf {
  let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
  base.join("LogicalLunge").join("state")
}

fn ui_thread(
  rx: Receiver<Msg>,
  wm_cmd: wm::CommandTx,
  manager: Arc<ProviderManager>,
  rt: tokio::runtime::Handle,
  opts: Options,
) -> anyhow::Result<()> {
  unsafe {
    CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    let hinst = GetModuleHandleW(None)?;
    let wc = WNDCLASSEXW {
      cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
      style: CS_DBLCLKS,
      lpfnWndProc: Some(wndproc),
      hInstance: hinst.into(),
      hCursor: LoadCursorW(None, IDC_ARROW)?,
      lpszClassName: CLASS,
      ..Default::default()
    };
    RegisterClassExW(&wc);
    let msg_hwnd = CreateWindowExW(
      WINDOW_EX_STYLE::default(),
      CLASS,
      w!("Logical Lunge native bar"),
      WINDOW_STYLE::default(),
      0,
      0,
      0,
      0,
      HWND_MESSAGE,
      None,
      hinst,
      None,
    )?;

    let started = Instant::now();
    let gfx = Gfx::new()?;
    let fonts = Fonts::load(&gfx.dwrite, &opts.pack_dir)?;
    let res = Res::new(&gfx)?;
    let mut model = Model::new(&opts.pack_dir);
    let pins_file = state_dir().join("tray-pins.json");
    model.pins = std::fs::read_to_string(&pins_file).ok().and_then(|s| serde_json::from_str(&s).ok());
    tracing::info!(
      "Native bar ready in {} ms ({})",
      started.elapsed().as_millis(),
      if gfx.warp { "WARP" } else { "GPU" }
    );

    UI.with(|u| {
      *u.borrow_mut() = Some(Ui {
        gfx,
        fonts,
        res,
        icons: Icons::default(),
        model,
        bars: Vec::new(),
        osd: None,
        displays: HashMap::new(),
        msg_hwnd,
        rx,
        wm_cmd,
        manager,
        rt,
        demo: opts.demo,
        pins_file,
        last_wheel: Instant::now(),
        last_ws_wheel: Instant::now(),
        volume_wheel: None,
        last_volume: None,
        last_mic: None,
        last_key: 0,
        recover_tries: 0,
      })
    });
    WAKE.store(msg_hwnd.0 as isize, Ordering::Release);
    with_ui(|ui| {
      ui.create_bars();
      ui.drain();
    });
    SetTimer(msg_hwnd, TIMER_CLOCK, ms_to_next_minute(), None);
    if !opts.demo {
      SetTimer(msg_hwnd, TIMER_ALIVE, 30_000, None);
    } else if std::env::var_os("LL_NATIVE_BAR_CYCLE").is_some() {
      SetTimer(msg_hwnd, TIMER_CYCLE, 1500, None);
    }

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
      let _ = TranslateMessage(&msg);
      DispatchMessageW(&msg);
    }
  }
  Ok(())
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
  UI.with(|u| {
    // re-entrant messages (sent while the UI is busy) go to DefWindowProc
    let mut guard = u.try_borrow_mut().ok()?;
    let ui = guard.as_mut()?;
    Some(f(ui))
  })
}

fn ms_to_next_minute() -> u32 {
  let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
  let into = now.as_millis() % 60_000;
  (60_000 - into as u32).max(200) + 30
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
  if msg == WM_MOUSEACTIVATE {
    // clicking the bar never takes the keyboard from the app
    return LRESULT(MA_NOACTIVATE as isize);
  }
  if msg == WM_ERASEBKGND {
    return LRESULT(1);
  }
  if let Some(Some(r)) = with_ui(|ui| ui.handle(hwnd, msg, wp, lp)) {
    return r;
  }
  unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn lparam_point(lp: LPARAM) -> (i32, i32) {
  ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

fn monitor_device(mon: HMONITOR) -> String {
  unsafe {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if !GetMonitorInfoW(mon, &mut info as *mut _ as *mut _).as_bool() {
      return String::new();
    }
    let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
    String::from_utf16_lossy(&info.szDevice[..end])
  }
}

impl Ui {
  fn handle(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if hwnd == self.msg_hwnd {
      match msg {
        WM_APP_WAKE => self.drain(),
        WM_TIMER if wp.0 == TIMER_CLOCK => {
          unsafe { SetTimer(self.msg_hwnd, TIMER_CLOCK, ms_to_next_minute(), None) };
          if self.model.tick_clock() {
            self.redraw_all();
          }
        }
        WM_TIMER if wp.0 == TIMER_REBUILD => {
          unsafe {
            let _ = KillTimer(self.msg_hwnd, TIMER_REBUILD);
          }
          self.create_bars();
        }
        WM_TIMER if wp.0 == TIMER_OSD => {
          unsafe {
            let _ = KillTimer(self.msg_hwnd, TIMER_OSD);
            if let Some(o) = &self.osd {
              let _ = ShowWindow(o.hwnd, SW_HIDE);
            }
          }
        }
        WM_TIMER if wp.0 == TIMER_ALIVE => self.alive(),
        WM_TIMER if wp.0 == TIMER_CYCLE => self.fake_switch(),
        WM_TIMER if wp.0 == TIMER_RECOVER => {
          unsafe {
            let _ = KillTimer(self.msg_hwnd, TIMER_RECOVER);
          }
          self.recover();
        }
        WM_APP_REBUILD => {
          // monitors / DPI change in bursts: rebuild once they settle
          unsafe { SetTimer(self.msg_hwnd, TIMER_REBUILD, 400, None) };
        }
        _ => return None,
      }
      return Some(LRESULT(0));
    }
    let i = self.bars.iter().position(|b| b.hwnd == hwnd)?;
    match msg {
      WM_PAINT => {
        unsafe {
          let _ = windows::Win32::Graphics::Gdi::ValidateRect(hwnd, None);
        }
        Some(LRESULT(0))
      }
      WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
        unsafe {
          let _ = PostMessageW(self.msg_hwnd, WM_APP_REBUILD, WPARAM(0), LPARAM(0));
        }
        None
      }
      WM_MOUSEMOVE => {
        let (x, y) = lparam_point(lp);
        self.mouse_move(i, x, y);
        Some(LRESULT(0))
      }
      WM_MOUSELEAVE => {
        let b = &mut self.bars[i];
        b.tracking = false;
        if b.hover.is_some() || b.hover_left || b.hover_right {
          b.hover = None;
          b.hover_left = false;
          b.hover_right = false;
          self.redraw(i);
        }
        Some(LRESULT(0))
      }
      WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP | WM_LBUTTONDBLCLK => {
        let (x, y) = lparam_point(lp);
        let button = match msg {
          WM_LBUTTONUP => 0,
          WM_RBUTTONUP => 1,
          WM_MBUTTONUP => 2,
          _ => 3,
        };
        self.click(i, x, y, button);
        Some(LRESULT(0))
      }
      WM_MOUSEWHEEL => {
        let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as i32;
        let (sx, sy) = lparam_point(lp);
        let mut p = POINT { x: sx, y: sy };
        unsafe {
          let _ = ScreenToClient(hwnd, &mut p);
        }
        self.wheel(i, p.x, p.y, delta);
        Some(LRESULT(0))
      }
      _ => None,
    }
  }

  fn drain(&mut self) {
    // icons arriving must repaint even when the data did not change
    let mut force = false;
    while let Ok(msg) = self.rx.try_recv() {
      if matches!(msg, Msg::Apps(_) | Msg::WinIcon(..)) {
        force = true;
      }
      match msg {
        Msg::Provider(e) => {
          if let Ok(output) = e.result {
            let tray = matches!(output, ProviderOutput::Systray(_));
            let audio = matches!(output, ProviderOutput::Audio(_));
            self.model.apply(output);
            if tray {
              self.init_pins();
            }
            if audio {
              self.audio_osd();
            }
          }
        }
        Msg::Wm(state) => self.model.wm = state,
        Msg::Apps(apps) => self.icons.set_apps(apps),
        Msg::WinIcon(h, png) => self.icons.set_win_icon(h, png),
        Msg::Display(u) => match u {
          brightness::Update::Brightness(dev, v) => {
            if let Some(d) = self.displays.get_mut(&dev) {
              if d.last_set.elapsed() > Duration::from_secs(5) || d.brightness.is_none() {
                d.brightness = v;
              }
              d.read_done = true;
            }
          }
          brightness::Update::Gamma(dev, g) => {
            if let Some(d) = self.displays.get_mut(&dev) {
              d.gamma = g;
            }
          }
        },
      }
    }
    // providers report every few seconds; most reports change nothing visible
    // (CPU 12.3 % -> 12.4 % still reads 12): those draw nothing
    let key = self.model.visible_key();
    if force || key != self.last_key {
      self.last_key = key;
      self.redraw_all();
    }
  }

  /// Demo only: moves the focus 1 -> 4 -> 2 -> 6 -> 1 without touching the WM.
  fn fake_switch(&mut self) {
    let order = ["1", "4", "2", "6"];
    let Some(mon) = self.model.wm.monitors.iter_mut().find(|m| m.has_focus) else { return };
    let cur = mon.workspaces.iter().find(|w| w.has_focus).map(|w| w.name.clone()).unwrap_or_default();
    let next = order[(order.iter().position(|n| *n == cur).map_or(0, |i| i + 1)) % order.len()];
    for w in mon.workspaces.iter_mut() {
      w.has_focus = w.name == next;
    }
    if !mon.workspaces.iter().any(|w| w.name == next) {
      mon.workspaces.push(wm::WmWorkspace { name: next.into(), has_focus: true, ..Default::default() });
    }
    self.redraw_all();
  }

  /// The core restarts the shell when fewer bars say "alive" than there are
  /// bar windows: sent from this (UI) thread, a hung bar goes quiet.
  fn alive(&self) {
    for b in &self.bars {
      core_api::post_async(format!("/bar-alive?id={}", b.alive_id));
    }
  }

  /// First run: the first icons are pinned (ii SysTray.qml).
  fn init_pins(&mut self) {
    if self.model.pins.is_some() {
      return;
    }
    let Some(tray) = &self.model.systray else { return };
    if tray.icons.is_empty() {
      return;
    }
    let pins: Vec<String> = tray.icons.iter().take(DEFAULT_PINNED).map(pin_key).collect();
    self.save_pins(pins);
  }

  fn save_pins(&mut self, pins: Vec<String>) {
    let _ = std::fs::create_dir_all(state_dir());
    let _ = std::fs::write(&self.pins_file, serde_json::to_string(&pins).unwrap_or_default());
    self.model.pins = Some(pins);
  }

  /// ii: the OSD shows whatever changed the volume (keys, other apps), on one
  /// monitor only -- the bar that was scrolled, else the focused monitor.
  fn audio_osd(&mut self) {
    let Some(audio) = &self.model.audio else { return };
    if let Some(dev) = &audio.default_playback_device {
      let now = (dev.volume, dev.is_muted);
      if let Some(prev) = self.last_volume {
        if prev != now {
          let target = match &self.volume_wheel {
            Some((d, at)) if at.elapsed() < Duration::from_millis(800) => Some(d.clone()),
            _ => None,
          };
          let value = if now.1 { 0 } else { now.0 as i32 };
          self.show_osd(target, OsdKind::Volume, value);
        }
      }
      self.last_volume = Some(now);
    }
    let mic = self.model.audio.as_ref().and_then(|a| a.default_recording_device.as_ref()).map(|m| (m.is_muted, m.volume));
    if let Some((muted, volume)) = mic {
      if self.last_mic.is_some_and(|prev| prev != muted) {
        self.show_osd(None, OsdKind::Mic, if muted { 0 } else { volume as i32 });
      }
      self.last_mic = Some(muted);
    }
  }

  fn create_bars(&mut self) {
    for b in self.bars.drain(..) {
      unsafe {
        let _ = DestroyWindow(b.hwnd);
      }
    }
    let mut monitors: Vec<(HMONITOR, RECT)> = Vec::new();
    unsafe extern "system" fn collect(m: HMONITOR, _: HDC, rc: *mut RECT, data: LPARAM) -> BOOL {
      let v = &mut *(data.0 as *mut Vec<(HMONITOR, RECT)>);
      v.push((m, *rc));
      BOOL(1)
    }
    unsafe {
      let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut monitors as *mut _ as isize));
    }
    let title = if self.demo { format!("{} (native)", TITLE) } else { TITLE.to_string() };
    for (mon, rc) in monitors {
      match self.create_bar(mon, rc, &title) {
        Ok(bar) => {
          if !self.displays.contains_key(&bar.device) {
            let d = Display::new(bar.device.clone());
            d.read(|u| send(Msg::Display(u)));
            self.displays.insert(bar.device.clone(), d);
          }
          self.bars.push(bar);
        }
        Err(err) => tracing::warn!("Native bar: monitor {:?}: {:?}", rc, err),
      }
    }
    self.redraw_all();
  }

  fn create_bar(&mut self, mon: HMONITOR, rc: RECT, title: &str) -> anyhow::Result<Bar> {
    unsafe {
      let (mut dx, mut dy) = (96u32, 96u32);
      let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
      let scale = dx as f32 / 96.0;
      let w = rc.right - rc.left;
      let h = (view::BAR_H * scale).round() as i32;
      let y = rc.top + if self.demo { (45.0 * scale).round() as i32 } else { 0 };
      let mut ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
      if self.demo {
        ex |= WS_EX_TOPMOST;
      }
      let hwnd = CreateWindowExW(
        ex,
        CLASS,
        &HSTRING::from(title),
        WS_POPUP,
        rc.left,
        y,
        w,
        h,
        None,
        None,
        GetModuleHandleW(None)?,
        None,
      )?;
      let target = self.gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let root = self.gfx.dcomp.CreateVisual()?;
      let px = |dip: f32| (dip * scale).round().max(1.0) as u32;
      let bg = Layer::new(&self.gfx, w as u32, h as u32)?;
      let pill = Layer::new(&self.gfx, px(view::TRACK_W), px(view::PILL))?;
      let fg = Layer::new(&self.gfx, px(view::TRACK_W), px(view::TRACK_H))?;
      let pill_clip = self.gfx.dcomp.CreateRectangleClip()?;
      let radius = view::PILL / 2.0 * scale;
      pill_clip.SetTop2(0.0)?;
      pill_clip.SetBottom2(view::PILL * scale)?;
      pill_clip.SetLeft2(0.0)?;
      pill_clip.SetRight2(0.0)?;
      pill_clip.SetTopLeftRadiusX2(radius)?;
      pill_clip.SetTopLeftRadiusY2(radius)?;
      pill_clip.SetTopRightRadiusX2(radius)?;
      pill_clip.SetTopRightRadiusY2(radius)?;
      pill_clip.SetBottomLeftRadiusX2(radius)?;
      pill_clip.SetBottomLeftRadiusY2(radius)?;
      pill_clip.SetBottomRightRadiusX2(radius)?;
      pill_clip.SetBottomRightRadiusY2(radius)?;
      pill.visual.SetClip(&pill_clip)?;
      // with no reference visual, insertAbove = FALSE puts the child on top
      // (TRUE would put it at the bottom): bg, then the pill, then the icons
      for layer in [&bg, &pill, &fg] {
        root.AddVisual(&layer.visual, false, None)?;
      }
      target.SetRoot(&root)?;
      let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
      let alive_id = uuid::Uuid::new_v4().to_string();
      if !self.demo {
        core_api::post_async(format!("/bar-alive?id={}", alive_id));
      }
      Ok(Bar {
        hwnd,
        device: monitor_device(mon),
        monitor: rc,
        scale,
        width: w as f32 / scale,
        _target: target,
        _root: root,
        bg,
        pill,
        pill_clip,
        pill_left: Animated::new(0.0),
        pill_right: Animated::new(0.0),
        pill_color: None,
        pill_idx: None,
        fg,
        alive_id,
        frame: view::Frame::default(),
        hover: None,
        hover_left: false,
        hover_right: false,
        tracking: false,
      })
    }
  }

  fn theme(&self) -> &'static view::Theme {
    if self.model.light {
      &view::LIGHT
    } else {
      &view::DARK
    }
  }

  fn redraw(&mut self, i: usize) {
    if let Err(err) = self.redraw_inner(i) {
      if device_lost(&err) {
        tracing::warn!("Native bar: graphics device lost ({:?}), rebuilding", err);
        unsafe { SetTimer(self.msg_hwnd, TIMER_RECOVER, 50, None) };
      } else {
        tracing::warn!("Native bar draw: {:?}", err);
      }
    }
  }

  fn redraw_inner(&mut self, i: usize) -> windows::core::Result<()> {
    let theme = self.theme();
    let mut requests = Vec::new();
    {
      let Ui { gfx, fonts, res, icons, model, bars, .. } = self;
      let bar = &mut bars[i];
      let s = bar.scale;
      let mut frame = None;
      gfx::draw_surface(&bar.bg.surface, s, |dc| {
        let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
        match view::paint(&mut p, model, theme, bar.width, bar.hover.as_ref(), bar.hover_left, bar.hover_right) {
          Ok(f) => frame = Some(f),
          Err(err) => tracing::warn!("Native bar paint: {:?}", err),
        }
        Ok(())
      })?;
      if let Some(f) = frame {
        bar.frame = f;
      }
      let track = bar.frame.ws_track;
      unsafe {
        bar.fg.visual.SetOffsetX2((track.x * s).round())?;
        bar.fg.visual.SetOffsetY2((track.y * s).round())?;
        bar.pill.visual.SetOffsetX2((track.x * s).round())?;
        bar.pill.visual.SetOffsetY2(((track.y + view::PILL_MARGIN) * s).round())?;
      }
      gfx::draw_surface(&bar.fg.surface, s, |dc| {
        let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
        if let Err(err) = view::paint_ws(&mut p, model, theme, bar.hover.as_ref()) {
          tracing::warn!("Native bar paint (workspaces): {:?}", err);
        }
        Ok(())
      })?;

      // the pill: repaint its colour only when the theme changes
      if bar.pill_color != Some(theme.primary) {
        let color = theme.primary;
        gfx::draw_surface(&bar.pill.surface, s, |dc| unsafe {
          let brush = gfx.brush(color)?;
          dc.FillRectangle(&gfx::Rect::new(0.0, 0.0, view::TRACK_W, view::PILL).d2d(), &brush);
          Ok(())
        })?;
        bar.pill_color = Some(color);
      }
      let idx = bar.frame.ws_idx;
      if idx != bar.pill_idx {
        let (left, right) = match idx {
          Some(k) => {
            let l = (k as f32 * view::CELL + view::PILL_MARGIN) * s;
            (l, l + view::PILL * s)
          }
          None => (0.0, 0.0),
        };
        match (bar.pill_idx, idx) {
          // first position, or the WM (dis)connected: no animation
          (None, _) | (_, None) => unsafe {
            bar.pill_left.set(left);
            bar.pill_right.set(right);
            bar.pill_clip.SetLeft2(left)?;
            bar.pill_clip.SetRight2(right)?;
          },
          (Some(old), Some(new)) => {
            // ii AnimatedTabIndexPair: the edge in front moves in 100 ms, the one behind in 300 ms
            let forward = new > old;
            let (left_ms, right_ms) = if forward { (300.0, 100.0) } else { (100.0, 300.0) };
            if let Some(a) = bar.pill_left.to(&gfx.dcomp, left, left_ms, OUT_SINE)? {
              unsafe { bar.pill_clip.SetLeft(&a)? };
            }
            if let Some(a) = bar.pill_right.to(&gfx.dcomp, right, right_ms, OUT_SINE)? {
              unsafe { bar.pill_clip.SetRight(&a)? };
            }
          }
        }
        bar.pill_idx = idx;
      }
      unsafe { gfx.dcomp.Commit()? };
    }
    for h in requests {
      // the window's own icon (Git Bash, game clients, installers); asked
      // again after 30 s when the core has none
      std::thread::spawn(move || {
        let png = match core_api::post(&format!("/winicon?h={}", h)) {
          Some((200, body)) => icons::data_url_bytes(&String::from_utf8_lossy(&body)),
          _ => None,
        };
        if png.is_none() {
          std::thread::sleep(Duration::from_secs(30));
        }
        send(Msg::WinIcon(h, png));
      });
    }
    Ok(())
  }

  /// The graphics device went away (driver update / reset, GPU removed):
  /// new device, new windows, same state. Retries with a growing pause.
  fn recover(&mut self) {
    match Gfx::new().and_then(|g| Res::new(&g).map(|r| (g, r))) {
      Ok((gfx, res)) => {
        tracing::info!("Native bar: graphics device rebuilt ({})", if gfx.warp { "WARP" } else { "GPU" });
        self.gfx = gfx;
        self.res = res;
        self.icons.clear_bitmaps();
        if let Some(o) = self.osd.take() {
          unsafe {
            let _ = DestroyWindow(o.hwnd);
          }
        }
        self.recover_tries = 0;
        self.create_bars();
      }
      Err(err) => {
        self.recover_tries += 1;
        let wait = (500 * self.recover_tries).min(10_000);
        tracing::warn!("Native bar: device rebuild failed ({:?}), again in {} ms", err, wait);
        unsafe { SetTimer(self.msg_hwnd, TIMER_RECOVER, wait, None) };
      }
    }
  }

  fn redraw_all(&mut self) {
    for i in 0..self.bars.len() {
      self.redraw(i);
    }
  }

  fn show_osd(&mut self, device: Option<String>, kind: OsdKind, value: i32) {
    // no device: the focused monitor's bar (the WM knows), else the first
    let focused = self.model.wm.monitors.iter().find(|m| m.has_focus).map(|m| m.device_name.clone());
    let want = device.or(focused);
    let Some(bar) = want
      .and_then(|d| self.bars.iter().find(|b| b.device.eq_ignore_ascii_case(&d)))
      .or(self.bars.first())
    else {
      return;
    };
    let (scale, mon) = (bar.scale, bar.monitor);
    let w = ((view::OSD_W + 2.0 * view::OSD_PAD) * scale).round() as i32;
    let h = ((view::OSD_H + 2.0 * view::OSD_PAD) * scale).round() as i32;
    let x = mon.left + (mon.right - mon.left - w) / 2;
    let y = mon.top + ((50.0 - view::OSD_PAD) * scale).round() as i32;
    if self.osd.as_ref().map_or(true, |o| o.scale != scale) {
      if let Some(o) = self.osd.take() {
        unsafe {
          let _ = DestroyWindow(o.hwnd);
        }
      }
      match self.create_osd(scale, w, h) {
        Ok(o) => self.osd = Some(o),
        Err(err) => {
          tracing::warn!("Native bar OSD: {:?}", err);
          return;
        }
      }
    }
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, model, osd, .. } = self;
    let Some(osd) = osd else { return };
    let mut requests = Vec::new();
    let drawn = gfx::draw_surface(&osd.layer.surface, scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = view::paint_osd(&mut p, model, theme, kind, value) {
        tracing::warn!("Native bar OSD paint: {:?}", err);
      }
      Ok(())
    });
    if let Err(err) = drawn {
      if device_lost(&err) {
        unsafe { SetTimer(self.msg_hwnd, TIMER_RECOVER, 50, None) };
      }
      return;
    }
    unsafe {
      let _ = gfx.dcomp.Commit();
      let _ = SetWindowPos(osd.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
      SetTimer(self.msg_hwnd, TIMER_OSD, 1000, None); // ii: osd.timeout = 1000
    }
  }

  fn create_osd(&self, scale: f32, w: i32, h: i32) -> anyhow::Result<OsdWin> {
    unsafe {
      let hwnd = CreateWindowExW(
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
        CLASS,
        w!("Logical Lunge · osd"),
        WS_POPUP,
        0,
        0,
        w,
        h,
        None,
        None,
        GetModuleHandleW(None)?,
        None,
      )?;
      let target = self.gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let layer = Layer::new(&self.gfx, w as u32, h as u32)?;
      target.SetRoot(&layer.visual)?;
      Ok(OsdWin { hwnd, scale, _target: target, layer })
    }
  }

  fn dip(&self, i: usize, x: i32, y: i32) -> (f32, f32) {
    let s = self.bars[i].scale;
    (x as f32 / s, y as f32 / s)
  }

  fn mouse_move(&mut self, i: usize, x: i32, y: i32) {
    let (dx, dy) = self.dip(i, x, y);
    let bar = &mut self.bars[i];
    if !bar.tracking {
      let mut t = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: bar.hwnd,
        dwHoverTime: 0,
      };
      unsafe {
        let _ = TrackMouseEvent(&mut t);
      }
      bar.tracking = true;
    }
    let hover = bar.frame.hit(dx, dy).map(|h| h.kind.clone());
    let left = bar.frame.left_zone.contains(dx, dy);
    let right = bar.frame.right_zone.contains(dx, dy);
    if left && !bar.hover_left {
      // brightness may have changed elsewhere (monitor buttons, another app):
      // re-read when the pointer comes to the edge, not on a timer (a
      // PowerShell every 30 s per monitor while nobody looks is wasted work)
      if let Some(d) = self.displays.get(&bar.device) {
        if d.stale() {
          d.read(|u| send(Msg::Display(u)));
        }
      }
    }
    if hover != bar.hover || left != bar.hover_left || right != bar.hover_right {
      bar.hover = hover;
      bar.hover_left = left;
      bar.hover_right = right;
      self.redraw(i);
    }
  }

  fn wm_command(&self, command: String) {
    let _ = self.wm_cmd.send(command);
  }

  fn provider(&self, name: &str, function: ProviderFunction) {
    let manager = self.manager.clone();
    let hash = format!("{}{}", HASH_PREFIX, name);
    self.rt.spawn(async move {
      if let Err(err) = manager.call_function(hash, function).await {
        tracing::warn!("Native bar: provider function: {:?}", err);
      }
    });
  }

  fn slide(&self, target: String) {
    let wm = self.wm_cmd.clone();
    let fallback = match target.as_str() {
      "next" => "command focus --next-workspace".to_string(),
      "prev" => "command focus --prev-workspace".to_string(),
      n => format!("command focus --workspace {}", n),
    };
    core_api::slide(target, move || {
      let _ = wm.send(fallback);
    });
  }

  /// button: 0 left, 1 right, 2 middle, 3 left double
  fn click(&mut self, i: usize, x: i32, y: i32, button: u8) {
    let (dx, dy) = self.dip(i, x, y);
    let Some(kind) = self.bars[i].frame.hit(dx, dy).map(|h| h.kind.clone()) else { return };
    let media = |f: fn(MediaControlArgs) -> MediaFunction| ProviderFunction::Media(f(MediaControlArgs { session_id: None }));
    match (kind, button) {
      (HitKind::Search, 0) => core_api::post_async("/cmd?a=overview".into()),
      (HitKind::ActiveWindow, 0) => core_api::post_async("/cmd?a=emit&e=ll:sidebar-left-toggle".into()),
      (HitKind::Paused, 0) => self.wm_command("command wm-toggle-pause".into()),
      (HitKind::Mode(name), 0) => self.wm_command(format!("command wm-disable-binding-mode --name {}", name)),
      (HitKind::Workspace(n), 0) => self.slide(n.to_string()),
      (HitKind::Workspace(_), 1) => core_api::post_async("/cmd?a=overview".into()),
      (HitKind::Media, 0) => self.provider("media", media(MediaFunction::TogglePlayPause)),
      (HitKind::Media, 1) => self.provider("media", media(MediaFunction::Next)),
      (HitKind::Media, 2) => self.provider("media", media(MediaFunction::Previous)),
      (HitKind::Snip, 0) => core_api::run_core(&["--snip"]),
      (HitKind::Osk, 0) => core_api::post_async("/cmd?a=emit&e=ll:osk-toggle".into()),
      (HitKind::Theme, 0) => {
        self.model.light = !self.model.light;
        self.redraw_all();
      }
      (HitKind::Indicators, 0) => core_api::post_async("/cmd?a=sidebar".into()),
      (HitKind::TrayIcon(id), b) => {
        let args = SystrayIconArgs { icon_id: id };
        let f = match b {
          0 => SystrayFunction::IconLeftClick(args),
          1 => SystrayFunction::IconRightClick(args),
          2 => SystrayFunction::IconMiddleClick(args),
          _ => SystrayFunction::IconLeftDoubleClick(args),
        };
        self.provider("systray", ProviderFunction::Systray(f));
      }
      _ => {}
    }
  }

  fn wheel(&mut self, i: usize, x: i32, y: i32, delta: i32) {
    let (dx, dy) = self.dip(i, x, y);
    let frame = &self.bars[i].frame;
    let up = delta > 0;
    if frame.ws_track.contains(dx, dy) {
      if self.last_ws_wheel.elapsed() < Duration::from_millis(100) {
        return;
      }
      self.last_ws_wheel = Instant::now();
      self.slide(if up { "prev" } else { "next" }.into());
      return;
    }
    let (left, right) = (frame.left_zone.contains(dx, dy), frame.right_zone.contains(dx, dy));
    if !left && !right {
      return;
    }
    if self.last_wheel.elapsed() < Duration::from_millis(40) {
      return;
    }
    self.last_wheel = Instant::now();
    let device = self.bars[i].device.clone();
    if left {
      let Some(d) = self.displays.get_mut(&device) else { return };
      match d.wheel(up) {
        Step::Gamma(v) => self.show_osd(Some(device), OsdKind::Gamma, v),
        Step::Brightness(v) => self.show_osd(Some(device), OsdKind::Brightness, v),
        Step::None => {}
      }
      return;
    }
    let Some(dev) = self.model.audio.as_ref().and_then(|a| a.default_playback_device.clone()) else { return };
    self.volume_wheel = Some((device, Instant::now()));
    let next = (dev.volume as i32 + if up { 5 } else { -5 }).clamp(0, 100);
    self.provider(
      "audio",
      ProviderFunction::Audio(AudioFunction::SetVolume(SetVolumeArgs { volume: next as f32, device_id: None })),
    );
    if dev.is_muted && up {
      self.provider(
        "audio",
        ProviderFunction::Audio(AudioFunction::SetMute(SetMuteArgs { mute: false, device_id: None })),
      );
    }
  }
}

/// Errors that mean the graphics device must be rebuilt.
fn device_lost(err: &windows::core::Error) -> bool {
  matches!(
    err.code().0 as u32,
    0x887A_0005 // DXGI_ERROR_DEVICE_REMOVED
      | 0x887A_0007 // DXGI_ERROR_DEVICE_RESET
      | 0x887A_0020 // DXGI_ERROR_DRIVER_INTERNAL_ERROR
      | 0x8899_000C // D2DERR_RECREATE_TARGET
  )
}
