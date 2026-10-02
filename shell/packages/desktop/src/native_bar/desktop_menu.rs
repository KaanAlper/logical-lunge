//! Our right-click menus of the desktop and of the bar's empty places
//! (menu.rs draws them). A right click on the desktop's empty space reaches
//! us as the core's `ll:desktop-menu` (its mouse hook keeps it from
//! Explorer); on an icon Explorer's own menu still opens.

use std::{os::windows::process::CommandExt, path::PathBuf};

use windows::{
  core::{w, GUID, PCWSTR},
  Win32::{
    Foundation::{HWND, LPARAM, POINT, WPARAM},
    UI::{
      Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEINPUT},
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  core_api,
  menu::{Item, MenuFocus},
  Ui,
};

/// The core's mark on keys and clicks it sends itself ("LLK1"): its hooks
/// let them through.
const LL_MARK: usize = 0x4C4C_4B31;
/// SHELLDLL_DefView's "Refresh" command
const CMD_REFRESH: usize = 0x7103;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
/// FOLDERID_Desktop
const DESKTOP: GUID = GUID::from_u128(0xB4BFCC3A_DB2C_424C_B029_7FE99A87C641);

#[link(name = "shell32")]
extern "system" {
  fn SHGetKnownFolderPath(id: *const GUID, flags: u32, token: isize, path: *mut *mut u16) -> i32;
}
#[link(name = "ole32")]
extern "system" {
  fn CoTaskMemFree(p: *const std::ffi::c_void);
}

fn cursor() -> POINT {
  let mut p = POINT::default();
  unsafe {
    let _ = GetCursorPos(&mut p);
  }
  p
}

/// The user's desktop folder (OneDrive moves it).
fn desktop_dir() -> Option<PathBuf> {
  unsafe {
    let mut p = std::ptr::null_mut();
    if SHGetKnownFolderPath(&DESKTOP, 0, 0, &mut p) != 0 || p.is_null() {
      return std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Desktop"));
    }
    let path = PCWSTR(p).to_string().ok().map(PathBuf::from);
    CoTaskMemFree(p.cast());
    path
  }
}

/// The window holding the desktop icons.
fn defview() -> Option<HWND> {
  unsafe {
    let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
    if let Ok(v) = FindWindowExW(progman, None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
      return Some(v);
    }
    // the classic layout moves it into a WorkerW
    let mut worker = HWND::default();
    loop {
      worker = FindWindowExW(None, worker, w!("WorkerW"), PCWSTR::null()).ok()?;
      if let Ok(v) = FindWindowExW(worker, None, w!("SHELLDLL_DefView"), PCWSTR::null()) {
        return Some(v);
      }
    }
  }
}

fn refresh_desktop() {
  if let Some(v) = defview() {
    unsafe {
      let _ = PostMessageW(v, WM_COMMAND, WPARAM(CMD_REFRESH), LPARAM(0));
    }
  }
}

/// "Yeni klasör", "Yeni klasör (2)" ... on the desktop.
fn new_folder(name: &str) {
  let Some(dir) = desktop_dir() else { return };
  for n in 1..1000 {
    let path = if n == 1 { dir.join(name) } else { dir.join(format!("{name} ({n})")) };
    if path.exists() {
      continue;
    }
    if let Err(err) = std::fs::create_dir(&path) {
      tracing::warn!("Desktop menu: new folder: {:?}", err);
    }
    break;
  }
  refresh_desktop();
}

/// Windows Terminal in the desktop folder, or a command prompt without it.
fn open_terminal() {
  let dir = desktop_dir().unwrap_or_else(|| PathBuf::from("C:\\"));
  if std::process::Command::new("wt.exe").arg("-d").arg(&dir).spawn().is_ok() {
    return;
  }
  let _ = std::process::Command::new("cmd.exe").current_dir(&dir).creation_flags(CREATE_NEW_CONSOLE).spawn();
}

/// Explorer's own desktop menu at `at`: a marked right click there (the
/// core's hook lets marked clicks through).
fn explorer_menu(at: POINT) {
  unsafe {
    let _ = SetCursorPos(at.x, at.y);
    let click = |flags| INPUT {
      r#type: INPUT_MOUSE,
      Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, dwExtraInfo: LL_MARK, ..Default::default() } },
    };
    let inputs = [click(MOUSEEVENTF_RIGHTDOWN), click(MOUSEEVENTF_RIGHTUP)];
    SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
  }
}

fn open_uri(uri: &str) {
  let _ = std::process::Command::new("explorer.exe").arg(uri).spawn();
}

impl Ui {
  /// The desktop's menu at the pointer (the core saw a right click on its
  /// empty space).
  pub(super) fn desktop_menu(&mut self) {
    let at = cursor();
    let tr = |s: &str| self.model.tr(s);
    let items = vec![
      Item::new("refresh", Some("refresh"), tr("Yenile")),
      Item::new("folder", Some("create_new_folder"), tr("Yeni klasör")),
      Item::sep(),
      Item::new("wallpaper", Some("wallpaper"), tr("Duvar kâğıdını değiştir")),
      Item::new("display", Some("desktop_windows"), tr("Görüntü ayarları")),
      Item::new("settings", Some("settings"), tr("Logical Lunge ayarları")),
      Item::new("terminal", Some("terminal"), tr("Terminal aç")),
      Item::sep(),
      Item::new("more", Some("more_horiz"), tr("Diğer seçenekler")),
    ];
    let folder = tr("Yeni klasör");
    self.menu_open(at, MenuFocus::Take, items, move |ui, id| match id {
      "refresh" => refresh_desktop(),
      "folder" => new_folder(&folder),
      "wallpaper" => (ui.emit)("ll:sidebar-open-page", serde_json::json!("walls")),
      "display" => open_uri("ms-settings:display"),
      "settings" => ui.settings_toggle(),
      "terminal" => open_terminal(),
      "more" => explorer_menu(at),
      _ => {}
    });
  }

  /// A right click on the bar where nothing has its own right click.
  pub(super) fn bar_menu(&mut self) {
    let at = cursor();
    let tr = |s: &str| self.model.tr(s);
    let items = vec![
      Item::new("taskmgr", Some("monitoring"), tr("Görev Yöneticisi")),
      Item::new("settings", Some("settings"), tr("Logical Lunge ayarları")),
      Item::sep(),
      Item::new("restart", Some("restart_alt"), tr("Masaüstünü yenile")),
    ];
    self.menu_open(at, MenuFocus::Take, items, move |ui, id| match id {
      "taskmgr" => {
        let _ = std::process::Command::new("taskmgr.exe").spawn();
      }
      "settings" => ui.settings_toggle(),
      "restart" => core_api::run_core(&["--restart-desktop"]),
      _ => {}
    });
  }
}
