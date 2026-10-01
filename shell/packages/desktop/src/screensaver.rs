//! Read and change the current user's Windows screen saver settings.
//! SystemParametersInfo updates the live session and the user profile.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
  enabled: bool,
  minutes: u32,
  secure: bool,
  selected: String,
  choices: Vec<Choice>,
}

#[derive(Serialize)]
pub struct Choice {
  name: String,
  path: String,
}

#[cfg(windows)]
mod win {
  use super::{Choice, State};
  use std::{ffi::c_void, path::{Path, PathBuf}};
  use windows::{
    core::w,
    Win32::{
      Foundation::{BOOL, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
      System::Registry::{RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ},
      UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETSCREENSAVEACTIVE, SPI_GETSCREENSAVESECURE,
        SPI_GETSCREENSAVETIMEOUT, SPI_SETSCREENSAVEACTIVE, SPI_SETSCREENSAVESECURE,
        SPI_SETSCREENSAVETIMEOUT, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
      },
    },
  };

  fn selected() -> Result<String, String> {
    let mut bytes = 0u32;
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Control Panel\\Desktop"), w!("SCRNSAVE.EXE"), RRF_RT_REG_SZ, None, None, Some(&mut bytes)) };
    if status == ERROR_FILE_NOT_FOUND { return Ok(String::new()); }
    if status != ERROR_SUCCESS { return Err(format!("Ekran koruyucu okunamadı: {}", status.0)); }
    let mut data = vec![0u16; (bytes as usize).div_ceil(2)];
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Control Panel\\Desktop"), w!("SCRNSAVE.EXE"), RRF_RT_REG_SZ, None, Some(data.as_mut_ptr().cast::<c_void>()), Some(&mut bytes)) };
    if status != ERROR_SUCCESS { return Err(format!("Ekran koruyucu okunamadı: {}", status.0)); }
    let end = data.iter().position(|&c| c == 0).unwrap_or(data.len());
    Ok(String::from_utf16_lossy(&data[..end]))
  }

  fn system_choices(current: &str) -> Vec<Choice> {
    let root = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let mut choices = Vec::new();
    for folder in ["System32", "SysWOW64"] {
      if let Ok(entries) = std::fs::read_dir(root.join(folder)) {
        for entry in entries.flatten() {
          let path = entry.path();
          if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("scr")) {
            choices.push(Choice {
              name: path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
              path: path.to_string_lossy().into_owned(),
            });
          }
        }
      }
    }
    if !current.is_empty() && Path::new(current).is_file() && !choices.iter().any(|c| c.path.eq_ignore_ascii_case(current)) {
      choices.push(Choice { name: Path::new(current).file_stem().unwrap_or_default().to_string_lossy().into_owned(), path: current.into() });
    }
    choices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    choices
  }

  fn get_bool(action: windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_ACTION) -> Result<bool, String> {
    let mut value = BOOL(0);
    unsafe { SystemParametersInfoW(action, 0, Some((&mut value as *mut BOOL).cast::<c_void>()), Default::default()) }.map_err(|e| e.to_string())?;
    Ok(value.as_bool())
  }

  pub fn state() -> Result<State, String> {
    let mut seconds = 0u32;
    unsafe { SystemParametersInfoW(SPI_GETSCREENSAVETIMEOUT, 0, Some((&mut seconds as *mut u32).cast::<c_void>()), Default::default()) }.map_err(|e| e.to_string())?;
    let selected = selected()?;
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
pub use win::{set, state};

#[cfg(not(windows))]
pub fn state() -> Result<State, String> { Err("Ekran koruyucu yalnızca Windows'ta kullanılabilir".into()) }

#[cfg(not(windows))]
pub fn set(_enabled: bool, _minutes: u32, _secure: bool, _selected: &str) -> Result<State, String> { state() }

#[cfg(all(test, windows))]
#[test]
fn reads_windows_settings_without_changing_them() {
  let state = state().expect("read screen saver state");
  assert!(state.minutes >= 1);
}
