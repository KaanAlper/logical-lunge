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

use crate::desktop_shell::{self, Desktop, PKEY_DATE, PKEY_NAME, PKEY_SIZE, PKEY_TYPE};

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

/// Every selected icon through the core's launcher (checked first; as the
/// user; failures on our card with "Birlikte aç" where nothing opens it),
/// never Explorer's open, which shows Windows' box for a dead shortcut.
fn open_entries(sel: &[desktop_shell::Entry]) {
  for e in sel {
    let mut route = String::from("/launch?file=");
    for b in e.path.bytes() {
      if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
        route.push(b as char);
      } else {
        route.push_str(&format!("%{b:02X}"));
      }
    }
    crate::native_bar::core_api::post_async(route);
  }
}

fn open_uri(uri: &str) {
  let _ = std::process::Command::new("explorer.exe").arg(uri).spawn();
}

fn same_key(a: PROPERTYKEY, b: PROPERTYKEY) -> bool {
  a.fmtid == b.fmtid && a.pid == b.pid
}

#[derive(Clone, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MenuItem {
  id: String, label: String, icon: String, enabled: bool, checked: bool,
  children: Vec<MenuItem>,
}
fn item(id: &str, label: &str, icon: &str) -> MenuItem {
  MenuItem { id: id.into(), label: label.into(), icon: icon.into(), enabled: true, ..Default::default() }
}
impl MenuItem {
  fn enabled(mut self, enabled: bool) -> Self { self.enabled = enabled; self }
  fn checked(mut self, checked: bool) -> Self { self.checked = checked; self }
  fn children(mut self, children: Vec<Self>) -> Self { self.children = children; self }
}
#[derive(Clone, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MenuModel { pub x: i32, pub y: i32, pub items: Vec<MenuItem>, pub icon_menu: bool, #[serde(skip)] new_entries: Vec<(String,String)> }
#[derive(Default)]
pub struct MenuState(pub std::sync::Mutex<MenuModel>);

struct Sta(bool);
impl Sta { fn new() -> Self { Self(unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) }.is_ok()) } }
impl Drop for Sta { fn drop(&mut self) { if self.0 { unsafe { windows::Win32::System::Com::CoUninitialize() }; } } }

fn model(keyboard: bool) -> MenuModel {
  let _com = Sta::new();
  let desk = Desktop::open();
  let new_entries = desktop_shell::new_entries().unwrap_or_default();
  let at = if keyboard { desk.as_ref().and_then(Desktop::focused_point).unwrap_or_else(cursor) } else { cursor() };
  let selected = if let Some(d) = &desk {
    if !keyboard { if let Some(i) = d.item_at(at) { d.select_for_menu(i); d.selection() } else { vec![] } }
    else { d.selection() }
  } else { vec![] };
  let items = if let Some(first) = selected.first() {
    let one = selected.len() == 1;
    vec![item("open", "Aç", "open_in_new"), item("runas", "Yönetici olarak çalıştır", "shield_person").enabled(one && first.runnable()),
      item("openas", "Birlikte aç…", "apps").enabled(one && !first.folder && !first.link),
      item("location", "Dosya konumunu aç", "folder_open").enabled(one && first.target.is_some()),
      item("cut", "Kes", "content_cut"), item("copy", "Kopyala", "content_copy"), item("link", "Kısayol oluştur", "shortcut"),
      item("rename", "Yeniden adlandır", "edit").enabled(one), item("delete", "Sil", "delete"), item("properties", "Özellikler", "info")]
  } else {
    let have = desk.is_some();
    let flag = |f| desk.as_ref().is_some_and(|d| d.has_flag(f));
    let size = desk.as_ref().map_or(48, Desktop::icon_size);
    let near = ICON_SIZES.iter().min_by_key(|(s, _)| (s - size).abs()).map_or(48, |(s, _)| *s);
    let sorted = desk.as_ref().and_then(Desktop::sorted_by);
    let (files, cut) = clipboard_files();
    desktop_shell::refresh_new_entries();
    let new_items = if new_entries.is_empty() { vec![item("new:folder", "Klasör", "create_new_folder").enabled(have), item("new:text", "Metin belgesi", "description")] }
      else { new_entries.iter().enumerate().map(|(i, (label, _))| item(&format!("new:{i}"), label, "description")).collect() };
    let mut view: Vec<_> = ICON_SIZES.iter().map(|(s, label)| item(&format!("size:{s}"), label, "").checked(*s == near).enabled(have)).collect();
    view.extend([item("auto", "Simgeleri otomatik düzenle", "").checked(flag(FWF_AUTOARRANGE)).enabled(have),
      item("grid", "Simgeleri ızgaraya hizala", "").checked(flag(FWF_SNAPTOGRID)).enabled(have),
      item("icons", "Masaüstü simgelerini göster", "").checked(have && !flag(FWF_NOICONS)).enabled(have)]);
    let sorts = [("name", "Ad", PKEY_NAME), ("size", "Boyut", PKEY_SIZE), ("type", "Öğe türü", PKEY_TYPE), ("date", "Değiştirme tarihi", PKEY_DATE)]
      .into_iter().map(|(id, label, key)| item(&format!("sort:{id}"), label, "").checked(sorted.is_some_and(|s| same_key(s, key))).enabled(have)).collect();
    let widgets = [("clock", "Saat"), ("media", "Medya"), ("system", "Sistem"), ("weather", "Hava durumu"), ("agenda", "Ajanda"), ("note", "Notlar")]
      .into_iter().map(|(id, label)| item(&format!("widget:{id}"), label, "widgets")).collect();
    vec![item("view", "Görüntüle", "grid_view").children(view), item("sort", "Sıralama ölçütü", "sort").children(sorts),
      item("refresh", "Yenile", "refresh").enabled(have), item("paste", "Yapıştır", "content_paste").enabled(have && !files.is_empty()),
      item("pastelink", "Kısayol yapıştır", "shortcut").enabled(!files.is_empty() && !cut), item("new", "Yeni", "add").children(new_items),
      item("widgets", "Widget ekle", "widgets").children(widgets), item("terminal", "Terminali burada aç", "terminal"),
      item("wallpaper", "Duvar kâğıdı", "wallpaper"), item("display", "Görüntü ayarları", "desktop_windows"), item("settings", "Logical Lunge ayarları", "settings")]
  };
  MenuModel { x: at.x, y: at.y, items, icon_menu: !selected.is_empty(), new_entries }
}

#[tauri::command]
pub fn desktop_menu_current(state: tauri::State<MenuState>) -> Result<MenuModel, String> {
  state.0.lock().map(|m| m.clone()).map_err(|e| e.to_string())
}
pub fn listen(app: &tauri::AppHandle) {
  use tauri::{Listener, Manager, Emitter};
  app.manage(MenuState::default());
  desktop_shell::refresh_new_entries();
  // a double click on a desktop icon / Enter on the desktop: we open the
  // selection (empty space: nothing, as in Explorer)
  for event in ["ll:desktop-open", "ll:desktop-open-key"] {
    app.listen(event, move |_| {
      std::thread::spawn(move || {
        let _com = Sta::new();
        let Some(d) = Desktop::open() else { return };
        if !event.ends_with("-key") {
          let Some(i) = d.item_at(cursor()) else { return };
          d.select_for_menu(i);
        }
        open_entries(&d.selection());
      });
    });
  }
  for event in ["ll:desktop-menu", "ll:desktop-menu-key"] {
    let app = app.clone();
    let handle = app.clone();
    app.listen(event, move |_| {
      let app = handle.clone();
      tauri::async_runtime::spawn(async move {
        let Ok(menu) = tokio::task::spawn_blocking(move || model(event.ends_with("-key"))).await else { return };
        if let Ok(mut saved) = app.state::<MenuState>().0.lock() { *saved = menu.clone(); }
        for window in app.webview_windows().into_values() {
          if window.title().ok().as_deref() != Some("Logical Lunge · desktop-menu") { continue; }
          let scale = crate::web_scale::factor() as f64;
          let dpi = window.scale_factor().unwrap_or(1.0);
          let width = (310.0 * scale * dpi) as i32;
          let height = (menu.items.len() as f64 * 35.0 + 16.0) * scale * dpi;
          let monitor = window.available_monitors().ok().and_then(|ms| ms.into_iter().find(|m| {
            let p=m.position(); let s=m.size(); menu.x>=p.x && menu.y>=p.y && menu.x<p.x+s.width as i32 && menu.y<p.y+s.height as i32
          }));
          let (x,y,h) = monitor.map_or((menu.x,menu.y,height as i32), |m| {
            let p=m.position(); let s=m.size(); let w=width.min(s.width as i32); let h=(height as i32).min(s.height as i32);
            (menu.x.clamp(p.x,p.x+s.width as i32-w),menu.y.clamp(p.y,p.y+s.height as i32-h),h)
          });
          let _ = window.set_size(tauri::PhysicalSize::new(width,h));
          let _ = window.set_position(tauri::PhysicalPosition::new(x,y));
          let _ = window.emit("ll:desktop-menu-model", &menu);
          let _ = window.show(); let _ = window.set_focus();
        }
      });
    });
  }
}

fn allowed(items: &[MenuItem], id: &str) -> bool {
  items.iter().any(|i| (i.id == id && i.enabled && i.children.is_empty()) || allowed(&i.children, id))
}
#[tauri::command]
pub async fn desktop_menu_action(app: tauri::AppHandle, id: String) -> Result<(), String> {
  use tauri::{Manager, Emitter};
  let saved = app.state::<MenuState>().0.lock().map_err(|e|e.to_string())?.clone();
  if !allowed(&saved.items, &id) { return Err("Menü komutu kullanılamıyor".into()); }
  if let Some(kind) = id.strip_prefix("widget:") {
    app.emit("ll:desktop-widget-add", serde_json::json!({"kind":kind,"x":saved.x,"y":saved.y})).map_err(|e|e.to_string())?;
    return Ok(());
  }
  if id == "settings" { return app.emit("ll:settings-toggle", ()).map_err(|e|e.to_string()); }
  if id == "wallpaper" { return app.emit("ll:sidebar-open-page", "walls").map_err(|e|e.to_string()); }
  tokio::task::spawn_blocking(move || {
    let _com = Sta::new(); let d = Desktop::open();
    match id.as_str() {
      "display" => open_uri("ms-settings:display"),
      "terminal" => open_terminal(d.as_ref()),
      "new:text" => new_text_document("Yeni Metin Belgesi"),
      "pastelink" => paste_shortcuts(&clipboard_files().0, "Kısayol"),
      other if other.starts_with("new:") => {
        if other == "new:folder" { if let Some(d)=&d { d.new_folder("Yeni klasör"); } }
        else if let Some((label,verb))=other[4..].parse::<usize>().ok().and_then(|i|saved.new_entries.get(i).cloned()) {
          desktop_shell::make_new(label,verb,POINT{x:saved.x,y:saved.y});
        }
      }
      _ => {
        let d = d.ok_or("Masaüstü görünümü bulunamadı")?;
        match id.as_str() {
          "open" if saved.icon_menu => open_entries(&d.selection()),
          "runas"|"openas"|"cut"|"copy"|"link"|"delete"|"properties" if saved.icon_menu => d.invoke(&id),
          "rename" if saved.icon_menu => d.rename(),
          "location" if saved.icon_menu => { if let Some(target)=d.selection().first().and_then(|e|e.target.clone()) { let _=std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{target}\"")).spawn(); } }
          "refresh"=>d.refresh(), "auto"=>d.toggle_flag(FWF_AUTOARRANGE), "grid"=>d.toggle_flag(FWF_SNAPTOGRID), "icons"=>d.toggle_flag(FWF_NOICONS),
          "sort:name"=>d.sort_by(PKEY_NAME), "sort:size"=>d.sort_by(PKEY_SIZE), "sort:type"=>d.sort_by(PKEY_TYPE), "sort:date"=>d.sort_by(PKEY_DATE),
          "paste"=>{let (files,cut)=clipboard_files();d.paste(&files,cut);},
          "size:96"=>d.set_icon_size(96), "size:48"=>d.set_icon_size(48), "size:32"=>d.set_icon_size(32),
          _=>return Err("Menü komutu kullanılamıyor".into()),
        }
      }
    }
    Ok(())
  }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
pub async fn app_properties(path: String) -> Result<(), String> {
  tokio::task::spawn_blocking(move || { let _com=Sta::new(); desktop_shell::properties(&path); }).await.map_err(|e|e.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn disabled_and_container_commands_cannot_be_invoked() {
    let items=vec![item("group","","" ).children(vec![item("run","",""),item("delete","","" ).enabled(false)])];
    assert!(allowed(&items,"run")); assert!(!allowed(&items,"delete")); assert!(!allowed(&items,"group")); assert!(!allowed(&items,"invented"));
  }
}

