#[path = "companion_policy.rs"]
mod companion_policy;
use companion_policy::is_descendant;

use std::time::Duration;

use tokio::task;
use windows::{
  core::{w, PCWSTR, PWSTR},
  Win32::{
    Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, POINT, RECT},
    Graphics::Dwm::{
      DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_BORDER_COLOR,
      DWMWA_CLOAKED, DWMWA_COLOR_NONE, DWMWA_EXTENDED_FRAME_BOUNDS,
      DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT, DWMWCP_DONOTROUND,
      DWMWCP_ROUND, DWMWCP_ROUNDSMALL,
    },
    System::{
      Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW,
        PROCESSENTRY32W, TH32CS_SNAPPROCESS,
      },
      Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
      },
    },
    UI::{
      Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
      },
      WindowsAndMessaging::{
        EnumWindows, FindWindowExW, GetAncestor, GetClassNameW, GetDesktopWindow,
        GetForegroundWindow, GetLayeredWindowAttributes, GetShellWindow,
        GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        IsZoomed, SendNotifyMessageW, SetForegroundWindow,
        GetPropW, GetWindowPlacement, RemovePropW, SetLayeredWindowAttributes,
        SetPropW,
        SetWindowLongPtrW, SetWindowPlacement,
        SetWindowPos, ShowWindowAsync, WindowFromPoint, GA_ROOT,
        GWL_EXSTYLE, GWL_STYLE, GW_OWNER, HWND_NOTOPMOST, HWND_TOP,
        HWND_TOPMOST, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA,
        LWA_COLORKEY, SET_WINDOW_POS_FLAGS, SWP_ASYNCWINDOWPOS,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE,
        SWP_NOOWNERZORDER, SWP_NOSENDCHANGING, SWP_NOSIZE, SWP_NOZORDER,
        SWP_SHOWWINDOW, SW_HIDE, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE,
        SW_SHOWNA, SW_SHOWNOACTIVATE, WINDOWPLACEMENT, WINDOW_EX_STYLE, WINDOW_STYLE,
        WM_CLOSE, WPF_ASYNCWINDOWPLACEMENT, WS_DLGFRAME, WS_EX_LAYERED,
        WS_THICKFRAME,
      },
    },
  },
};

use super::com::{IApplicationView, COM_INIT};
use crate::{
  Color, CornerStyle, Delta, Dispatcher, LengthValue, OpacityValue, Point,
  Rect, RectDelta, WindowId, WindowZOrder,
};

/// Magic number used to identify programmatic mouse inputs from our own
/// process.
pub(crate) const FOREGROUND_INPUT_IDENTIFIER: u32 = 6379;

/// Platform-specific implementation of [`NativeWindow`].
#[derive(Clone, Debug)]
pub(crate) struct NativeWindow {
  pub(crate) handle: isize,
}

impl NativeWindow {
  /// Creates an instance of `NativeWindow`.
  #[must_use]
  pub(crate) fn new(handle: isize) -> Self {
    Self { handle }
  }

  /// Implements [`NativeWindow::id`].
  #[must_use]
  pub(crate) fn id(&self) -> WindowId {
    WindowId(self.handle)
  }

  /// Implements [`NativeWindow::title`].
  #[allow(clippy::unnecessary_wraps)]
  pub(crate) fn title(&self) -> crate::Result<String> {
    let mut text: [u16; 512] = [0; 512];
    let length = unsafe { GetWindowTextW(self.hwnd(), &mut text) };

    #[allow(clippy::cast_sign_loss)]
    Ok(String::from_utf16_lossy(&text[..length as usize]))
  }

  /// Implements [`NativeWindow::process_name`].
  ///
  /// A Store app's frame (`ApplicationFrameWindow`) belongs to
  /// ApplicationFrameHost, every Store app's alike; while the app runs, its
  /// own window (`Windows.UI.Core.CoreWindow`) is the frame's child and
  /// names the app. A suspended app's is detached: the host's name then.
  pub(crate) fn process_name(&self) -> crate::Result<String> {
    let mut window = self.hwnd();
    if self.class_name().is_ok_and(|c| c == "ApplicationFrameWindow") {
      let app = unsafe {
        FindWindowExW(
          self.hwnd(),
          HWND(0),
          w!("Windows.UI.Core.CoreWindow"),
          PCWSTR::null(),
        )
      };
      if app.0 != 0 {
        window = app;
      }
    }

    let mut process_id = 0u32;
    unsafe {
      GetWindowThreadProcessId(window, Some(&raw mut process_id));
    }

    let process_handle = unsafe {
      OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
    }?;

    let mut buffer = [0u16; 256];
    let mut length = u32::try_from(buffer.len())?;

    unsafe {
      let query_res = QueryFullProcessImageNameW(
        process_handle,
        PROCESS_NAME_WIN32,
        PWSTR(buffer.as_mut_ptr()),
        &raw mut length,
      );

      // Always close the process handle regardless of the query result.
      CloseHandle(process_handle)?;

      query_res
    }?;

    let exe_path = String::from_utf16_lossy(&buffer[..length as usize]);

    exe_path
      .split('\\')
      .next_back()
      .map(|file_name| {
        file_name.split('.').next().unwrap_or(file_name).to_string()
      })
      .ok_or_else(|| {
        crate::Error::Platform("Failed to parse process name.".to_string())
      })
  }

  /// Implements [`NativeWindow::frame`].
  pub(crate) fn frame(&self) -> crate::Result<Rect> {
    let mut rect = RECT::default();

    let dwm_res = unsafe {
      #[allow(clippy::cast_possible_truncation)]
      DwmGetWindowAttribute(
        self.hwnd(),
        DWMWA_EXTENDED_FRAME_BOUNDS,
        std::ptr::from_mut(&mut rect).cast(),
        std::mem::size_of::<RECT>() as u32,
      )
    };

    if let Ok(()) = dwm_res {
      Ok(Rect::from_ltrb(
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
      ))
    } else {
      // Common (windows being created or destroyed) and handled: no warning.
      tracing::debug!("Failed to get window's frame position. Falling back to border position.");
      self.frame_with_shadows()
    }
  }

  /// Implements [`NativeWindow::position`].
  pub(crate) fn position(&self) -> crate::Result<(f64, f64)> {
    let frame = self.frame()?;
    Ok((f64::from(frame.left), f64::from(frame.top)))
  }

  /// Implements [`NativeWindow::size`].
  pub(crate) fn size(&self) -> crate::Result<(f64, f64)> {
    let frame = self.frame()?;
    Ok((f64::from(frame.width()), f64::from(frame.height())))
  }

  /// Implements [`NativeWindow::is_valid`].
  pub(crate) fn is_valid(&self) -> bool {
    unsafe { IsWindow(self.hwnd()) }.as_bool()
  }

  /// Implements [`NativeWindow::is_visible`].
  pub(crate) fn is_visible(&self) -> crate::Result<bool> {
    let is_visible = unsafe { IsWindowVisible(self.hwnd()) }.as_bool();

    Ok(is_visible && !self.is_cloaked()?)
  }

  /// Implements [`NativeWindow::is_minimized`].
  #[allow(clippy::unnecessary_wraps)]
  pub(crate) fn is_minimized(&self) -> crate::Result<bool> {
    Ok(unsafe { IsIconic(self.hwnd()) }.as_bool())
  }

  /// Implements [`NativeWindow::is_maximized`].
  #[allow(clippy::unnecessary_wraps)]
  pub(crate) fn is_maximized(&self) -> crate::Result<bool> {
    Ok(unsafe { IsZoomed(self.hwnd()) }.as_bool())
  }

  /// Implements [`NativeWindow::is_resizable`].
  #[allow(clippy::unnecessary_wraps)]
  pub(crate) fn is_resizable(&self) -> crate::Result<bool> {
    Ok(self.has_window_style(WS_THICKFRAME))
  }

  /// Implements [`NativeWindow::is_desktop_window`].
  #[allow(clippy::unnecessary_wraps)]
  pub(crate) fn is_desktop_window(&self) -> crate::Result<bool> {
    Ok(*self == desktop_window())
  }

  /// Implements [`NativeWindow::set_frame`].
  pub(crate) fn set_frame(&self, rect: &Rect) -> crate::Result<()> {
    unsafe {
      SetWindowPos(
        self.hwnd(),
        HWND_NOTOPMOST,
        rect.x(),
        rect.y(),
        rect.width(),
        rect.height(),
        SWP_NOACTIVATE
          | SWP_NOZORDER
          | SWP_NOCOPYBITS
          | SWP_NOSENDCHANGING
          | SWP_ASYNCWINDOWPOS
          | SWP_FRAMECHANGED,
      )
    }?;

    Ok(())
  }

  /// Implements [`NativeWindow::resize`].
  pub(crate) fn resize(
    &self,
    width: i32,
    height: i32,
  ) -> crate::Result<()> {
    unsafe {
      SetWindowPos(
        self.hwnd(),
        HWND_NOTOPMOST,
        0,
        0,
        width,
        height,
        SWP_NOACTIVATE
          | SWP_NOZORDER
          | SWP_NOMOVE
          | SWP_NOCOPYBITS
          | SWP_NOSENDCHANGING
          | SWP_ASYNCWINDOWPOS
          | SWP_FRAMECHANGED,
      )
    }?;

    Ok(())
  }

  /// Implements [`NativeWindow::reposition`].
  pub(crate) fn reposition(&self, x: i32, y: i32) -> crate::Result<()> {
    unsafe {
      SetWindowPos(
        self.hwnd(),
        HWND_NOTOPMOST,
        x,
        y,
        0,
        0,
        SWP_NOACTIVATE
          | SWP_NOZORDER
          | SWP_NOSIZE
          | SWP_NOCOPYBITS
          | SWP_NOSENDCHANGING
          | SWP_ASYNCWINDOWPOS
          | SWP_FRAMECHANGED,
      )
    }?;

    Ok(())
  }

  /// Implements [`NativeWindow::minimize`].
  pub(crate) fn minimize(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(self.hwnd(), SW_MINIMIZE).ok() }?;
    Ok(())
  }

  /// Implements [`NativeWindow::maximize`].
  pub(crate) fn maximize(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(self.hwnd(), SW_MAXIMIZE).ok() }?;
    Ok(())
  }

  /// Implements [`NativeWindow::focus`].
  pub(crate) fn focus(&self) -> crate::Result<()> {
    let input = [INPUT {
      r#type: INPUT_MOUSE,
      Anonymous: INPUT_0 {
        mi: MOUSEINPUT {
          dwExtraInfo: FOREGROUND_INPUT_IDENTIFIER as usize,
          ..Default::default()
        },
      },
    }];

    // Bypass restriction for setting the foreground window by sending an
    // input to our own process first.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    unsafe {
      SendInput(&input, std::mem::size_of::<INPUT>() as i32)
    };

    // Set as the foreground window.
    unsafe { SetForegroundWindow(self.hwnd()) }.ok()?;

    Ok(())
  }

  /// Implements [`NativeWindow::close`].
  pub(crate) fn close(&self) -> crate::Result<()> {
    unsafe { SendNotifyMessageW(self.hwnd(), WM_CLOSE, None, None) }?;
    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::hwnd`].
  pub(crate) fn hwnd(&self) -> HWND {
    HWND(self.handle)
  }

  /// Implements [`NativeWindowWindowsExt::class_name`].
  pub(crate) fn class_name(&self) -> crate::Result<String> {
    let mut buffer = [0u16; 256];
    let result = unsafe { GetClassNameW(self.hwnd(), &mut buffer) };

    if result == 0 {
      return Err(windows::core::Error::from_win32().into());
    }

    #[allow(clippy::cast_sign_loss)]
    let class_name = String::from_utf16_lossy(&buffer[..result as usize]);
    Ok(class_name)
  }

  /// Implements [`NativeWindowWindowsExt::frame_with_shadows`].
  pub(crate) fn frame_with_shadows(&self) -> crate::Result<Rect> {
    let mut rect = RECT::default();

    unsafe {
      GetWindowRect(self.hwnd(), std::ptr::from_mut(&mut rect).cast())
    }?;

    Ok(Rect::from_ltrb(
      rect.left,
      rect.top,
      rect.right,
      rect.bottom,
    ))
  }

  /// Implements [`NativeWindowWindowsExt::shadow_borders`].
  // TODO: Return tuple of (left, top, right, bottom) instead of
  // `RectDelta`.
  pub(crate) fn shadow_borders(&self) -> crate::Result<RectDelta> {
    let border_pos = self.frame_with_shadows()?;
    let frame_pos = self.frame()?;

    Ok(RectDelta::new(
      LengthValue::from_px(frame_pos.left - border_pos.left),
      LengthValue::from_px(frame_pos.top - border_pos.top),
      LengthValue::from_px(border_pos.right - frame_pos.right),
      LengthValue::from_px(border_pos.bottom - frame_pos.bottom),
    ))
  }

  /// Implements [`NativeWindowWindowsExt::has_owner_window`].
  pub(crate) fn has_owner_window(&self) -> bool {
    unsafe { GetWindow(self.hwnd(), GW_OWNER) }.0 != 0
  }

  /// Implements [`NativeWindowWindowsExt::has_window_style`].
  pub(crate) fn has_window_style(&self, style: WINDOW_STYLE) -> bool {
    let current_style =
      unsafe { GetWindowLongPtrW(self.hwnd(), GWL_STYLE) };

    #[allow(clippy::cast_possible_wrap)]
    let style = style.0 as isize;
    (current_style & style) != 0
  }

  /// Implements [`NativeWindowWindowsExt::has_window_style_ex`].
  pub(crate) fn has_window_style_ex(
    &self,
    style: WINDOW_EX_STYLE,
  ) -> bool {
    let current_style =
      unsafe { GetWindowLongPtrW(self.hwnd(), GWL_EXSTYLE) };

    #[allow(clippy::cast_possible_wrap)]
    let style = style.0 as isize;
    (current_style & style) != 0
  }

  /// Implements [`NativeWindowWindowsExt::set_window_pos`].
  pub(crate) fn set_window_pos(
    &self,
    z_order: &WindowZOrder,
    rect: &Rect,
    flags: SET_WINDOW_POS_FLAGS,
  ) -> crate::Result<()> {
    let z_order_hwnd = match z_order {
      WindowZOrder::TopMost => HWND_TOPMOST,
      WindowZOrder::Top => HWND_TOP,
      WindowZOrder::Normal => HWND_NOTOPMOST,
      WindowZOrder::AfterWindow(window_id) => HWND(window_id.0),
    };

    unsafe {
      SetWindowPos(
        self.hwnd(),
        z_order_hwnd,
        rect.x(),
        rect.y(),
        rect.width(),
        rect.height(),
        flags,
      )
    }?;

    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::show`].
  pub(crate) fn show(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(self.hwnd(), SW_SHOWNA) }.ok()?;
    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::hide`].
  pub(crate) fn hide(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(self.hwnd(), SW_HIDE) }.ok()?;
    Ok(())
  }

  pub(crate) fn hide_companion(&self) -> crate::Result<()> {
    const TAG: PCWSTR = w!("LogicalLunge.HiddenCompanion");
    // Mark before hiding: a crash between calls must still be recoverable.
    unsafe { SetPropW(self.hwnd(), TAG, HANDLE(1)) }?;
    if let Err(err) = self.hide() {
      let _ = unsafe { RemovePropW(self.hwnd(), TAG) };
      return Err(err);
    }
    Ok(())
  }

  pub(crate) fn show_companion(&self) -> crate::Result<()> {
    // A destroyed HWND may already have been reused by an unrelated window.
    if unsafe { GetPropW(self.hwnd(), w!("LogicalLunge.HiddenCompanion")) }.0 == 0 {
      return Ok(());
    }
    self.show()?;
    let _ = unsafe { RemovePropW(self.hwnd(), w!("LogicalLunge.HiddenCompanion")) };
    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::restore`].
  pub(crate) fn restore(
    &self,
    outer_frame: Option<&Rect>,
  ) -> crate::Result<()> {
    match outer_frame {
      None => {
        unsafe { ShowWindowAsync(self.hwnd(), SW_RESTORE) }.ok()?;
        Ok(())
      }
      Some(rect) => {
        // Logical Lunge: without activating. Restoring a maximized or
        // minimized window into its tile must not take the focus (restoring
        // the minimized windows found on startup focused each in turn).
        let placement = WINDOWPLACEMENT {
          #[allow(clippy::cast_possible_truncation)]
          length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
          flags: WPF_ASYNCWINDOWPLACEMENT,
          showCmd: SW_SHOWNOACTIVATE.0 as u32,
          rcNormalPosition: RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
          },
          ..Default::default()
        };

        unsafe { SetWindowPlacement(self.hwnd(), &raw const placement) }?;
        Ok(())
      }
    }
  }

  /// Implements [`NativeWindowWindowsExt::companions`].
  pub(crate) fn companions(&self) -> Vec<crate::NativeWindow> {
    let mut own_pid = 0;
    unsafe { GetWindowThreadProcessId(self.hwnd(), Some(&raw mut own_pid)) };
    let mut frame = RECT::default();
    if own_pid == 0 || unsafe { GetWindowRect(self.hwnd(), &raw mut frame) }.is_err() {
      return Vec::new();
    }
    let parents = process_parents();
    // the visible frame too: an overlay is laid over that, without the invisible resize borders
    let visible = self.frame().ok().map(|v| RECT { left: v.left, top: v.top, right: v.right, bottom: v.bottom });
    let this_process = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };

    let mut handles: Vec<isize> = Vec::new();
    #[allow(clippy::items_after_statements)]
    extern "system" fn collect(handle: HWND, data: LPARAM) -> BOOL {
      let handles = data.0 as *mut Vec<isize>;
      unsafe { (*handles).push(handle.0) };
      true.into()
    }
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(std::ptr::from_mut(&mut handles) as _)) };

    handles
      .into_iter()
      .filter(|&handle| handle != self.handle)
      .map(NativeWindow::new)
      .filter(|w| {
        let hwnd = w.hwnd();
        if !unsafe { IsWindowVisible(hwnd) }.as_bool()
          || unsafe { GetWindow(hwnd, GW_OWNER) }.0 != 0
        {
          return false;
        }
        let mut cloaked = 0u32;
        let read = unsafe {
          DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, std::ptr::from_mut(&mut cloaked).cast(), 4)
        };
        if read.is_err() || cloaked != 0 {
          return false;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        let mut rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &raw mut rect) }.is_err() {
          return false;
        }
        if is_descendant(pid, own_pid, &parents) {
          return rect.left < frame.right
            && frame.left < rect.right
            && rect.top < frame.bottom
            && frame.top < rect.bottom;
        }
        // Another program's overlay over this window (a game overlay such
        // as Discord's): not managed, so the workspace switch left it up
        // over the next workspace until its program noticed. Not this
        // process's own windows (the borders) nor Logical Lunge's.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        let own_frames = [Some(frame), visible];
        pid != this_process
          && own_frames.iter().flatten().any(|f| overlay_over(f, &rect, ex))
          && !w.process_name().is_ok_and(|n| n.to_ascii_lowercase().starts_with("lunge"))
      })
      .map(Into::into)
      .collect()
  }

  /// Implements [`NativeWindowWindowsExt::set_cloaked`].
  pub(crate) fn set_cloaked(&self, cloaked: bool) -> crate::Result<()> {
    COM_INIT.with(|com_init| -> crate::Result<()> {
      com_init.borrow_mut().with_retry(|com| {
        let view_collection = com.application_view_collection()?;

        let mut view: Option<IApplicationView> = None;
        unsafe {
          view_collection.get_view_for_hwnd(self.hwnd().0, &raw mut view)
        }
        .ok()?;

        let view = view.ok_or_else(|| {
          crate::Error::Platform(
            "Unable to get application view by window handle.".to_string(),
          )
        })?;

        // Ref: https://github.com/Ciantic/AltTabAccessor/issues/1#issuecomment-1426877843
        unsafe { view.set_cloak(1, if cloaked { 2 } else { 0 }) }
          .ok()
          .map_err(|_| {
            crate::Error::Platform("Failed to cloak window.".to_string())
          })
      })
    })
  }

  /// Implements [`NativeWindowWindowsExt::mark_fullscreen`].
  pub(crate) fn mark_fullscreen(
    &self,
    fullscreen: bool,
  ) -> crate::Result<()> {
    COM_INIT.with(|com_init| -> crate::Result<()> {
      com_init.borrow_mut().with_retry(|com| {
        let taskbar_list = com.taskbar_list()?;

        unsafe {
          taskbar_list.MarkFullscreenWindow(self.hwnd(), fullscreen)
        }?;

        Ok(())
      })
    })
  }

  /// Implements [`NativeWindowWindowsExt::set_taskbar_visibility`].
  pub(crate) fn set_taskbar_visibility(
    &self,
    visible: bool,
  ) -> crate::Result<()> {
    COM_INIT.with(|com_init| -> crate::Result<()> {
      com_init.borrow_mut().with_retry(|com| {
        let taskbar_list = com.taskbar_list()?;

        if visible {
          unsafe { taskbar_list.AddTab(self.hwnd())? };
        } else {
          unsafe { taskbar_list.DeleteTab(self.hwnd())? };
        }

        Ok(())
      })
    })
  }

  /// Implements [`NativeWindowWindowsExt::add_window_style_ex`].
  pub(crate) fn add_window_style_ex(&self, style: WINDOW_EX_STYLE) {
    let current_style =
      unsafe { GetWindowLongPtrW(self.hwnd(), GWL_EXSTYLE) };

    #[allow(clippy::cast_possible_wrap)]
    if current_style & style.0 as isize == 0 {
      let new_style = current_style | style.0 as isize;

      unsafe { SetWindowLongPtrW(self.hwnd(), GWL_EXSTYLE, new_style) };
    }
  }

  /// Implements [`NativeWindowWindowsExt::set_z_order`].
  /// Implements [`NativeWindowWindowsExt::show_no_activate`].
  pub(crate) fn show_no_activate(&self) -> crate::Result<()> {
    unsafe { ShowWindowAsync(self.hwnd(), SW_SHOWNOACTIVATE) }.ok()?;
    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::is_controllable`]. Cached per
  /// process (a process's level does not change); checked on every redraw.
  pub(crate) fn is_controllable(&self) -> bool {
    use std::{collections::HashMap, sync::Mutex};

    static CACHE: Mutex<Option<HashMap<u32, bool>>> = Mutex::new(None);

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(self.hwnd(), Some(&raw mut pid)) };
    if pid == 0 {
      return true;
    }
    if let Ok(cache) = CACHE.lock() {
      if let Some(known) = cache.as_ref().and_then(|c| c.get(&pid)) {
        return *known;
      }
    }
    let controllable = Self::process_controllable(pid);
    if let Ok(mut cache) = CACHE.lock() {
      let map = cache.get_or_insert_with(HashMap::new);
      // pids are reused: keep it small
      if map.len() > 256 {
        map.clear();
      }
      map.insert(pid, controllable);
    }
    controllable
  }

  fn process_controllable(pid: u32) -> bool {
    use windows::Win32::{
      Security::TOKEN_QUERY,
      System::Threading::OpenProcessToken,
    };

    let ours = own_integrity();
    // unknown: assume yes (as before this check existed)
    let Ok(process) = (unsafe {
      OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
    }) else {
      return true;
    };
    let mut token = HANDLE::default();
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    let _ = unsafe { CloseHandle(process) };
    if opened.is_err() {
      // a token we may not read belongs to a higher level
      return ours >= INTEGRITY_HIGH;
    }
    let level = token_integrity(token);
    let _ = unsafe { CloseHandle(token) };
    level.map_or(true, |level| level <= ours)
  }

  /// Implements [`NativeWindowWindowsExt::restored_frame`].
  pub(crate) fn restored_frame(&self) -> crate::Result<Rect> {
    let mut placement = WINDOWPLACEMENT {
      #[allow(clippy::cast_possible_truncation)]
      length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
      ..Default::default()
    };
    unsafe { GetWindowPlacement(self.hwnd(), &raw mut placement) }?;
    let r = placement.rcNormalPosition;
    Ok(Rect::from_ltrb(r.left, r.top, r.right, r.bottom))
  }

  /// Implements [`NativeWindowWindowsExt::set_slot`].
  pub(crate) fn set_slot(&self, slot: Option<&Rect>) -> crate::Result<()> {
    const LT: PCWSTR = w!("LungeSlotLT");
    const RB: PCWSTR = w!("LungeSlotRB");
    unsafe {
      match slot {
        Some(rect) => {
          let (lt, rb) = (
            pack_slot(rect.left, rect.top),
            pack_slot(rect.right, rect.bottom),
          );
          // unchanged: no work (every redraw of a tile passes here)
          if GetPropW(self.hwnd(), LT).0 != lt
            || GetPropW(self.hwnd(), RB).0 != rb
          {
            SetPropW(self.hwnd(), LT, HANDLE(lt))?;
            SetPropW(self.hwnd(), RB, HANDLE(rb))?;
          }
        }
        None => {
          if GetPropW(self.hwnd(), LT).0 != 0 {
            let _ = RemovePropW(self.hwnd(), LT);
            let _ = RemovePropW(self.hwnd(), RB);
          }
        }
      }
    }
    Ok(())
  }

  pub(crate) fn set_z_order(
    &self,
    z_order: &WindowZOrder,
  ) -> crate::Result<()> {
    let z_order_hwnd = match z_order {
      WindowZOrder::TopMost => HWND_TOPMOST,
      WindowZOrder::Top => HWND_TOP,
      WindowZOrder::Normal => HWND_NOTOPMOST,
      WindowZOrder::AfterWindow(window_id) => HWND(window_id.0),
    };

    // Logical Lunge: SWP_NOSENDCHANGING, as for every other move the WM
    // makes. A z-order change is no reason for the app to pick a new size,
    // but WM_WINDOWPOSCHANGING let it: a self-fullscreen app spread back
    // over the monitor and a terminal snapped to its character grid each
    // time focus moved, and the WM pushed them back into their tiles
    // (flicker while the mouse crossed between windows).
    let flags = SWP_NOACTIVATE
      | SWP_NOCOPYBITS
      | SWP_ASYNCWINDOWPOS
      | SWP_SHOWWINDOW
      | SWP_NOMOVE
      | SWP_NOSIZE
      | SWP_NOSENDCHANGING;

    unsafe { SetWindowPos(self.hwnd(), z_order_hwnd, 0, 0, 0, 0, flags) }?;

    // Z-order can sometimes still be incorrect after the above call.
    let handle = self.handle;
    task::spawn(async move {
      tokio::time::sleep(Duration::from_millis(10)).await;
      let _ = unsafe {
        SetWindowPos(HWND(handle), z_order_hwnd, 0, 0, 0, 0, flags)
      };
    });

    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::set_title_bar_visibility`].
  pub(crate) fn set_title_bar_visibility(
    &self,
    visible: bool,
  ) -> crate::Result<()> {
    let style = unsafe { GetWindowLongPtrW(self.hwnd(), GWL_STYLE) };

    #[allow(clippy::cast_possible_wrap)]
    let new_style = if visible {
      style | (WS_DLGFRAME.0 as isize)
    } else {
      style & !(WS_DLGFRAME.0 as isize)
    };

    if new_style != style {
      unsafe {
        SetWindowLongPtrW(self.hwnd(), GWL_STYLE, new_style);
        SetWindowPos(
          self.hwnd(),
          HWND_NOTOPMOST,
          0,
          0,
          0,
          0,
          SWP_FRAMECHANGED
            | SWP_NOMOVE
            | SWP_NOSIZE
            | SWP_NOZORDER
            | SWP_NOOWNERZORDER
            | SWP_NOACTIVATE
            | SWP_NOCOPYBITS
            | SWP_NOSENDCHANGING
            | SWP_ASYNCWINDOWPOS,
        )?;
      }
    }

    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::set_border_color`].
  pub(crate) fn set_border_color(
    &self,
    color: Option<&Color>,
  ) -> crate::Result<()> {
    let bgr = match color {
      Some(color) => color.to_bgr(),
      None => DWMWA_COLOR_NONE,
    };

    unsafe {
      #[allow(clippy::cast_possible_truncation)]
      DwmSetWindowAttribute(
        self.hwnd(),
        DWMWA_BORDER_COLOR,
        std::ptr::from_ref(&bgr).cast(),
        std::mem::size_of::<u32>() as u32,
      )?;
    }

    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::set_corner_style`].
  pub(crate) fn set_corner_style(
    &self,
    corner_style: &CornerStyle,
  ) -> crate::Result<()> {
    let corner_preference = match corner_style {
      CornerStyle::Default => DWMWCP_DEFAULT,
      CornerStyle::Square => DWMWCP_DONOTROUND,
      CornerStyle::Rounded => DWMWCP_ROUND,
      CornerStyle::SmallRounded => DWMWCP_ROUNDSMALL,
    };

    unsafe {
      #[allow(clippy::cast_possible_truncation)]
      DwmSetWindowAttribute(
        self.hwnd(),
        DWMWA_WINDOW_CORNER_PREFERENCE,
        std::ptr::from_ref(&(corner_preference.0)).cast(),
        std::mem::size_of::<i32>() as u32,
      )?;
    }

    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::set_transparency`].
  pub(crate) fn set_transparency(
    &self,
    opacity_value: &OpacityValue,
  ) -> crate::Result<()> {
    // Make the window layered if it isn't already.
    self.add_window_style_ex(WS_EX_LAYERED);

    unsafe {
      SetLayeredWindowAttributes(
        self.hwnd(),
        None,
        opacity_value.to_alpha(),
        LWA_ALPHA,
      )?;
    }

    Ok(())
  }

  /// Implements [`NativeWindowWindowsExt::adjust_transparency`].
  pub(crate) fn adjust_transparency(
    &self,
    opacity_delta: &Delta<OpacityValue>,
  ) -> crate::Result<()> {
    let mut alpha = u8::MAX;
    let mut flag = LAYERED_WINDOW_ATTRIBUTES_FLAGS::default();

    unsafe {
      GetLayeredWindowAttributes(
        self.hwnd(),
        None,
        Some(&raw mut alpha),
        Some(&raw mut flag),
      )?;
    }

    if flag.contains(LWA_COLORKEY) {
      return Err(crate::Error::Platform(
        "Window uses color key for its transparency and cannot be adjusted."
          .to_string(),
      ));
    }

    let target_alpha = if opacity_delta.is_negative {
      alpha.saturating_sub(opacity_delta.inner.to_alpha())
    } else {
      alpha.saturating_add(opacity_delta.inner.to_alpha())
    };

    self.set_transparency(&OpacityValue::from_alpha(target_alpha))
  }

  /// Whether the window is cloaked. For some UWP apps, `WS_VISIBLE` will
  /// be present even if the window isn't actually visible. The
  /// `DWMWA_CLOAKED` attribute is used to check whether these apps are
  /// visible.
  fn is_cloaked(&self) -> crate::Result<bool> {
    let mut cloaked = 0u32;

    unsafe {
      #[allow(clippy::cast_possible_truncation)]
      DwmGetWindowAttribute(
        self.hwnd(),
        DWMWA_CLOAKED,
        std::ptr::from_mut::<u32>(&mut cloaked).cast(),
        std::mem::size_of::<u32>() as u32,
      )
    }?;

    Ok(cloaked != 0)
  }
}

impl PartialEq for NativeWindow {
  fn eq(&self, other: &Self) -> bool {
    self.handle == other.handle
  }
}

impl Eq for NativeWindow {}

impl From<NativeWindow> for crate::NativeWindow {
  fn from(window: NativeWindow) -> Self {
    crate::NativeWindow { inner: window }
  }
}

/// Every process's parent (one snapshot; empty if it cannot be taken).
fn process_parents() -> std::collections::HashMap<u32, u32> {
  let mut parents = std::collections::HashMap::new();
  let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
    return parents;
  };
  #[allow(clippy::cast_possible_truncation)]
  let mut entry = PROCESSENTRY32W {
    dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
    ..Default::default()
  };
  let mut more = unsafe { Process32FirstW(snapshot, &raw mut entry) }.is_ok();
  while more {
    parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
    more = unsafe { Process32NextW(snapshot, &raw mut entry) }.is_ok();
  }
  let _ = unsafe { CloseHandle(snapshot) };
  parents
}

/// Whether `pid` was started (directly or further down) by `ancestor`. The
/// walk is bounded: a parent id can be reused by an unrelated process.

/// Implements [`Dispatcher::visible_windows`].
pub(crate) fn visible_windows(
  _: &Dispatcher,
) -> crate::Result<Vec<crate::NativeWindow>> {
  let mut handles: Vec<isize> = Vec::new();

  #[allow(clippy::items_after_statements)]
  extern "system" fn visible_windows_proc(
    handle: HWND,
    data: LPARAM,
  ) -> BOOL {
    let handles = data.0 as *mut Vec<isize>;
    unsafe { (*handles).push(handle.0) };
    true.into()
  }

  unsafe {
    EnumWindows(
      Some(visible_windows_proc),
      LPARAM(std::ptr::from_mut(&mut handles) as _),
    )
  }?;

  Ok(
    handles
      .into_iter()
      .map(NativeWindow::new)
      .filter(|window| window.is_visible().unwrap_or(false))
      .map(Into::into)
      .collect(),
  )
}

/// Implements [`Dispatcher::focused_window`].
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn focused_window(
  _: &Dispatcher,
) -> crate::Result<crate::NativeWindow> {
  let handle = unsafe { GetForegroundWindow() };
  Ok(NativeWindow::new(handle.0).into())
}

/// Implements [`Dispatcher::window_from_point`].
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn window_from_point(
  point: &Point,
  _: &Dispatcher,
) -> crate::Result<Option<crate::NativeWindow>> {
  let point = POINT {
    x: point.x,
    y: point.y,
  };

  let handle = unsafe { WindowFromPoint(point) };
  if handle.0 == 0 {
    return Ok(None);
  }

  let root = unsafe { GetAncestor(handle, GA_ROOT) };
  if root.0 == 0 {
    return Ok(None);
  }

  Ok(Some(NativeWindow::new(root.0).into()))
}

/// Implements [`Dispatcher::reset_focus`].
pub(crate) fn reset_focus(_dispatcher: &Dispatcher) -> crate::Result<()> {
  desktop_window().focus()
}

/// Gets the `NativeWindow` instance of the desktop window.
///
/// This is the explorer.exe wallpaper window (i.e. "Progman"). If
/// explorer.exe isn't running, then default to the desktop window below
/// the wallpaper window.
#[must_use]
fn desktop_window() -> NativeWindow {
  // Logical Lunge: an empty workspace gives the keyboard to the core's
  // invisible focus window (it swallows keys). The desktop took keys to its
  // icons, and when focusing it failed, keys went on to a hidden window of
  // another workspace. Without the core, the desktop as before.
  let sink = unsafe {
    windows::Win32::UI::WindowsAndMessaging::FindWindowW(
      w!("LogicalLunge.FocusSink"),
      PCWSTR::null(),
    )
  };
  if sink.0 != 0 {
    return NativeWindow::new(sink.0);
  }

  let handle = match unsafe { GetShellWindow() } {
    HWND(0) => unsafe { GetDesktopWindow() },
    handle => handle,
  };

  NativeWindow::new(handle.0)
}

/// Two coordinates in one property value; each half is offset so that no
/// real slot packs to 0 (which reads as "no property"). Read by the core
/// (`Rounder`) and by the borders.
fn pack_slot(a: i32, b: i32) -> isize {
  const BIAS: u32 = 0x4000_0000;
  #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
  let packed = (u64::from((a as u32).wrapping_add(BIAS)) << 32)
    | u64::from((b as u32).wrapping_add(BIAS));
  #[allow(clippy::cast_possible_wrap)]
  {
    packed as isize
  }
}

const INTEGRITY_MEDIUM: u32 = 0x2000;
const INTEGRITY_HIGH: u32 = 0x3000;

/// Integrity level of a token (Low 0x1000, Medium 0x2000, High 0x3000,
/// System 0x4000).
fn token_integrity(token: HANDLE) -> Option<u32> {
  use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    TokenIntegrityLevel, TOKEN_MANDATORY_LABEL,
  };

  unsafe {
    let mut len = 0u32;
    let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &raw mut len);
    if len == 0 {
      return None;
    }
    let mut buf = vec![0u8; len as usize];
    GetTokenInformation(
      token,
      TokenIntegrityLevel,
      Some(buf.as_mut_ptr().cast()),
      len,
      &raw mut len,
    )
    .ok()?;
    let label = &*(buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
    let sid = label.Label.Sid;
    let count = *GetSidSubAuthorityCount(sid);
    if count == 0 {
      return None;
    }
    Some(*GetSidSubAuthority(sid, u32::from(count - 1)))
  }
}

/// This process's integrity level (read once).
fn own_integrity() -> u32 {
  use std::sync::OnceLock;
  use windows::Win32::{
    Security::TOKEN_QUERY,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
  };

  static LEVEL: OnceLock<u32> = OnceLock::new();
  *LEVEL.get_or_init(|| unsafe {
    let mut token = HANDLE::default();
    if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token).is_err() {
      return INTEGRITY_MEDIUM;
    }
    let level = token_integrity(token).unwrap_or(INTEGRITY_MEDIUM);
    let _ = CloseHandle(token);
    level
  })
}

/// A window laid exactly over `frame` (within 2 px on every side) that is a
/// tool window or a click-through layered one: an overlay of that window.
fn overlay_over(frame: &RECT, rect: &RECT, ex_style: u32) -> bool {
  const TOOLWINDOW: u32 = 0x80;
  const LAYERED: u32 = 0x8_0000;
  const TRANSPARENT: u32 = 0x20;
  let overlay_style = ex_style & TOOLWINDOW != 0 || (ex_style & LAYERED != 0 && ex_style & TRANSPARENT != 0);
  let near = |a: i32, b: i32| (a - b).abs() <= 2;
  overlay_style
    && rect.right > rect.left
    && rect.bottom > rect.top
    && near(rect.left, frame.left)
    && near(rect.top, frame.top)
    && near(rect.right, frame.right)
    && near(rect.bottom, frame.bottom)
}

#[cfg(test)]
mod companion_tests {
  use std::collections::HashMap;

  use windows::Win32::Foundation::RECT;

  use super::{is_descendant, overlay_over};

  /// Every offset of a candidate's edges from -4 to 4 px and every style of
  /// three flags: an overlay exactly when all edges are within 2 px and the
  /// window is a tool window or click-through layered.
  #[test]
  fn overlays_are_laid_exactly_over_the_window() {
    let frame = RECT { left: 5, top: 45, right: 956, bottom: 1075 };
    let styles = [0u32, 0x80, 0x8_0000, 0x20, 0x8_0020, 0x8_00A0, 0x0800_0088];
    for &ex in &styles {
      let style_ok = ex & 0x80 != 0 || (ex & 0x8_0000 != 0 && ex & 0x20 != 0);
      for dl in -4..=4 {
        for dt in -4..=4 {
          for dr in [-4, -2, 0, 2, 4] {
            for db in [-4, -3, 0, 3, 4] {
              let rect = RECT { left: frame.left + dl, top: frame.top + dt, right: frame.right + dr, bottom: frame.bottom + db };
              let want = style_ok && [dl, dt, dr, db].iter().all(|d: &i32| d.abs() <= 2);
              assert_eq!(overlay_over(&frame, &rect, ex), want, "ex {ex:#x} offsets {dl} {dt} {dr} {db}");
            }
          }
        }
      }
    }
    // Discord's overlay over a tiled game (recorded): its window, its style
    let game = RECT { left: 5, top: 45, right: 956, bottom: 1075 };
    assert!(overlay_over(&game, &game, 0x0008_00A0));
    // a full-screen overlay (NVIDIA's) over a tile is not that tile's
    assert!(!overlay_over(&game, &RECT { left: 0, top: 0, right: 1920, bottom: 1080 }, 0x0800_0088));
  }

  #[test]
  fn descendants_follow_the_parent_chain() {
    // generated trees: each of 8 processes has a lower id as its parent (0: none)
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    for _ in 0..500 {
      let mut parents = HashMap::new();
      for pid in 1..=8u32 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        #[allow(clippy::cast_possible_truncation)]
        parents.insert(pid, (seed >> 33) as u32 % pid);
      }
      for pid in 1..=8u32 {
        let mut ancestors = Vec::new();
        let mut current = pid;
        while let Some(&parent) = parents.get(&current).filter(|&&p| p != 0) {
          ancestors.push(parent);
          current = parent;
        }
        for ancestor in 1..=8u32 {
          assert_eq!(is_descendant(pid, ancestor, &parents), ancestors.contains(&ancestor), "{pid} / {ancestor} in {parents:?}");
        }
      }
    }
  }

  #[test]
  fn a_reused_parent_id_ends_the_walk() {
    let looped = HashMap::from([(1, 2), (2, 1), (5, 5)]);
    assert!(!is_descendant(1, 3, &looped));
    assert!(!is_descendant(5, 4, &looped));
    assert!(is_descendant(1, 2, &looped));
  }
}
