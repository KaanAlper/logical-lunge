//! Our right-click menus of the desktop (its icons and its empty space) and
//! of the bar's empty places (menu.rs draws them). The core's hooks keep
//! every right click and menu key on the desktop from Explorer and send
//! `ll:desktop-menu` / `ll:desktop-menu-key`; Windows' own desktop menu never
//! opens. The commands are Explorer's (desktop_shell.rs): its view of the
//! desktop runs them.

use std::{os::windows::process::CommandExt, path::PathBuf};

use windows::{
  core::{w, Interface, GUID, HSTRING, PCWSTR},
  Win32::{
    Foundation::{HGLOBAL, POINT},
    System::{
      Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER},
      DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW},
      Memory::{GlobalLock, GlobalUnlock},
    },
    UI::{
      Shell::{
        DragQueryFileW, IShellLinkW, PropertiesSystem::PROPERTYKEY, ShellLink, FWF_AUTOARRANGE, FWF_NOICONS, FWF_SNAPTOGRID,
        HDROP,
      },
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  core_api,
  desktop_shell::{Desktop, Entry, NewMenu, PKEY_DATE, PKEY_NAME, PKEY_SIZE, PKEY_TYPE},
  menu::{Item, MenuFocus},
  Ui,
};

const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
/// FOLDERID_Desktop
const DESKTOP: GUID = GUID::from_u128(0xB4BFCC3A_DB2C_424C_B029_7FE99A87C641);
/// CF_HDROP
const CF_HDROP: u32 = 15;
/// DROPEFFECT_MOVE in "Preferred DropEffect" (Explorer's Cut)
const DROPEFFECT_MOVE: u32 = 2;
/// Explorer's desktop icon sizes: large, medium, small
const ICON_SIZES: [(i32, &str); 3] = [(96, "Büyük simgeler"), (48, "Orta simgeler"), (32, "Küçük simgeler")];

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

/// Files on the clipboard (Copy / Cut in Explorer) and whether they were cut.
fn clipboard_files() -> (Vec<String>, bool) {
  let mut files = Vec::new();
  let mut cut = false;
  unsafe {
    if IsClipboardFormatAvailable(CF_HDROP).is_err() || OpenClipboard(None).is_err() {
      return (files, cut);
    }
    if let Ok(h) = GetClipboardData(CF_HDROP) {
      let drop = HDROP(h.0);
      let n = DragQueryFileW(drop, u32::MAX, None);
      for i in 0..n {
        let len = DragQueryFileW(drop, i, None) as usize;
        let mut buf = vec![0u16; len + 1];
        let got = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
        files.push(String::from_utf16_lossy(&buf[..got]));
      }
    }
    let effect = RegisterClipboardFormatW(w!("Preferred DropEffect"));
    if effect != 0 {
      if let Ok(h) = GetClipboardData(effect) {
        let g = HGLOBAL(h.0);
        let p = GlobalLock(g) as *const u32;
        if !p.is_null() {
          cut = *p & DROPEFFECT_MOVE != 0;
          let _ = GlobalUnlock(g);
        }
      }
    }
    let _ = CloseClipboard();
  }
  (files, cut)
}

/// A free name on the desktop: "name.ext", "name (2).ext" ...
fn free_name(dir: &std::path::Path, stem: &str, ext: &str) -> Option<PathBuf> {
  (1..1000)
    .map(|n| if n == 1 { dir.join(format!("{stem}{ext}")) } else { dir.join(format!("{stem} ({n}){ext}")) })
    .find(|p| !p.exists())
}

fn new_text_document(name: &str) {
  let Some(dir) = desktop_dir() else { return };
  if let Some(path) = free_name(&dir, name, ".txt") {
    if let Err(err) = std::fs::write(&path, b"") {
      tracing::warn!("Desktop menu: new text document: {:?}", err);
    }
  }
}

/// Shortcuts on the desktop to the clipboard's files ("Paste shortcut").
fn paste_shortcuts(files: &[String], suffix: &str) {
  let Some(dir) = desktop_dir() else { return };
  for f in files {
    let src = PathBuf::from(f);
    let stem = src.file_stem().or(src.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(path) = free_name(&dir, &format!("{stem} - {suffix}"), ".lnk") else { continue };
    unsafe {
      let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else { continue };
      let _ = link.SetPath(&HSTRING::from(f.as_str()));
      if let Ok(file) = link.cast::<IPersistFile>() {
        if let Err(err) = file.Save(&HSTRING::from(path.to_string_lossy().as_ref()), true) {
          tracing::warn!("Desktop menu: paste shortcut: {:?}", err);
        }
      }
    }
  }
}

/// Windows Terminal in the desktop folder, or a command prompt without it;
/// started by Explorer so it is not elevated.
fn open_terminal(desk: Option<&Desktop>) {
  let dir = desktop_dir().unwrap_or_else(|| PathBuf::from("C:\\"));
  let dir_s = dir.to_string_lossy().into_owned();
  let has_wt = std::env::var_os("LOCALAPPDATA")
    .map(|l| PathBuf::from(l).join("Microsoft\\WindowsApps\\wt.exe"))
    .is_some_and(|p| p.exists());
  if let Some(d) = desk {
    let started = if has_wt { d.run("wt.exe", &format!("-d \"{dir_s}\""), &dir_s) } else { d.run("cmd.exe", "", &dir_s) };
    if started {
      return;
    }
  }
  if has_wt && std::process::Command::new("wt.exe").arg("-d").arg(&dir).spawn().is_ok() {
    return;
  }
  let _ = std::process::Command::new("cmd.exe").current_dir(&dir).creation_flags(CREATE_NEW_CONSOLE).spawn();
}

fn open_uri(uri: &str) {
  let _ = std::process::Command::new("explorer.exe").arg(uri).spawn();
}

fn same_key(a: PROPERTYKEY, b: PROPERTYKEY) -> bool {
  a.fmtid == b.fmtid && a.pid == b.pid
}

impl Ui {
  /// A right click on the desktop: the icon's menu on an icon, the
  /// desktop's menu on its empty space.
  pub(super) fn desktop_menu(&mut self) {
    let at = cursor();
    let desk = Desktop::open();
    match desk.as_ref().and_then(|d| d.item_at(at)) {
      Some(i) => {
        let Some(d) = desk else { return };
        d.select_for_menu(i);
        let sel = d.selection();
        self.desktop_icon_menu(d, sel, at);
      }
      None => self.desktop_space_menu(desk, at),
    }
  }

  /// The menu key or Shift+F10 on the desktop: the selection's menu at its
  /// focused icon, else the desktop's menu at the pointer.
  pub(super) fn desktop_menu_key(&mut self) {
    let desk = Desktop::open();
    match desk.as_ref().and_then(|d| d.focused_point()) {
      Some(at) => {
        let Some(d) = desk else { return };
        let sel = d.selection();
        self.desktop_icon_menu(d, sel, at);
      }
      None => self.desktop_space_menu(desk, cursor()),
    }
  }

  fn desktop_icon_menu(&mut self, d: Desktop, sel: Vec<Entry>, at: POINT) {
    if sel.is_empty() {
      return self.desktop_space_menu(Some(d), at);
    }
    let tr = |s: &str| self.model.tr(s);
    let one = sel.len() == 1;
    let first = sel[0].clone();
    let items = vec![
      Item::new("open", Some("open_in_new"), tr("Aç")),
      Item::new("runas", Some("shield_person"), tr("Yönetici olarak çalıştır")).enabled(one && first.runnable()),
      Item::new("openas", Some("apps"), tr("Birlikte aç…")).enabled(one && !first.folder && !first.link),
      Item::new("location", Some("folder_open"), tr("Dosya konumunu aç")).enabled(one && first.target.is_some()),
      Item::sep(),
      Item::new("cut", Some("content_cut"), tr("Kes")),
      Item::new("copy", Some("content_copy"), tr("Kopyala")),
      Item::new("link", Some("shortcut"), tr("Kısayol oluştur")),
      Item::sep(),
      Item::new("rename", Some("edit"), tr("Yeniden adlandır")).enabled(one),
      Item::new("delete", Some("delete"), tr("Sil")),
      Item::sep(),
      Item::new("properties", Some("info"), tr("Özellikler")),
    ];
    self.menu_open(at, MenuFocus::Take, items, move |_ui, id| match id {
      "open" | "runas" | "openas" | "cut" | "copy" | "link" | "delete" | "properties" => d.invoke(id),
      "rename" => d.rename(),
      "location" => {
        if let Some(target) = &first.target {
          let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{target}\"")).spawn();
        }
      }
      _ => {}
    });
  }

  fn desktop_space_menu(&mut self, d: Option<Desktop>, at: POINT) {
    let tr = |s: &str| self.model.tr(s);
    let (files, cut) = clipboard_files();
    let have = d.is_some();
    let size = d.as_ref().map_or(0, |d| d.icon_size());
    let near = ICON_SIZES.iter().min_by_key(|(s, _)| (s - size).abs()).map_or(48, |(s, _)| *s);
    let flag = |f| d.as_ref().is_some_and(|d| d.has_flag(f));
    let sorted = d.as_ref().and_then(|d| d.sorted_by());
    let is_sorted = |k| sorted.is_some_and(|s| same_key(s, k));
    let mut view: Vec<Item> = ICON_SIZES
      .iter()
      .map(|(s, label)| Item::new(&format!("size:{s}"), None, tr(label)).checked(have && *s == near).enabled(have))
      .collect();
    view.extend([
      Item::sep(),
      Item::new("auto", None, tr("Simgeleri otomatik düzenle")).checked(flag(FWF_AUTOARRANGE)).enabled(have),
      Item::new("grid", None, tr("Simgeleri ızgaraya hizala")).checked(flag(FWF_SNAPTOGRID)).enabled(have),
      Item::sep(),
      Item::new("icons", None, tr("Masaüstü simgelerini göster")).checked(have && !flag(FWF_NOICONS)).enabled(have),
    ]);
    let sort = vec![
      Item::new("sort:name", None, tr("Ad")).checked(is_sorted(PKEY_NAME)).enabled(have),
      Item::new("sort:size", None, tr("Boyut")).checked(is_sorted(PKEY_SIZE)).enabled(have),
      Item::new("sort:type", None, tr("Öğe türü")).checked(is_sorted(PKEY_TYPE)).enabled(have),
      Item::new("sort:date", None, tr("Değiştirme tarihi")).checked(is_sorted(PKEY_DATE)).enabled(have),
    ];
    // Windows' own "New" menu (what the installed apps registered, in its
    // order); ours only when its handler cannot be had
    let new_menu = NewMenu::open(d.as_ref()).filter(|(_, items)| !items.is_empty());
    let new = match &new_menu {
      Some((_, items)) => {
        let mut list = Vec::new();
        for (i, it) in items.iter().enumerate() {
          // folder and shortcut, then the file types (a type's verb is its extension)
          let file = it.verb.starts_with('.');
          if i > 0 && file != items[i - 1].verb.starts_with('.') {
            list.push(Item::sep());
          }
          let icon = match it.verb.as_str() {
            "NewFolder" => "create_new_folder",
            "NewLink" => "shortcut",
            _ => "description",
          };
          list.push(Item::new(&format!("new:{}", it.id), Some(icon), it.label.clone()));
        }
        list
      }
      None => vec![
        Item::new("new:folder", Some("create_new_folder"), tr("Klasör")).enabled(have),
        Item::new("new:text", Some("description"), tr("Metin belgesi")),
      ],
    };
    let items = vec![
      Item::new("view", Some("grid_view"), tr("Görüntüle")).submenu(view),
      Item::new("sort", Some("sort"), tr("Sıralama ölçütü")).submenu(sort),
      Item::new("refresh", Some("refresh"), tr("Yenile")).enabled(have),
      Item::sep(),
      Item::new("paste", Some("content_paste"), tr("Yapıştır")).enabled(have && !files.is_empty()),
      Item::new("pastelink", Some("shortcut"), tr("Kısayol yapıştır")).enabled(!files.is_empty() && !cut),
      Item::new("new", Some("add"), tr("Yeni")).submenu(new),
      Item::sep(),
      Item::new("widgets", Some("widgets"), tr("Widget ekle")).submenu(self.widgets_add_menu()),
      Item::new("wallpaper", Some("wallpaper"), tr("Duvar kâğıdını değiştir")),
      Item::new("display", Some("desktop_windows"), tr("Görüntü ayarları")),
      Item::new("settings", Some("settings"), tr("Logical Lunge ayarları")),
      Item::sep(),
      Item::new("terminal", Some("terminal"), tr("Terminal aç")),
    ];
    let folder_name = tr("Yeni klasör");
    let text_name = tr("Yeni Metin Belgesi");
    let link_suffix = tr("Kısayol");
    self.menu_open(at, MenuFocus::Take, items, move |ui, id| {
      let d = d.as_ref();
      if let (Some((menu, _)), Some(n)) = (&new_menu, id.strip_prefix("new:").and_then(|n| n.parse::<u32>().ok())) {
        return menu.invoke(n, at, d.map(Desktop::window).unwrap_or_default());
      }
      match id {
        "wallpaper" => crate::bus::publish(crate::bus::Event::SidebarOpenPage("walls".into())),
        "display" => open_uri("ms-settings:display"),
        "settings" => ui.settings_toggle(),
        "terminal" => open_terminal(d),
        "pastelink" => paste_shortcuts(&files, &link_suffix),
        "new:text" => new_text_document(&text_name),
        w if w.starts_with("widget:") => {
          ui.widgets_pick(w, at);
        }
        _ => {
          let Some(d) = d else { return };
          match id {
            "refresh" => d.refresh(),
            "auto" => d.toggle_flag(FWF_AUTOARRANGE),
            "grid" => d.toggle_flag(FWF_SNAPTOGRID),
            "icons" => d.toggle_flag(FWF_NOICONS),
            "sort:name" => d.sort_by(PKEY_NAME),
            "sort:size" => d.sort_by(PKEY_SIZE),
            "sort:type" => d.sort_by(PKEY_TYPE),
            "sort:date" => d.sort_by(PKEY_DATE),
            "paste" => d.paste(&files, cut),
            "new:folder" => {
              if !d.new_folder(&folder_name) {
                tracing::warn!("Desktop menu: new folder failed");
              }
            }
            other => {
              if let Some(size) = other.strip_prefix("size:").and_then(|s| s.parse().ok()) {
                d.set_icon_size(size);
              }
            }
          }
        }
      }
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
