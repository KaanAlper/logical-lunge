//! Read and change the current user's Windows screen saver settings.
//! SystemParametersInfo updates the live session and the user profile.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
  pub enabled: bool,
  pub minutes: u32,
  pub secure: bool,
  pub selected: String,
  pub choices: Vec<Choice>,
}

#[derive(Serialize)]
pub struct Choice {
  pub name: String,
  pub path: String,
}

#[cfg(windows)]
mod win {
  use super::{Choice, State};
  use crate::common::windows::read_reg_string;
  use std::{ffi::c_void, path::{Path, PathBuf}};
  use windows::{
    core::{w, HSTRING, PWSTR},
    Win32::{
      Foundation::{FreeLibrary, BOOL, ERROR_SUCCESS, HANDLE, HINSTANCE},
      System::{
        LibraryLoader::{LoadLibraryExW, LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE},
        Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ},
      },
      UI::WindowsAndMessaging::{
        LoadStringW, SystemParametersInfoW, SPI_GETSCREENSAVEACTIVE, SPI_GETSCREENSAVESECURE,
        SPI_GETSCREENSAVETIMEOUT, SPI_SETSCREENSAVEACTIVE, SPI_SETSCREENSAVESECURE,
        SPI_SETSCREENSAVETIMEOUT, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
      },
    },
  };

  fn selected() -> Result<String, String> {
    read_reg_string(HKEY_CURRENT_USER, "Control Panel\\Desktop", "SCRNSAVE.EXE")
      .map(Option::unwrap_or_default)
      .map_err(|code| format!("Ekran koruyucu okunamadı: {}", code))
  }

  /// The name Windows itself shows for a screen saver: string 1 of the .scr
  /// (its description, in the system language); the file name without one.
  fn friendly_name(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let file = HSTRING::from(path.to_string_lossy().as_ref());
    let flags = LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE;
    let Ok(module) = (unsafe { LoadLibraryExW(&file, HANDLE::default(), flags) }) else { return stem };
    let mut text = [0u16; 128];
    let len = unsafe { LoadStringW(HINSTANCE(module.0), 1, PWSTR(text.as_mut_ptr()), text.len() as i32) };
    let _ = unsafe { FreeLibrary(module) };
    if len > 0 { String::from_utf16_lossy(&text[..len as usize]) } else { stem }
  }

  /// `.scr` files under %LOCALAPPDATA%\LogicalLunge\screensavers, two
  /// folders deep.
  fn library() -> Vec<PathBuf> {
    let Some(data) = std::env::var_os("LOCALAPPDATA") else { return Vec::new() };
    let mut found = Vec::new();
    let mut folders = vec![(PathBuf::from(data).join("LogicalLunge").join("screensavers"), 0)];
    while let Some((folder, depth)) = folders.pop() {
      let Ok(entries) = std::fs::read_dir(&folder) else { continue };
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
          if depth < 2 { folders.push((path, depth + 1)); }
        } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("scr")) {
          found.push(path);
        }
      }
    }
    found
  }

  /// Logical Lunge's own video screen saver, next to this program (the
  /// package's app folder).
  fn ours() -> Option<PathBuf> {
    let scr = std::env::current_exe().ok()?.parent()?.join("LogicalLunge.scr");
    scr.is_file().then_some(scr)
  }

  /// The long form of a path Windows may keep short (`PROGRA~1`), so the
  /// selected saver matches its entry.
  fn long_path(path: &str) -> String {
    if path.is_empty() { return String::new(); }
    match std::fs::canonicalize(path) {
      Ok(p) => {
        let s = p.to_string_lossy().into_owned();
        s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
      }
      Err(_) => path.to_string(),
    }
  }

  /// Windows starts the saver from SCRNSAVE.EXE without quotes: a path with
  /// spaces (Program Files) is stored in its short form.
  fn short_path(path: &str) -> String {
    #[link(name = "kernel32")]
    extern "system" {
      fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    if !path.contains(' ') { return path.to_string(); }
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 520];
    let n = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n > 0 && n < buf.len() { String::from_utf16_lossy(&buf[..n]) } else { path.to_string() }
  }

  fn system_choices(current: &str) -> Vec<Choice> {
    let root = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let file_of = |path: &Path| path.file_name().map(|f| f.to_string_lossy().to_lowercase());
    let mut choices: Vec<Choice> = Vec::new();
    // SysWOW64 holds 32-bit copies of the same savers: each one is listed once, from System32
    for folder in ["System32", "SysWOW64"] {
      if let Ok(entries) = std::fs::read_dir(root.join(folder)) {
        for entry in entries.flatten() {
          let path = entry.path();
          let is_scr = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("scr"));
          if is_scr && !choices.iter().any(|c| file_of(Path::new(&c.path)) == file_of(&path)) {
            choices.push(Choice { name: friendly_name(&path), path: path.to_string_lossy().into_owned() });
          }
        }
      }
    }
    if let Some(scr) = ours() {
      choices.push(Choice { name: "Logical Lunge".into(), path: scr.to_string_lossy().into_owned() });
    }
    // imported ones (the core's --saver-pick), a zip's in their own folder
    for path in library() {
      if !choices.iter().any(|c| c.path.eq_ignore_ascii_case(&path.to_string_lossy())) {
        choices.push(Choice { name: friendly_name(&path), path: path.to_string_lossy().into_owned() });
      }
    }
    if !current.is_empty() && Path::new(current).is_file() && !choices.iter().any(|c| c.path.eq_ignore_ascii_case(current)) {
      choices.push(Choice { name: friendly_name(Path::new(current)), path: current.into() });
    }
    choices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    choices
  }

  /// Runs a listed screen saver: full screen (`/s`) or its own settings
  /// (`/c`). Only the savers the page lists can be started from here.
  pub fn run(path: &str, configure: bool) -> Result<(), String> {
    if !system_choices(&long_path(&selected()?)).iter().any(|c| c.path.eq_ignore_ascii_case(path)) {
      return Err("Bilinmeyen ekran koruyucu".into());
    }
    std::process::Command::new(path)
      .arg(if configure { "/c" } else { "/s" })
      .spawn()
      .map(|_| ())
      .map_err(|err| format!("Ekran koruyucu açılamadı: {err}"))
  }

  fn get_bool(action: windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_ACTION) -> Result<bool, String> {
    let mut value = BOOL(0);
    unsafe { SystemParametersInfoW(action, 0, Some((&mut value as *mut BOOL).cast::<c_void>()), Default::default()) }.map_err(|e| e.to_string())?;
    Ok(value.as_bool())
  }

  pub fn state() -> Result<State, String> {
    let mut seconds = 0u32;
    unsafe { SystemParametersInfoW(SPI_GETSCREENSAVETIMEOUT, 0, Some((&mut seconds as *mut u32).cast::<c_void>()), Default::default()) }.map_err(|e| e.to_string())?;
    let selected = long_path(&selected()?);
    Ok(State {
      enabled: get_bool(SPI_GETSCREENSAVEACTIVE)? && !selected.is_empty(),
      minutes: (seconds / 60).max(1),
      secure: get_bool(SPI_GETSCREENSAVESECURE)?,
      choices: system_choices(&selected),
      selected,
    })
  }

  pub fn set(enabled: bool, minutes: u32, secure: bool, selected: &str) -> Result<State, String> {
    if !(1..=120).contains(&minutes) { return Err("Bekleme süresi 1–120 dakika olmalı".into()); }
    if enabled {
      let path = Path::new(selected);
      if !path.is_file() || !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("scr")) {
        return Err("Geçerli bir ekran koruyucu seçin".into());
      }
    }
    if !selected.is_empty() {
      let selected = short_path(selected);
      let wide: Vec<u16> = selected.encode_utf16().chain(std::iter::once(0)).collect();
      let status = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, w!("Control Panel\\Desktop"), w!("SCRNSAVE.EXE"), REG_SZ.0,
        Some(wide.as_ptr().cast::<c_void>()), (wide.len() * 2) as u32) };
      if status != ERROR_SUCCESS { return Err(format!("Ekran koruyucu kaydedilemedi: {}", status.0)); }
    }
    let flags = SPIF_UPDATEINIFILE | SPIF_SENDCHANGE;
    unsafe {
      SystemParametersInfoW(SPI_SETSCREENSAVETIMEOUT, minutes * 60, None, flags).map_err(|e| e.to_string())?;
      SystemParametersInfoW(SPI_SETSCREENSAVESECURE, secure as u32, None, flags).map_err(|e| e.to_string())?;
      SystemParametersInfoW(SPI_SETSCREENSAVEACTIVE, enabled as u32, None, flags).map_err(|e| e.to_string())?;
    }
    state()
  }
}

#[cfg(windows)]
pub use win::{run, set, state};

#[cfg(not(windows))]
pub fn state() -> Result<State, String> { Err("Ekran koruyucu yalnızca Windows'ta kullanılabilir".into()) }

#[cfg(not(windows))]
pub fn set(_enabled: bool, _minutes: u32, _secure: bool, _selected: &str) -> Result<State, String> { state() }

#[cfg(not(windows))]
pub fn run(_path: &str, _configure: bool) -> Result<(), String> { state().map(|_| ()) }

#[cfg(all(test, windows))]
#[test]
fn reads_windows_settings_without_changing_them() {
  let state = state().expect("read screen saver state");
  assert!(state.minutes >= 1);
}
