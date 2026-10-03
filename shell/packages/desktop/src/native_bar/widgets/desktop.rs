//! The desktop around the widgets: monitors and their work areas, the
//! window right above the desktop icons, a locked session, a fullscreen app
//! covering a monitor, and the widget windows themselves.

use super::*;

/// A monitor: GDI device name, work area (pixels), DPI, primary.
#[derive(Clone)]
pub(super) struct Mon {
  pub(super) device: String,
  pub(super) work: RECT,
  pub(super) dpi: u32,
  pub(super) primary: bool,
}

pub(super) fn monitors() -> Vec<Mon> {
  unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<Mon>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(m, &mut info as *mut _ as *mut _).as_bool() {
      let (mut dx, mut dy) = (96u32, 96u32);
      let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
      let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
      list.push(Mon {
        device: String::from_utf16_lossy(&info.szDevice[..end]),
        work: info.monitorInfo.rcWork,
        dpi: dx,
        primary: info.monitorInfo.dwFlags & 1 != 0,
      });
    }
    BOOL(1)
  }
  let mut list: Vec<Mon> = Vec::new();
  unsafe {
    let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut _ as isize));
  }
  list
}

/// The monitor a widget is on: its own, else the primary one.
pub(super) fn mon_for(mons: &[Mon], device: &str) -> Option<Mon> {
  mons
    .iter()
    .find(|m| !device.is_empty() && m.device.eq_ignore_ascii_case(device))
    .or_else(|| mons.iter().find(|m| m.primary))
    .or(mons.first())
    .cloned()
}

pub(super) fn area_dip(m: &Mon) -> (f32, f32) {
  let s = crate::native_bar::scale::of_dpi(m.dpi);
  ((m.work.right - m.work.left) as f32 / s, (m.work.bottom - m.work.top) as f32 / s)
}


/// The session is locked (or the secure desktop shows): the input desktop
/// is Winlogon's, which a user's process cannot open.
pub(super) fn locked() -> bool {
  unsafe {
    match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) {
      Ok(d) => {
        let _ = CloseDesktop(d);
        false
      }
      Err(_) => true,
    }
  }
}

pub(super) fn class_of(h: HWND) -> String {
  let mut buf = [0u16; 64];
  let n = unsafe { GetClassNameW(h, &mut buf) }.max(0) as usize;
  String::from_utf16_lossy(&buf[..n])
}

/// The monitor a fullscreen window covers (games, videos): the foreground
/// window spans the whole monitor. The desktop and our own windows don't count.
pub(super) fn covered_monitor() -> Option<RECT> {
  unsafe {
    let fg = GetForegroundWindow();
    if fg.is_invalid() {
      return None;
    }
    let class = class_of(fg);
    if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") || class == "LungeNativeBar" {
      return None;
    }
    let mut r = RECT::default();
    GetWindowRect(fg, &mut r).ok()?;
    let m = windows::Win32::Graphics::Gdi::MonitorFromWindow(fg, windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONULL);
    if m.is_invalid() {
      return None;
    }
    let mut info = windows::Win32::Graphics::Gdi::MONITORINFO { cbSize: std::mem::size_of::<windows::Win32::Graphics::Gdi::MONITORINFO>() as u32, ..Default::default() };
    if !GetMonitorInfoW(m, &mut info).as_bool() {
      return None;
    }
    let mr = info.rcMonitor;
    (r.left <= mr.left && r.top <= mr.top && r.right >= mr.right && r.bottom >= mr.bottom).then_some(mr)
  }
}

/// The top-level window that holds the desktop icons (Progman, or a
/// WorkerW in the classic layout).
pub(super) fn icons_host() -> HWND {
  unsafe extern "system" fn find(top: HWND, lp: LPARAM) -> BOOL {
    if IsWindowVisible(top).as_bool() && FindWindowExW(top, None, w!("SHELLDLL_DefView"), PCWSTR::null()).is_ok_and(|h| !h.is_invalid()) {
      *(lp.0 as *mut HWND) = top;
      return BOOL(0);
    }
    BOOL(1)
  }
  let mut host = HWND::default();
  unsafe {
    let _ = EnumWindows(Some(find), LPARAM(&mut host as *mut HWND as isize));
  }
  host
}

/// Puts a widget right above the desktop icons: under every other window,
/// in front of the icons and the wallpaper. Without a desktop, at the bottom.
pub(super) fn place_above_desktop(hwnd: HWND) {
  unsafe {
    let host = icons_host();
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
    // The core raises a note/place editor as topmost. Once it becomes a
    // passive widget again, HWND_TOP alone would leave it in that band.
    if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0 {
      let _ = SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
    }
    if host.is_invalid() {
      let _ = SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 0, 0, flags);
      return;
    }
    // the window just above the desktop's: ours go right under it
    let above = GetWindow(host, GW_HWNDPREV).unwrap_or_default();
    if above == hwnd {
      return;
    }
    // a topmost window right above the desktop (no ordinary window open):
    // ours go to the top of the ordinary ones, which is right above it too
    // (inserting after a topmost window would make ours topmost)
    let topmost = !above.is_invalid() && GetWindowLongPtrW(above, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
    let after = if above.is_invalid() || topmost { HWND_TOP } else { above };
    let _ = SetWindowPos(hwnd, after, 0, 0, 0, 0, flags);
  }
}

/// All passive widgets must sit above the icons and below visible apps.
/// Hidden/cloaked siblings do not obscure the retained composition surfaces;
/// demanding immediate HWND adjacency used to restack on unrelated events.
pub(super) fn layer_ok(ours: &[HWND], editing: Option<HWND>) -> bool {
  let host = icons_host();
  if host.is_invalid() || ours.is_empty() {
    return true;
  }
  struct Stack<'a> {
    host: HWND,
    ours: &'a [HWND],
    editing: Option<HWND>,
    windows: Vec<policy::Window>,
  }
  unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    let stack = &mut *(lp.0 as *mut Stack<'_>);
    let role = if hwnd == stack.host { policy::Role::Desktop }
      else if Some(hwnd) == stack.editing { policy::Role::Editing }
      else if stack.ours.contains(&hwnd) { policy::Role::Widget }
      else { policy::Role::Other };
    let visible = IsWindowVisible(hwnd).as_bool();
    let topmost = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
    let mut cloaked = 0u32;
    if role == policy::Role::Other && visible && !topmost {
      let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, std::mem::size_of::<u32>() as u32);
    }
    stack.windows.push(policy::Window { role, visible, topmost, cloaked: cloaked != 0 });
    BOOL(1)
  }
  let mut stack = Stack { host, ours, editing, windows: Vec::new() };
  if unsafe { EnumWindows(Some(each), LPARAM(&mut stack as *mut _ as isize)) }.is_err() {
    return true; // An incomplete snapshot cannot justify moving windows.
  }
  policy::layer_is_correct(&stack.windows)
}

pub(super) fn cursor() -> POINT {
  let mut p = POINT::default();
  unsafe {
    let _ = GetCursorPos(&mut p);
  }
  p
}

pub(super) fn key_down(vk: u16) -> bool {
  unsafe { GetKeyState(vk as i32) < 0 }
}

pub(super) fn tools_dir() -> PathBuf {
  std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("tools"))).unwrap_or_default()
}

pub(super) fn make_window(gfx: &gfx::Gfx, x: i32, y: i32, w: u32, h: u32) -> anyhow::Result<(HWND, IDCompositionTarget, IDCompositionVisual2, Layer)> {
  // the shell's windows are left out of tiling and focus rules; no-activate:
  // a press does not take the focus from the window in front
  let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
  unsafe {
    let hwnd = CreateWindowExW(ex, CLASS, TITLE, WS_POPUP, x, y, w as i32, h as i32, None, None, GetModuleHandleW(None)?, None)?;
    let made = (|| -> windows::core::Result<_> {
      let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let root = gfx.dcomp.CreateVisual()?;
      let layer = Layer::new(gfx, w.max(1), h.max(1))?;
      root.AddVisual(&layer.visual, false, None)?;
      target.SetRoot(&root)?;
      Ok((target, root, layer))
    })();
    match made {
      Ok((t, r, l)) => Ok((hwnd, t, r, l)),
      Err(err) => {
        let _ = DestroyWindow(hwnd);
        Err(err.into())
      }
    }
  }
}
