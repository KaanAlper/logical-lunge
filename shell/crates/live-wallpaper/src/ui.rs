//! The UI thread: a hidden top-level window for notifications (the core's
//! reload and stop, Explorer restarting, monitors changing, lock, display
//! and power state), one window per monitor that shows a wallpaper, and
//! the pause rules. Windows start invisible and fade in on their first
//! frame, so a starting video never flashes black.
//!
//! The player runs while a wallpaper is set. A monitor that is not there
//! (an undocked laptop), settings being rewritten, Explorer restarting or
//! the video thread failing never end it: it waits, rebuilds or starts the
//! thread again. It ends when the core clears the settings (reload) or
//! stops the desktop (WM_CLOSE).

use std::{
  cell::{Cell, RefCell},
  path::PathBuf,
  sync::{
    atomic::{AtomicIsize, AtomicU32, Ordering},
    mpsc::{self, Sender},
  },
  thread::JoinHandle,
  time::{Duration, Instant},
};

use windows::{
  core::{w, HSTRING, PCWSTR},
  Win32::{
    Foundation::{
      BOOL, COLORREF, HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM,
    },
    Graphics::Gdi::{
      EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW,
      MonitorFromWindow, DISPLAY_DEVICEW, HDC, HMONITOR, MONITORINFO,
      MONITORINFOEXW, MONITOR_DEFAULTTONULL,
    },
    System::{
      LibraryLoader::GetModuleHandleW,
      Power::{
        GetSystemPowerStatus, RegisterPowerSettingNotification,
        POWERBROADCAST_SETTING, SYSTEM_POWER_STATUS,
      },
      RemoteDesktop::{
        WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
      },
      StationsAndDesktops::{
        CloseDesktop, OpenInputDesktop, DESKTOP_CONTROL_FLAGS,
        DESKTOP_READOBJECTS,
      },
      SystemServices::{
        GUID_ACDC_POWER_SOURCE, GUID_CONSOLE_DISPLAY_STATE,
      },
    },
    UI::{
      Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
      WindowsAndMessaging::*,
    },
  },
};

use crate::{
  config::Config,
  desktop::{self, Layer},
  log,
  render::{self, Cmd, Target, WM_APP_FIRST_FRAME, WM_APP_RENDER_DIED},
};

/// The core finds this window (FindWindow) to have the settings read again
/// (WM_APP_RELOAD) or to stop the player (WM_CLOSE).
const MSG_CLASS: PCWSTR = w!("LogicalLunge.LiveWallpaper");
const SCREEN_CLASS: PCWSTR = w!("LogicalLunge.LiveWallpaper.Screen");
pub const WM_APP_RELOAD: u32 = WM_APP + 1;
/// is a fullscreen app on a monitor, are the windows still in place (and
/// the other pause rules, again)
const TIMER_CHECK: usize = 1;
/// Explorer replaced the WorkerW: back under the icons
const TIMER_RELAYER: usize = 2;
/// settings unreadable for a moment, Explorer restarted, a window lost
const TIMER_REBUILD: usize = 3;
const TIMER_FADE: usize = 4;
/// the video thread ended without being asked: a new one
const TIMER_RENDER: usize = 5;
/// monitors changed (they change in bursts)
const TIMER_DISPLAY: usize = 6;
const PBT_POWERSETTINGCHANGE: u32 = 0x8013;
const WTS_CONSOLE_CONNECT: u32 = 1;
const WTS_CONSOLE_DISCONNECT: u32 = 2;
const WTS_REMOTE_CONNECT: u32 = 3;
const WTS_REMOTE_DISCONNECT: u32 = 4;
const WTS_SESSION_LOCK: u32 = 7;
const WTS_SESSION_UNLOCK: u32 = 8;
/// video threads started in ten minutes before the player stops starting
/// them (until the next reload)
const RENDER_STARTS: usize = 5;
/// ms before a rebuild for windows out of place, doubled each time they
/// stay so
const REBUILD_FIRST: u32 = 300;
const REBUILD_MAX: u32 = 30_000;

#[derive(Clone, Copy, PartialEq)]
enum Quit {
  /// the core stopped the desktop
  Stopped,
  /// no wallpaper is set any more
  Unset,
}

struct Screen {
  /// the video thread's name for it (window handles are reused)
  id: u64,
  rect: RECT,
  hwnd: HWND,
  file: PathBuf,
  covered: bool,
}

struct Render {
  tx: Sender<Cmd>,
  thread: Option<JoinHandle<()>>,
}

struct App {
  msg: HWND,
  render: Option<Render>,
  render_starts: Vec<Instant>,
  config: Config,
  screens: Vec<Screen>,
  /// the monitors at the last rebuild: (device, id, rectangle)
  monitors: Vec<(String, String, RECT)>,
  layer: Layer,
  hook: Option<HWINEVENTHOOK>,
  last_paused: Vec<bool>,
  next_id: u64,
  /// logged once until it changes
  unreadable: bool,
  missing: bool,
  /// a rebuild is waiting on its timer, and how long the next one waits
  /// (longer while windows keep failing to stay in place)
  rebuild_pending: bool,
  rebuild_delay: u32,
}

thread_local! {
  static APP: RefCell<Option<App>> = const { RefCell::new(None) };
  static LOCKED: Cell<bool> = const { Cell::new(false) };
  static DISCONNECTED: Cell<bool> = const { Cell::new(false) };
  static DISPLAY_OFF: Cell<bool> = const { Cell::new(false) };
  static ON_BATTERY: Cell<bool> = const { Cell::new(false) };
  static QUIT: Cell<Option<Quit>> = const { Cell::new(None) };
}
static MSG_HWND: AtomicIsize = AtomicIsize::new(0);
static WORKERW: AtomicIsize = AtomicIsize::new(0);
static PROGMAN: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// Runs `f` on the app; skipped when it is already in use further up the
/// stack (a message sent to us while a rebuild waits for Explorer): the
/// half-second check catches up.
fn with_app(f: impl FnOnce(&mut App)) {
  APP.with(|cell| {
    if let Ok(mut app) = cell.try_borrow_mut() {
      if let Some(app) = app.as_mut() {
        f(app);
      }
    }
  });
}

fn quit(why: Quit) {
  QUIT.set(Some(why));
  unsafe { PostQuitMessage(0) };
}

pub fn run() {
  unsafe {
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let mut class = WNDCLASSEXW {
      cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
      lpfnWndProc: Some(msg_proc),
      hInstance: instance.into(),
      lpszClassName: MSG_CLASS,
      ..Default::default()
    };
    RegisterClassExW(&class);
    class.lpfnWndProc = Some(screen_proc);
    class.lpszClassName = SCREEN_CLASS;
    RegisterClassExW(&class);
    let Ok(msg) = CreateWindowExW(
      WS_EX_TOOLWINDOW,
      MSG_CLASS,
      w!("Logical Lunge live wallpaper"),
      WS_POPUP,
      0,
      0,
      0,
      0,
      None,
      None,
      instance,
      None,
    ) else {
      log::line("no message window");
      return;
    };
    MSG_HWND.store(msg.0 as isize, Ordering::Release);
    TASKBAR_CREATED.store(
      RegisterWindowMessageW(w!("TaskbarCreated")),
      Ordering::Release,
    );
    // started with administrator rights, the core's and Explorer's
    // messages would be filtered out
    for m in [
      WM_APP_RELOAD,
      WM_CLOSE,
      TASKBAR_CREATED.load(Ordering::Acquire),
    ] {
      let _ = ChangeWindowMessageFilterEx(msg, m, MSGFLT_ALLOW, None);
    }
    let _ = WTSRegisterSessionNotification(msg, NOTIFY_FOR_THIS_SESSION);
    let _ = RegisterPowerSettingNotification(
      HANDLE(msg.0),
      &GUID_CONSOLE_DISPLAY_STATE,
      DEVICE_NOTIFY_WINDOW_HANDLE,
    );
    let _ = RegisterPowerSettingNotification(
      HANDLE(msg.0),
      &GUID_ACDC_POWER_SOURCE,
      DEVICE_NOTIFY_WINDOW_HANDLE,
    );
    let mut power = SYSTEM_POWER_STATUS::default();
    if GetSystemPowerStatus(&mut power).is_ok() {
      ON_BATTERY.set(power.ACLineStatus == 0);
    }
    // the core may start it while the session is locked (a reload)
    LOCKED.set(session_locked());
    SetTimer(msg, TIMER_CHECK, 500, None);

    APP.with(|a| {
      *a.borrow_mut() = Some(App {
        msg,
        render: None,
        render_starts: Vec::new(),
        config: Config::default(),
        screens: Vec::new(),
        monitors: Vec::new(),
        layer: Layer::Bare,
        hook: None,
        last_paused: Vec::new(),
        next_id: 0,
        unreadable: false,
        missing: false,
        rebuild_pending: false,
        rebuild_delay: REBUILD_FIRST,
      })
    });
    with_app(|a| a.rebuild());

    let mut m = MSG::default();
    loop {
      let got = GetMessageW(&mut m, None, 0, 0).0;
      if got == 0 || got == -1 {
        break;
      }
      let _ = TranslateMessage(&m);
      DispatchMessageW(&m);
    }

    // the core no longer finds this process; then the video stops before
    // the windows go
    MSG_HWND.store(0, Ordering::Release);
    let _ = DestroyWindow(msg);
    let app = APP.with(|a| a.borrow_mut().take());
    if let Some(mut app) = app {
      app.stop_render();
      if let Some(h) = app.hook.take() {
        let _ = UnhookWinEvent(h);
      }
      for s in app.screens.drain(..) {
        let _ = DestroyWindow(s.hwnd);
      }
    }
  }
  // Ended because nothing was set, but a wallpaper was set again in the
  // meantime (its reload came while this one was closing): a new player
  // takes over (it waits for this one's mutex).
  if QUIT.get() == Some(Quit::Unset)
    && Config::load().is_some_and(|c| c.active())
  {
    if let Ok(exe) = std::env::current_exe() {
      let _ = std::process::Command::new(exe).spawn();
    }
  }
}

impl App {
  /// Settings read again, windows made again: one per monitor that has a
  /// video.
  fn rebuild(&mut self) {
    self.rebuild_pending = false;
    match Config::load() {
      Some(c) => {
        self.config = c;
        self.unreadable = false;
      }
      None => {
        // being rewritten, or held by a scanner: the current ones stay
        if !self.unreadable {
          log::line("settings unreadable; trying again");
          self.unreadable = true;
        }
        unsafe { SetTimer(self.msg, TIMER_REBUILD, 1000, None) };
        return;
      }
    }
    if !self.config.active() {
      // the core cleared it: the static wallpaper (a frame of the video)
      // stays on the desktop
      quit(Quit::Unset);
      return;
    }
    self.release();
    for s in self.screens.drain(..) {
      unsafe {
        let _ = DestroyWindow(s.hwnd);
      }
    }
    self.layer = desktop::find_layer();
    self.hook_explorer();
    self.monitors = monitors();
    if self.layer == Layer::Missing {
      // Explorer is there but its desktop is not (yet) as expected
      if !self.missing {
        log::line(
          "the desktop's windows are not as expected; trying again",
        );
        self.missing = true;
      }
      unsafe { SetTimer(self.msg, TIMER_REBUILD, 2000, None) };
      return;
    }
    self.missing = false;
    for (device, id, rect) in self.monitors.clone() {
      let Some(file) = self.config.file_for(&[&device, &id]).cloned()
      else {
        continue;
      };
      if !file.is_file() {
        log::line(&format!("missing video {}", file.display()));
        continue;
      }
      let Some(hwnd) = create_screen(rect) else {
        continue;
      };
      if !desktop::attach(hwnd, self.layer, rect) {
        log::line(&format!(
          "could not put the wallpaper of {device} behind the icons"
        ));
      }
      self.next_id += 1;
      self.screens.push(Screen {
        id: self.next_id,
        rect,
        hwnd,
        file,
        covered: false,
      });
    }
    // no monitor of the settings is there (undocked): the player waits for
    // the monitors to change
    if self.render.is_none() {
      self.start_render();
    }
    self.send_targets();
    self.last_paused.clear();
    // a game may be on screen already
    self.check();
  }

  fn send_targets(&mut self) {
    let targets = self
      .screens
      .iter()
      .map(|s| Target {
        id: s.id,
        hwnd: s.hwnd.0 as isize,
        width: (s.rect.right - s.rect.left) as u32,
        height: (s.rect.bottom - s.rect.top) as u32,
        file: s.file.clone(),
      })
      .collect();
    self.send(Cmd::Targets(targets));
  }

  fn send(&mut self, cmd: Cmd) {
    let Some(r) = &self.render else { return };
    if r.tx.send(cmd).is_err() {
      // the thread is gone: a new one shortly
      unsafe { SetTimer(self.msg, TIMER_RENDER, 3000, None) };
    }
  }

  /// The video thread lets go of every window (before they are
  /// destroyed): answered, or after a second.
  fn release(&mut self) {
    let Some(r) = &self.render else { return };
    let (done, wait) = mpsc::sync_channel(1);
    if r.tx.send(Cmd::Release(done)).is_ok() {
      let _ = wait.recv_timeout(Duration::from_secs(1));
    }
  }

  fn start_render(&mut self) {
    let now = Instant::now();
    self
      .render_starts
      .retain(|t| now.duration_since(*t) < Duration::from_secs(600));
    if self.render_starts.len() >= RENDER_STARTS {
      log::line("the video thread keeps ending; not started again until the next reload");
      return;
    }
    self.render_starts.push(now);
    let (tx, rx) = mpsc::channel();
    let ui = self.msg.0 as isize;
    let thread = std::thread::Builder::new()
      .name("render".into())
      .spawn(move || {
        // a panic must not leave a player that holds the wallpaper and
        // draws nothing
        let asked =
          std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render::run(rx, ui)
          }))
          .unwrap_or(false);
        if !asked {
          unsafe {
            let _ = PostMessageW(
              HWND(ui as _),
              WM_APP_RENDER_DIED,
              WPARAM(0),
              LPARAM(0),
            );
          }
        }
      })
      .ok();
    self.render = Some(Render { tx, thread });
  }

  /// Stops the video thread, waiting at most two seconds (a driver stuck
  /// in a call must not keep a closing player alive).
  fn stop_render(&mut self) {
    let Some(mut r) = self.render.take() else {
      return;
    };
    let _ = r.tx.send(Cmd::Quit);
    let Some(thread) = r.thread.take() else {
      return;
    };
    let until = Instant::now() + Duration::from_secs(2);
    while !thread.is_finished() && Instant::now() < until {
      std::thread::sleep(Duration::from_millis(20));
    }
    if thread.is_finished() {
      let _ = thread.join();
    }
  }

  fn restart_render(&mut self) {
    self.stop_render();
    self.start_render();
    self.send_targets();
    self.last_paused.clear();
    self.update_pause();
  }

  /// Explorer's window events only (not every window of the system): a
  /// new or vanished desktop WorkerW.
  fn hook_explorer(&mut self) {
    unsafe {
      if let Some(h) = self.hook.take() {
        let _ = UnhookWinEvent(h);
      }
      WORKERW.store(
        match self.layer {
          Layer::Raised { workerw, .. } | Layer::Classic { workerw } => {
            workerw
          }
          Layer::Bare | Layer::Missing => 0,
        },
        Ordering::Release,
      );
      let progman =
        FindWindowW(w!("Progman"), PCWSTR::null()).unwrap_or_default();
      PROGMAN.store(progman.0 as isize, Ordering::Release);
      if progman.is_invalid() {
        return;
      }
      let mut pid = 0u32;
      GetWindowThreadProcessId(progman, Some(&mut pid));
      let h = SetWinEventHook(
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_DESTROY,
        None,
        Some(on_event),
        pid,
        0,
        WINEVENT_OUTOFCONTEXT,
      );
      if !h.is_invalid() {
        self.hook = Some(h);
      }
    }
  }

  /// Every window alive and a child of the window the layout puts it in.
  fn in_place(&self, layer: Layer) -> bool {
    let parent = desktop::parent(layer);
    self.screens.iter().all(|s| unsafe {
      IsWindow(s.hwnd).as_bool()
        && GetParent(s.hwnd).unwrap_or_default().0 as isize == parent
    })
  }

  fn relayer(&mut self) {
    let layer = desktop::find_layer();
    // an unrelated WorkerW came and went
    if layer == self.layer && self.in_place(layer) {
      return;
    }
    // a window went with Explorer's (or the layout is not ready): all
    // again
    let gone = self
      .screens
      .iter()
      .any(|s| unsafe { !IsWindow(s.hwnd).as_bool() });
    if gone || layer == Layer::Missing {
      self.rebuild();
      return;
    }
    self.layer = layer;
    WORKERW.store(
      match layer {
        Layer::Raised { workerw, .. } | Layer::Classic { workerw } => {
          workerw
        }
        Layer::Bare | Layer::Missing => 0,
      },
      Ordering::Release,
    );
    let all = self
      .screens
      .iter()
      .all(|s| desktop::attach(s.hwnd, layer, s.rect));
    if !all {
      self.schedule_rebuild();
    }
  }

  /// A rebuild for windows out of place: once at a time, later each time.
  fn schedule_rebuild(&mut self) {
    if self.rebuild_pending {
      return;
    }
    self.rebuild_pending = true;
    unsafe { SetTimer(self.msg, TIMER_REBUILD, self.rebuild_delay, None) };
    self.rebuild_delay = (self.rebuild_delay * 2).min(REBUILD_MAX);
  }

  fn check(&mut self) {
    // a window lost or moved out from under the icons (in the classic
    // layout ours go with the WorkerW Explorer replaces)
    if !self.screens.is_empty() && !self.in_place(self.layer) {
      self.schedule_rebuild();
    } else if !self.rebuild_pending {
      self.rebuild_delay = REBUILD_FIRST;
    }
    // locked without the unlock reaching us (started while locked): the
    // input desktop tells
    if LOCKED.get() && !session_locked() {
      LOCKED.set(false);
    }
    let covered = unsafe { fullscreen_monitor() };
    for s in &mut self.screens {
      s.covered = covered.is_some_and(|r| r == s.rect);
    }
    self.update_pause();
  }

  /// Tells the video thread when something changed.
  fn update_pause(&mut self) {
    let all = LOCKED.get()
      || DISCONNECTED.get()
      || DISPLAY_OFF.get()
      || (self.config.pause_on_battery && ON_BATTERY.get());
    let paused: Vec<bool> = self
      .screens
      .iter()
      .map(|s| all || (self.config.pause_fullscreen && s.covered))
      .collect();
    if paused != self.last_paused {
      self.last_paused = paused.clone();
      self.send(Cmd::Paused(paused));
    }
  }
}

fn create_screen(rect: RECT) -> Option<HWND> {
  unsafe {
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let hwnd = CreateWindowExW(
      WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
      SCREEN_CLASS,
      w!(""),
      WS_POPUP,
      rect.left,
      rect.top,
      rect.right - rect.left,
      rect.bottom - rect.top,
      None,
      None,
      instance,
      None,
    )
    .ok()?;
    Some(hwnd)
  }
}

/// The input desktop is Winlogon's while the session is locked (or the
/// secure desktop shows), which a user's process cannot open.
fn session_locked() -> bool {
  unsafe {
    match OpenInputDesktop(
      DESKTOP_CONTROL_FLAGS(0),
      false,
      DESKTOP_READOBJECTS,
    ) {
      Ok(desk) => {
        let _ = CloseDesktop(desk);
        false
      }
      Err(_) => true,
    }
  }
}

fn wide(s: &[u16]) -> String {
  String::from_utf16_lossy(
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())],
  )
}

/// (GDI device name, device interface path, rectangle in screen pixels) of
/// every monitor. The interface path is the id IDesktopWallpaper (the
/// static wallpaper) gives the monitor.
fn monitors() -> Vec<(String, String, RECT)> {
  unsafe extern "system" fn each(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    lp: LPARAM,
  ) -> BOOL {
    let list = &mut *(lp.0 as *mut Vec<(String, String, RECT)>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(
      monitor,
      &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
    )
    .as_bool()
    {
      let device = wide(&info.szDevice);
      let mut dd = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
      };
      let name = HSTRING::from(device.as_str());
      let id = if EnumDisplayDevicesW(
        &name,
        0,
        &mut dd,
        EDD_GET_DEVICE_INTERFACE_NAME,
      )
      .as_bool()
      {
        wide(&dd.DeviceID)
      } else {
        String::new()
      };
      list.push((device, id, info.monitorInfo.rcMonitor));
    }
    BOOL(1)
  }
  let mut list: Vec<(String, String, RECT)> = Vec::new();
  unsafe {
    let _ = EnumDisplayMonitors(
      HDC::default(),
      None,
      Some(each),
      LPARAM(&mut list as *mut _ as isize),
    );
  }
  list
}

/// The monitor a fullscreen app covers (games, videos in fullscreen): the
/// foreground window spans it entirely. Maximized windows leave the bar's
/// space free, so they do not count.
unsafe fn fullscreen_monitor() -> Option<RECT> {
  let fg = GetForegroundWindow();
  if fg.is_invalid() {
    return None;
  }
  let mut class = [0u16; 64];
  let n = GetClassNameW(fg, &mut class).max(0) as usize;
  let class = String::from_utf16_lossy(&class[..n]);
  if matches!(
    class.as_str(),
    "Progman"
      | "WorkerW"
      | "Shell_TrayWnd"
      | "LogicalLunge.LiveWallpaper.Screen"
  ) {
    return None;
  }
  let mut r = RECT::default();
  GetWindowRect(fg, &mut r).ok()?;
  let monitor = MonitorFromWindow(fg, MONITOR_DEFAULTTONULL);
  if monitor.is_invalid() {
    return None;
  }
  let mut info = MONITORINFO {
    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
    ..Default::default()
  };
  if !GetMonitorInfoW(monitor, &mut info).as_bool() {
    return None;
  }
  let m = info.rcMonitor;
  (r.left <= m.left
    && r.top <= m.top
    && r.right >= m.right
    && r.bottom >= m.bottom)
    .then_some(m)
}

unsafe extern "system" fn on_event(
  _: HWINEVENTHOOK,
  _event: u32,
  hwnd: HWND,
  object: i32,
  _child: i32,
  _: u32,
  _: u32,
) {
  if object != OBJID_WINDOW.0 {
    return;
  }
  // ours went away, or a desktop WorkerW came or went: top-level or a
  // child of Progman, not the one in every File Explorer window
  if hwnd.0 as isize != WORKERW.load(Ordering::Acquire) {
    let mut class = [0u16; 16];
    let n = GetClassNameW(hwnd, &mut class).max(0) as usize;
    if class[..n] != *"WorkerW".encode_utf16().collect::<Vec<u16>>() {
      return;
    }
    let parent = GetParent(hwnd).unwrap_or_default().0 as isize;
    if parent != 0 && parent != PROGMAN.load(Ordering::Acquire) {
      return;
    }
  }
  // after Explorer settles
  SetTimer(
    HWND(MSG_HWND.load(Ordering::Acquire) as _),
    TIMER_RELAYER,
    300,
    None,
  );
}

unsafe extern "system" fn msg_proc(
  hwnd: HWND,
  msg: u32,
  wp: WPARAM,
  lp: LPARAM,
) -> LRESULT {
  match msg {
    WM_TIMER => {
      match wp.0 {
        TIMER_CHECK => with_app(|a| a.check()),
        TIMER_RELAYER => {
          let _ = KillTimer(hwnd, TIMER_RELAYER);
          with_app(|a| a.relayer());
        }
        TIMER_REBUILD => {
          let _ = KillTimer(hwnd, TIMER_REBUILD);
          with_app(|a| a.rebuild());
        }
        TIMER_RENDER => {
          let _ = KillTimer(hwnd, TIMER_RENDER);
          with_app(|a| a.restart_render());
        }
        TIMER_DISPLAY => {
          let _ = KillTimer(hwnd, TIMER_DISPLAY);
          // a game switching modes leaves the monitors as they were
          with_app(|a| {
            if monitors() != a.monitors {
              a.rebuild();
            } else {
              a.check();
            }
          });
        }
        _ => {}
      }
      LRESULT(0)
    }
    WM_APP_RELOAD => {
      with_app(|a| {
        // a reload also gives a stopped video thread another chance
        a.render_starts.clear();
        a.rebuild();
      });
      LRESULT(0)
    }
    WM_APP_FIRST_FRAME => LRESULT(0),
    WM_APP_RENDER_DIED => {
      SetTimer(hwnd, TIMER_RENDER, 3000, None);
      LRESULT(0)
    }
    WM_DISPLAYCHANGE => {
      SetTimer(hwnd, TIMER_DISPLAY, 500, None);
      LRESULT(0)
    }
    WM_WTSSESSION_CHANGE => {
      match wp.0 as u32 {
        WTS_SESSION_LOCK => LOCKED.set(true),
        WTS_SESSION_UNLOCK => LOCKED.set(false),
        // a remote session nobody is looking at
        WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT => {
          DISCONNECTED.set(true)
        }
        WTS_CONSOLE_CONNECT | WTS_REMOTE_CONNECT => {
          DISCONNECTED.set(false)
        }
        _ => {}
      }
      with_app(|a| a.update_pause());
      LRESULT(0)
    }
    WM_POWERBROADCAST
      if wp.0 as u32 == PBT_POWERSETTINGCHANGE && lp.0 != 0 =>
    {
      let setting = &*(lp.0 as *const POWERBROADCAST_SETTING);
      if setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
        // 0 off, 1 on, 2 dimmed
        DISPLAY_OFF.set(setting.Data[0] == 0);
      } else if setting.PowerSetting == GUID_ACDC_POWER_SOURCE {
        // 0 mains, 1 battery, 2 short-term (UPS)
        ON_BATTERY.set(setting.Data[0] != 0);
      }
      with_app(|a| a.update_pause());
      LRESULT(1)
    }
    WM_CLOSE => {
      quit(Quit::Stopped);
      LRESULT(0)
    }
    _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Acquire) => {
      // Explorer restarted: its desktop windows are new (and ours went
      // with the old ones)
      SetTimer(hwnd, TIMER_REBUILD, 1000, None);
      LRESULT(0)
    }
    _ => DefWindowProcW(hwnd, msg, wp, lp),
  }
}

unsafe extern "system" fn screen_proc(
  hwnd: HWND,
  msg: u32,
  wp: WPARAM,
  lp: LPARAM,
) -> LRESULT {
  match msg {
    WM_ERASEBKGND => LRESULT(1),
    WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
    WM_WINDOWPOSCHANGING if lp.0 != 0 => {
      // without Explorer the window is top-level: it stays under
      // everything
      if GetWindowLongPtrW(hwnd, GWL_STYLE) & WS_CHILD.0 as isize == 0 {
        let pos = &mut *(lp.0 as *mut WINDOWPOS);
        pos.hwndInsertAfter = HWND_BOTTOM;
        pos.flags &= !SWP_NOZORDER;
      }
      LRESULT(0)
    }
    _ => DefWindowProcW(hwnd, msg, wp, lp),
  }
}
