//! "Dosya konumunu aç": a File Explorer window with the item selected.
//!
//! Through the shell's own call (`SHOpenFolderAndSelectItems`) rather than
//! an `explorer.exe /select,<path>` command line: a path with spaces or
//! quotes was split or wrapped in quotes by the command line, Explorer did
//! not understand it and opened its default folder (Documents) instead.

use std::path::{Path, PathBuf};

/// Opens Explorer at `path` with it selected, on a thread of its own (the
/// call can wait for Explorer). A missing item opens its nearest existing
/// folder; nothing at all left reports through `failed` (a card).
pub fn reveal_in_explorer(path: impl Into<PathBuf>, failed: impl FnOnce(String) + Send + 'static) {
  let path = path.into();
  std::thread::spawn(move || {
    if let Err(err) = reveal(&path) {
      failed(err);
    }
  });
}

#[cfg(windows)]
fn reveal(path: &Path) -> Result<(), String> {
  use windows::{
    core::HSTRING,
    Win32::{
      System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
      UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems},
    },
  };

  // the item itself, else the closest folder above it that still exists
  let target = std::iter::successors(Some(path), |p| p.parent())
    .find(|p| p.exists())
    .ok_or_else(|| format!("Konum bulunamadı: {}", path.display()))?;

  unsafe {
    let inited = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
    let pidl = ILCreateFromPathW(&HSTRING::from(target.as_os_str()));
    let result = if pidl.is_null() {
      Err(format!("Konum açılamadı: {}", target.display()))
    } else {
      // no child list: the item is selected in its parent folder
      let opened = SHOpenFolderAndSelectItems(pidl, None, 0).map_err(|e| e.message().to_string());
      ILFree(Some(pidl));
      opened
    };
    if inited {
      CoUninitialize();
    }
    result
  }
}

#[cfg(not(windows))]
fn reveal(path: &Path) -> Result<(), String> {
  Err(format!("Konum açılamadı: {}", path.display()))
}
