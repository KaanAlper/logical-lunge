//! `LogicalLunge.scr`: the player as a Windows screen saver (a copy of
//! lunge-wallpaper.exe under that name; the build makes it). Windows starts
//! it with `/s` (show), `/p <window>` (the small preview in the screen
//! saver dialog) or `/c` (settings; no argument means the same). It plays
//! a video chosen in Logical Lunge (state\screensaver-video.json,
//! `{"videos":[paths],"shuffle":bool}`; several videos: one at random) on
//! every monitor with the live wallpaper's renderer, and ends at the first
//! key, click or real mouse movement.

use std::{
  path::PathBuf,
  sync::mpsc,
  sync::atomic::{AtomicI32, Ordering},
  time::{SystemTime, UNIX_EPOCH},
};

use windows::{
  core::{w, PCWSTR},
  Win32::{
    Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
      EnumDisplayMonitors, FillRect, GetStockObject, BeginPaint, EndPaint,
      BLACK_BRUSH, HBRUSH, HDC, HMONITOR, PAINTSTRUCT,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
      HiDpi::{
        SetProcessDpiAwarenessContext,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
      },
      WindowsAndMessaging::*,
    },
  },
};

use crate::{config::data_dir, log, render};

/// How far the mouse may drift (a desk bump) before the saver ends.
const MOVE_SLACK: i32 = 8;

#[derive(Debug, PartialEq)]
enum Mode {
  Show,
  Preview(isize),
  Configure,
  /// `/a` (change password, Windows 9x): nothing to do
  Nothing,
}

fn mode(args: &[String]) -> Mode {
  let Some(first) = args.get(1) else { return Mode::Configure };
  let flag = first.to_ascii_lowercase();
  // "/p 1234", "/p:1234", "-p 1234"
  let (name, inline) = match flag.split_once(':') {
    Some((n, v)) => (n.to_string(), Some(v.to_string())),
    None => (flag.clone(), None),
  };
  match name.trim_start_matches(['/', '-']) {
    "s" => Mode::Show,
    "p" | "l" => inline
      .or_else(|| args.get(2).cloned())
      .and_then(|v| v.trim().parse::<isize>().ok())
      .map_or(Mode::Nothing, Mode::Preview),
    "a" => Mode::Nothing,
    _ => Mode::Configure,
  }
}

/// Started as a screen saver: by its .scr name, or with a saver switch.
pub fn wanted(args: &[String]) -> bool {
  let scr = std::env::current_exe()
    .ok()
    .and_then(|p| p.extension().map(|e| e.eq_ignore_ascii_case("scr")))
    .unwrap_or(false);
  scr
    || args.get(1).is_some_and(|a| {
      let a = a.to_ascii_lowercase();
      a == "/s" || a.starts_with("/p") || a.starts_with("/c")
    })
}

/// The videos chosen for the saver that are still there.
fn videos() -> (Vec<PathBuf>, bool) {
  let path = data_dir().join("state").join("screensaver-video.json");
  let Ok(text) = std::fs::read_to_string(&path) else {
    return (Vec::new(), true);
  };
  let text = text.trim_start_matches('\u{feff}');
  let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
    log::line("screen saver settings unreadable");
    return (Vec::new(), true);
  };
  let list = v["videos"]
    .as_array()
    .map(|a| {
      a.iter()
        .filter_map(|x| x.as_str())
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect()
    })
    .unwrap_or_default();
  (list, v["shuffle"].as_bool() != Some(false))
}

fn pick(list: &[PathBuf], shuffle: bool) -> Option<PathBuf> {
  if list.is_empty() {
    return None;
  }
  let i = if shuffle {
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.subsec_nanos() as usize ^ d.as_secs() as usize)
      .unwrap_or(0);
    nanos % list.len()
  } else {
    0
  };
  Some(list[i].clone())
}

static START_X: AtomicI32 = AtomicI32::new(i32::MIN);
static START_Y: AtomicI32 = AtomicI32::new(i32::MIN);

const SAVER_CLASS: PCWSTR = w!("LogicalLunge.ScreenSaver");

unsafe extern "system" fn saver_proc(
  hwnd: HWND,
  msg: u32,
  wp: WPARAM,
  lp: LPARAM,
) -> LRESULT {
  match msg {
    WM_SETCURSOR => {
      SetCursor(None);
      LRESULT(1)
    }
    WM_ERASEBKGND => LRESULT(1),
    WM_PAINT => {
      // black until the first frame (and around a video that fails)
      let mut ps = PAINTSTRUCT::default();
      let dc = BeginPaint(hwnd, &mut ps);
      FillRect(dc, &ps.rcPaint, HBRUSH(GetStockObject(BLACK_BRUSH).0));
      let _ = EndPaint(hwnd, &ps);
      LRESULT(0)
    }
    WM_MOUSEMOVE => {
      let mut p = POINT::default();
      let _ = GetCursorPos(&mut p);
      let (x, y) = (START_X.load(Ordering::Relaxed), START_Y.load(Ordering::Relaxed));
      if x == i32::MIN {
        START_X.store(p.x, Ordering::Relaxed);
        START_Y.store(p.y, Ordering::Relaxed);
      } else if (p.x - x).abs() > MOVE_SLACK || (p.y - y).abs() > MOVE_SLACK {
        PostQuitMessage(0);
      }
      LRESULT(0)
    }
    WM_KEYDOWN | WM_SYSKEYDOWN | WM_LBUTTONDOWN | WM_RBUTTONDOWN
    | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_MOUSEWHEEL => {
      PostQuitMessage(0);
      LRESULT(0)
    }
    WM_ACTIVATEAPP if wp.0 == 0 => {
      PostQuitMessage(0);
      LRESULT(0)
    }
    // the screen saver dialog closes the preview by destroying its window
    WM_DESTROY => {
      PostQuitMessage(0);
      LRESULT(0)
    }
    // SC_SCREENSAVE: Windows must not start another saver over this one
    WM_SYSCOMMAND if wp.0 & 0xFFF0 == 0xF140 => LRESULT(0),
    _ => DefWindowProcW(hwnd, msg, wp, lp),
  }
}

unsafe extern "system" fn preview_proc(
  hwnd: HWND,
  msg: u32,
  wp: WPARAM,
  lp: LPARAM,
) -> LRESULT {
  match msg {
    WM_ERASEBKGND => LRESULT(1),
    WM_PAINT => {
      let mut ps = PAINTSTRUCT::default();
      let dc = BeginPaint(hwnd, &mut ps);
      FillRect(dc, &ps.rcPaint, HBRUSH(GetStockObject(BLACK_BRUSH).0));
      let _ = EndPaint(hwnd, &ps);
      LRESULT(0)
    }
    WM_DESTROY => {
      PostQuitMessage(0);
      LRESULT(0)
    }
    _ => DefWindowProcW(hwnd, msg, wp, lp),
  }
}

fn monitor_rects() -> Vec<RECT> {
  unsafe extern "system" fn each(
    _: HMONITOR,
    _: HDC,
    r: *mut RECT,
    lp: LPARAM,
  ) -> BOOL {
    (*(lp.0 as *mut Vec<RECT>)).push(*r);
    BOOL(1)
  }
  let mut list: Vec<RECT> = Vec::new();
  unsafe {
    let _ = EnumDisplayMonitors(
      HDC::default(),
      None,
      Some(each),
      LPARAM(&mut list as *mut Vec<RECT> as isize),
    );
  }
  list
}

/// Plays `file` in `windows` until the message loop ends.
fn play(windows: &[(HWND, RECT)], file: Option<PathBuf>) {
  let ui = windows.first().map(|w| w.0 .0 as isize).unwrap_or(0);
  let (tx, rx) = mpsc::channel();
  let thread = file.map(|file| {
    let targets: Vec<render::Target> = windows
      .iter()
      .enumerate()
      .map(|(i, (hwnd, r))| render::Target {
        id: i as u64 + 1,
        hwnd: hwnd.0 as isize,
        width: (r.right - r.left).max(1) as u32,
        height: (r.bottom - r.top).max(1) as u32,
        file: file.clone(),
      })
      .collect();
    let _ = tx.send(render::Cmd::Targets(targets));
    std::thread::spawn(move || {
      let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render::run(rx, ui)
      }));
    })
  });
  unsafe {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
      let _ = TranslateMessage(&msg);
      DispatchMessageW(&msg);
    }
  }
  let _ = tx.send(render::Cmd::Quit);
  if let Some(t) = thread {
    // a driver stuck in a call must not keep the saver on screen
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !t.is_finished() && std::time::Instant::now() < until {
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if t.is_finished() {
      let _ = t.join();
    }
  }
}

fn register(class: PCWSTR, proc_: WNDPROC) {
  unsafe {
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let wc = WNDCLASSEXW {
      cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
      lpfnWndProc: proc_,
      hInstance: instance.into(),
      lpszClassName: class,
      ..Default::default()
    };
    RegisterClassExW(&wc);
  }
}

fn show() {
  register(SAVER_CLASS, Some(saver_proc));
  let (list, shuffle) = videos();
  let file = pick(&list, shuffle);
  if file.is_none() {
    log::line("screen saver: no video chosen; showing black");
  }
  let mut windows = Vec::new();
  unsafe {
    let instance = GetModuleHandleW(None).unwrap_or_default();
    for r in monitor_rects() {
      let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        SAVER_CLASS,
        w!("Logical Lunge screen saver"),
        WS_POPUP | WS_VISIBLE,
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top,
        None,
        None,
        instance,
        None,
      ) else {
        continue;
      };
      windows.push((hwnd, r));
    }
    if let Some((first, _)) = windows.first() {
      let _ = SetForegroundWindow(*first);
    }
    ShowCursor(false);
  }
  play(&windows, file);
  unsafe {
    ShowCursor(true);
    for (hwnd, _) in &windows {
      let _ = DestroyWindow(*hwnd);
    }
  }
}

fn preview(parent: isize) {
  register(w!("LogicalLunge.ScreenSaver.Preview"), Some(preview_proc));
  let parent = HWND(parent as _);
  let mut r = RECT::default();
  unsafe {
    if GetClientRect(parent, &mut r).is_err() {
      return;
    }
    let instance = GetModuleHandleW(None).unwrap_or_default();
    let Ok(hwnd) = CreateWindowExW(
      WINDOW_EX_STYLE(0),
      w!("LogicalLunge.ScreenSaver.Preview"),
      w!(""),
      WS_CHILD | WS_VISIBLE,
      0,
      0,
      r.right - r.left,
      r.bottom - r.top,
      parent,
      None,
      instance,
      None,
    ) else {
      return;
    };
    let (list, shuffle) = videos();
    play(&[(hwnd, r)], pick(&list, shuffle));
  }
}

/// Windows' "Settings" for our screen saver: the right panel opens on its
/// screen saver tab (the core relays it).
fn configure() {
  let opened = std::env::current_exe()
    .ok()
    .and_then(|exe| Some(exe.parent()?.join("lunge.exe")))
    .filter(|core| core.exists())
    .and_then(|core| {
      use std::os::windows::process::CommandExt;
      std::process::Command::new(core).args(["--open-page", "screensaver"]).creation_flags(0x0800_0000).status().ok()
    })
    .is_some_and(|status| status.success());
  if !opened {
    // Logical Lunge is not running: there is no panel to open, and its
    // questions never use Windows' message boxes
    crate::log::line("screen saver settings: Logical Lunge is not running");
  }
}

pub fn main(args: &[String]) {
  unsafe {
    let _ = SetProcessDpiAwarenessContext(
      DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    );
  }
  match mode(args) {
    Mode::Show => show(),
    Mode::Preview(parent) => preview(parent),
    Mode::Configure => configure(),
    Mode::Nothing => {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn a(list: &[&str]) -> Vec<String> {
    std::iter::once("x.scr").chain(list.iter().copied()).map(String::from).collect()
  }

  #[test]
  fn reads_the_switches_windows_passes() {
    assert_eq!(mode(&a(&["/s"])), Mode::Show);
    assert_eq!(mode(&a(&["/S"])), Mode::Show);
    assert_eq!(mode(&a(&["/p", "1234"])), Mode::Preview(1234));
    assert_eq!(mode(&a(&["/p:1234"])), Mode::Preview(1234));
    assert_eq!(mode(&a(&["/c:5678"])), Mode::Configure);
    assert_eq!(mode(&a(&[])), Mode::Configure);
    assert_eq!(mode(&a(&["/a"])), Mode::Nothing);
    assert_eq!(mode(&a(&["/p"])), Mode::Nothing);
  }

  #[test]
  fn picks_only_from_the_list() {
    assert_eq!(pick(&[], true), None);
    let one = vec![PathBuf::from("a.mp4")];
    assert_eq!(pick(&one, true), Some(PathBuf::from("a.mp4")));
    assert_eq!(pick(&one, false), Some(PathBuf::from("a.mp4")));
  }
}
