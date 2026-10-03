//! Explorer's own view of the desktop, driven from our menus. The desktop
//! window's IShellView and IFolderView2 come from Explorer through
//! IShellWindows (SWC_DESKTOP), so every command runs where Windows runs it:
//! in Explorer, unelevated, with its undo, its progress and its rename box.
//! Our process (elevated) only asks; nothing here imitates Explorer.

use windows::{
  core::{w, Interface, BSTR, GUID, PCSTR, PCWSTR, PSTR, PWSTR, VARIANT},
  Win32::{
    Foundation::{HWND, LPARAM, POINT, WPARAM},
    Graphics::Gdi::{ClientToScreen, ScreenToClient},
    System::{
      Com::{CoCreateInstance, CoTaskMemFree, IDispatch, IServiceProvider, CLSCTX_ALL, CLSCTX_INPROC_SERVER},
      Ole::IObjectWithSite,
      Registry::HKEY,
    },
    UI::{
      Shell::{
        Folder, IContextMenu3, IFolderView2, IShellBrowser, IShellDispatch2, IShellExtInit, IShellFolderViewDual,
        IShellItem, IShellItem2, IShellView, IShellWindows, PropertiesSystem::PROPERTYKEY, SHGetKnownFolderIDList,
        SHObjectProperties, ShellWindows, CLSID_NewMenu, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO,
        CMINVOKECOMMANDINFOEX, FOLDERFLAGS, FOLDERID_Desktop, FVM_ICON, GCS_VERBW, SHOP_FILEPATH, SIGDN_FILESYSPATH,
        SID_STopLevelBrowser, SORTCOLUMN, SORT_ASCENDING, SVGIO_ALLVIEW, SVGIO_BACKGROUND, SVSI_DESELECTOTHERS,
        SVSI_FOCUSED, SVSI_SELECT, SWC_DESKTOP, SWFO_NEEDDISPATCH,
      },
      WindowsAndMessaging::{
        CreatePopupMenu, DestroyMenu, FindWindowExW, GetAncestor, GetMenuItemCount, GetMenuItemInfoW, GetSubMenu,
        SetForegroundWindow, GA_ROOT, HMENU, MENUITEMINFOW, MFT_SEPARATOR, MIIM_FTYPE, MIIM_ID, MIIM_STRING,
        SW_SHOWNORMAL, WM_INITMENUPOPUP,
      },
    },
  },
};

/// SFGAO_FOLDER / SFGAO_LINK
const SFGAO_FOLDER: u32 = 0x2000_0000;
const SFGAO_LINK: u32 = 0x0001_0000;

/// The storage property set (name, size, type, date modified).
const FMTID_STORAGE: GUID = GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC);
pub const PKEY_NAME: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 10 };
pub const PKEY_SIZE: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 12 };
pub const PKEY_TYPE: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 4 };
pub const PKEY_DATE: PROPERTYKEY = PROPERTYKEY { fmtid: FMTID_STORAGE, pid: 14 };
/// System.Link.TargetParsingPath: a shortcut's target
const PKEY_LINK_TARGET: PROPERTYKEY =
  PROPERTYKEY { fmtid: GUID::from_u128(0xB9B4B3FC_2B51_4A42_B5D8_324146AFCF25), pid: 2 };

/// One selected item.
#[derive(Clone, Debug, Default)]
pub struct Entry {
  pub path: String,
  pub folder: bool,
  pub link: bool,
  /// a shortcut's target
  pub target: Option<String>,
}

impl Entry {
  /// Runs as a program (as administrator makes sense).
  pub fn runnable(&self) -> bool {
    if self.folder {
      return false;
    }
    let p = self.target.as_deref().unwrap_or(&self.path).to_ascii_lowercase();
    [".exe", ".bat", ".cmd", ".msi", ".com", ".ps1"].iter().any(|e| p.ends_with(e)) || (self.link && self.target.is_none())
  }
}

/// The desktop's view in Explorer.
pub struct Desktop {
  shell: IShellView,
  view: IFolderView2,
  /// the icon list (the view's own window when it has none)
  list: HWND,
}

fn pwstr(p: PWSTR) -> Option<String> {
  if p.is_null() {
    return None;
  }
  let s = unsafe { p.to_string() }.ok();
  unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
  s
}

impl Desktop {
  pub fn open() -> Option<Desktop> {
    unsafe {
      let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
      let empty = VARIANT::default();
      let mut hwnd = 0i32;
      let disp: IDispatch = windows.FindWindowSW(&empty, &empty, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH).ok()?;
      let sp: IServiceProvider = disp.cast().ok()?;
      let browser: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser).ok()?;
      let shell = browser.QueryActiveShellView().ok()?;
      let view: IFolderView2 = shell.cast().ok()?;
      let defview = shell.GetWindow().ok()?;
      let list = FindWindowExW(defview, None, w!("SysListView32"), PCWSTR::null()).unwrap_or(defview);
      Some(Desktop { shell, view, list })
    }
  }

  fn flags(&self) -> u32 {
    unsafe { self.view.GetCurrentFolderFlags() }.unwrap_or(0)
  }

  pub fn has_flag(&self, flag: FOLDERFLAGS) -> bool {
    self.flags() & flag.0 as u32 != 0
  }

  pub fn toggle_flag(&self, flag: FOLDERFLAGS) {
    let on = !self.has_flag(flag);
    let f = flag.0 as u32;
    unsafe {
      let _ = self.view.SetCurrentFolderFlags(f, if on { f } else { 0 });
    }
  }

  /// The icon under a screen point (none while the icons are hidden).
  pub fn item_at(&self, screen: POINT) -> Option<i32> {
    if self.has_flag(windows::Win32::UI::Shell::FWF_NOICONS) {
      return None;
    }
    let mut pt = screen;
    unsafe {
      let _ = ScreenToClient(self.list, &mut pt);
      let mut cell = POINT::default();
      self.view.GetSpacing(&mut cell).ok()?;
      let n = self.view.ItemCount(SVGIO_ALLVIEW).ok()?;
      for i in 0..n {
        let Ok(pidl) = self.view.Item(i) else { continue };
        let at = self.view.GetItemPosition(pidl);
        CoTaskMemFree(Some(pidl as *const _));
        if let Ok(p) = at {
          if pt.x >= p.x && pt.x < p.x + cell.x && pt.y >= p.y && pt.y < p.y + cell.y {
            return Some(i);
          }
        }
      }
    }
    None
  }

  /// A right click on an icon selects it unless it is part of the
  /// selection already (as in Explorer).
  pub fn select_for_menu(&self, i: i32) {
    unsafe {
      let Ok(pidl) = self.view.Item(i) else { return };
      let state = self.view.GetSelectionState(pidl).unwrap_or(0);
      CoTaskMemFree(Some(pidl as *const _));
      if state & SVSI_SELECT.0 as u32 == 0 {
        let _ = self.view.SelectItem(i, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0) as u32);
      }
    }
  }

  pub fn selection(&self) -> Vec<Entry> {
    let mut out = Vec::new();
    unsafe {
      let Ok(items) = self.view.GetSelection(false) else { return out };
      let n = items.GetCount().unwrap_or(0);
      for k in 0..n {
        let Ok(item): windows::core::Result<IShellItem> = items.GetItemAt(k) else { continue };
        let Some(path) = item.GetDisplayName(SIGDN_FILESYSPATH).ok().and_then(pwstr) else { continue };
        let attrs = item.GetAttributes(windows::Win32::System::SystemServices::SFGAO_FLAGS(SFGAO_FOLDER | SFGAO_LINK)).map(|a| a.0).unwrap_or(0);
        let link = attrs & SFGAO_LINK != 0;
        let target = if link {
          item.cast::<IShellItem2>().ok().and_then(|i2| i2.GetString(&PKEY_LINK_TARGET).ok()).and_then(pwstr).filter(|t| !t.is_empty())
        } else {
          None
        };
        out.push(Entry { path, folder: attrs & SFGAO_FOLDER != 0 && !link, link, target });
      }
    }
    out
  }

  /// The focused (selected) icon's middle on the screen, for the menu key.
  pub fn focused_point(&self) -> Option<POINT> {
    unsafe {
      let i = self.view.GetFocusedItem().ok()?;
      let pidl = self.view.Item(i).ok()?;
      let state = self.view.GetSelectionState(pidl).unwrap_or(0);
      let at = self.view.GetItemPosition(pidl);
      CoTaskMemFree(Some(pidl as *const _));
      if state & SVSI_SELECT.0 as u32 == 0 {
        return None;
      }
      let mut p = at.ok()?;
      let mut cell = POINT::default();
      let _ = self.view.GetSpacing(&mut cell);
      p.x += cell.x / 2;
      p.y += cell.y / 2;
      let _ = ClientToScreen(self.list, &mut p);
      Some(p)
    }
  }

  /// A canonical verb ("open", "runas", "openas", "cut", "copy", "link",
  /// "delete", "properties") on the selection, run by Explorer.
  pub fn invoke(&self, verb: &str) {
    let mut v = verb.as_bytes().to_vec();
    v.push(0);
    unsafe {
      if let Err(err) = self.view.InvokeVerbOnSelection(PCSTR(v.as_ptr())) {
        tracing::warn!("Desktop: {}: {:?}", verb, err);
      }
    }
  }

  /// Explorer's rename box on the focused icon (it needs the keyboard).
  pub fn rename(&self) {
    unsafe {
      let _ = SetForegroundWindow(GetAncestor(self.list, GA_ROOT));
      let _ = self.view.DoRename();
    }
  }

  pub fn icon_size(&self) -> i32 {
    let (mut mode, mut size) = (FVM_ICON, 0);
    unsafe {
      let _ = self.view.GetViewModeAndIconSize(&mut mode, &mut size);
    }
    size
  }

  pub fn set_icon_size(&self, size: i32) {
    unsafe {
      let _ = self.view.SetViewModeAndIconSize(FVM_ICON, size);
    }
  }

  pub fn sorted_by(&self) -> Option<PROPERTYKEY> {
    let mut cols = [SORTCOLUMN::default()];
    unsafe { self.view.GetSortColumns(&mut cols) }.ok()?;
    Some(cols[0].propkey)
  }

  pub fn sort_by(&self, key: PROPERTYKEY) {
    unsafe {
      let _ = self.view.SetSortColumns(&[SORTCOLUMN { propkey: key, direction: SORT_ASCENDING }]);
    }
  }

  pub fn refresh(&self) {
    unsafe {
      let _ = self.shell.Refresh();
    }
  }

  fn dual(&self) -> Option<IShellFolderViewDual> {
    unsafe { self.shell.GetItemObject(SVGIO_BACKGROUND) }.ok()
  }

  fn folder(&self) -> Option<Folder> {
    unsafe { self.dual()?.Folder() }.ok()
  }

  /// Copies (or moves) files onto the desktop: Explorer's own copy, with its
  /// progress and its questions about names that exist.
  pub fn paste(&self, files: &[String], move_them: bool) {
    let Some(folder) = self.folder() else { return };
    for f in files {
      let item = VARIANT::from(BSTR::from(f.as_str()));
      let none = VARIANT::from(0i32);
      let done = unsafe { if move_them { folder.MoveHere(&item, &none) } else { folder.CopyHere(&item, &none) } };
      if let Err(err) = done {
        tracing::warn!("Desktop: paste {}: {:?}", f, err);
      }
    }
  }

  /// A new folder, made by Explorer (it gets the next free name itself).
  pub fn new_folder(&self, name: &str) -> bool {
    let Some(folder) = self.folder() else { return false };
    unsafe { folder.NewFolder(&BSTR::from(name), &VARIANT::from(0i32)) }.is_ok()
  }

  /// Starts a program through Explorer: unelevated, as from the Start menu
  /// (our process runs as administrator).
  pub fn run(&self, file: &str, args: &str, dir: &str) -> bool {
    let Some(dual) = self.dual() else { return false };
    let Some(shell) = unsafe { dual.Application() }.ok().and_then(|a| a.cast::<IShellDispatch2>().ok()) else {
      return false;
    };
    unsafe {
      shell
        .ShellExecute(
          &BSTR::from(file),
          &VARIANT::from(BSTR::from(args)),
          &VARIANT::from(BSTR::from(dir)),
          &VARIANT::from(BSTR::from("open")),
          &VARIANT::from(1i32),
        )
        .is_ok()
    }
  }
}

/// Windows' own "New" menu of the desktop (the handler Explorer shows): its
/// entries are what the installed apps registered (ShellNew), in Windows'
/// order and words, and it makes the chosen item itself, then starts its
/// rename in the desktop's view.
pub struct NewMenu {
  menu: IContextMenu3,
  hmenu: HMENU,
}

/// One entry of the "New" menu.
pub struct NewItem {
  pub id: u32,
  pub label: String,
  /// the handler's verb ("NewFolder" for a folder; empty for most files)
  pub verb: String,
}

/// The ids the handler numbers its entries from.
const NEW_FIRST: u32 = 1;
const NEW_LAST: u32 = 0x7FFF;

/// "&Klasör" -> "Klasör": a menu's mnemonic marks go, a doubled one is an "&".
fn without_mnemonics(raw: &str) -> String {
  let mut label = String::new();
  let mut chars = raw.chars().peekable();
  while let Some(c) = chars.next() {
    if c != '&' {
      label.push(c);
    } else if chars.peek() == Some(&'&') {
      chars.next();
      label.push('&');
    }
  }
  label
}

impl NewMenu {
  /// The handler for the desktop folder, with its entries; `desk` (Explorer's
  /// view) lets it place and rename the new item.
  pub fn open(desk: Option<&Desktop>) -> Option<(NewMenu, Vec<NewItem>)> {
    unsafe {
      let pidl = SHGetKnownFolderIDList(&FOLDERID_Desktop, 0, None).ok()?;
      let init: windows::core::Result<IShellExtInit> = CoCreateInstance(&CLSID_NewMenu, None, CLSCTX_INPROC_SERVER);
      let init = init.and_then(|i| i.Initialize(Some(pidl.cast_const()), None, HKEY::default()).map(|_| i));
      CoTaskMemFree(Some(pidl.cast_const().cast()));
      let init = init.ok()?;
      if let (Some(d), Ok(site)) = (desk, init.cast::<IObjectWithSite>()) {
        let _ = site.SetSite(&d.shell);
      }
      let menu: IContextMenu3 = init.cast().ok()?;
      let hmenu = CreatePopupMenu().ok()?;
      let nm = NewMenu { menu, hmenu };
      nm.menu.QueryContextMenu(hmenu, 0, NEW_FIRST, NEW_LAST, CMF_NORMAL).ok()?;
      // it adds one item, "New", and fills its submenu when that opens
      let sub = GetSubMenu(hmenu, 0);
      if sub.is_invalid() {
        return None;
      }
      let _ = nm.menu.HandleMenuMsg(WM_INITMENUPOPUP, WPARAM(sub.0 as usize), LPARAM(0));
      let items = (0..GetMenuItemCount(sub).max(0) as u32).filter_map(|i| nm.item(sub, i)).collect();
      Some((nm, items))
    }
  }

  fn item(&self, sub: HMENU, index: u32) -> Option<NewItem> {
    let mut text = [0u16; 260];
    let mut info = MENUITEMINFOW {
      cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
      fMask: MIIM_FTYPE | MIIM_ID | MIIM_STRING,
      dwTypeData: PWSTR(text.as_mut_ptr()),
      cch: text.len() as u32,
      ..Default::default()
    };
    unsafe { GetMenuItemInfoW(sub, index, true, &mut info) }.ok()?;
    if info.fType.0 & MFT_SEPARATOR.0 != 0 || info.wID < NEW_FIRST {
      return None;
    }
    let label = without_mnemonics(&String::from_utf16_lossy(&text[..(info.cch as usize).min(text.len())]));
    if label.trim().is_empty() {
      return None;
    }
    let mut verb = [0u16; 64];
    let got = unsafe {
      self.menu.GetCommandString((info.wID - NEW_FIRST) as usize, GCS_VERBW, None, PSTR(verb.as_mut_ptr().cast()), verb.len() as u32)
    };
    let end = verb.iter().position(|&c| c == 0).unwrap_or(0);
    let verb = if got.is_ok() { String::from_utf16_lossy(&verb[..end]) } else { String::new() };
    Some(NewItem { id: info.wID, label, verb })
  }

  /// Makes the entry: the handler creates it at `at` (screen) and starts
  /// its rename in the view.
  pub fn invoke(&self, id: u32, at: POINT, owner: HWND) {
    let info = CMINVOKECOMMANDINFOEX {
      cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
      fMask: CMIC_MASK_PTINVOKE,
      hwnd: owner,
      lpVerb: PCSTR((id - NEW_FIRST) as usize as *const u8),
      nShow: SW_SHOWNORMAL.0,
      ptInvoke: at,
      ..Default::default()
    };
    if let Err(err) = unsafe { self.menu.InvokeCommand(std::ptr::from_ref(&info).cast::<CMINVOKECOMMANDINFO>()) } {
      tracing::warn!("Desktop: new item: {:?}", err);
    }
  }
}

impl Drop for NewMenu {
  fn drop(&mut self) {
    unsafe {
      let _ = DestroyMenu(self.hmenu);
    }
  }
}

/// The "New" menu's entries as last read: (label, verb). Reading them takes
/// the handler 0.2-0.35 s, too long for a menu that opens on a click, so it
/// happens on a worker (at start, and after each use for the next one).
static NEW_ENTRIES: std::sync::Mutex<Option<Vec<(String, String)>>> = std::sync::Mutex::new(None);

/// COM for a worker thread, in the apartment the shell's handlers expect.
struct Sta;

impl Sta {
  fn new() -> Self {
    unsafe {
      let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    Sta
  }
}

impl Drop for Sta {
  fn drop(&mut self) {
    unsafe { windows::Win32::System::Com::CoUninitialize() };
  }
}

/// The entries as last read (None until the first read is done).
pub fn new_entries() -> Option<Vec<(String, String)>> {
  NEW_ENTRIES.lock().ok()?.clone()
}

/// Reads the entries again, on a worker.
pub fn refresh_new_entries() {
  std::thread::spawn(|| {
    let _com = Sta::new();
    if let Some((_, items)) = NewMenu::open(None) {
      if let Ok(mut entries) = NEW_ENTRIES.lock() {
        *entries = Some(items.into_iter().map(|i| (i.label, i.verb)).collect());
      }
    }
  });
}

/// Makes the entry (found by its verb and label) at `at` (screen), on a
/// worker: Explorer's view comes along, so the handler places the new item
/// there and starts its rename.
pub fn make_new(label: String, verb: String, at: POINT) {
  std::thread::spawn(move || {
    let _com = Sta::new();
    let desk = Desktop::open();
    let Some((menu, items)) = NewMenu::open(desk.as_ref()) else { return };
    let item = items.iter().find(|i| i.verb == verb && i.label == label).or_else(|| items.iter().find(|i| i.verb == verb));
    match item {
      Some(i) => menu.invoke(i.id, at, desk.as_ref().map(Desktop::window).unwrap_or_default()),
      None => tracing::warn!("Desktop: new item {:?} is gone from the New menu", label),
    }
  });
}

impl Desktop {
  /// Explorer's window for the desktop's icons (the owner of its dialogs).
  pub fn window(&self) -> HWND {
    self.list
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mnemonic_marks_go_and_a_doubled_one_stays() {
    for (raw, label) in [("&Klasör", "Klasör"), ("A && B", "A & B"), ("Metin &Belgesi", "Metin Belgesi"), ("&&&x", "&x")] {
      assert_eq!(without_mnemonics(raw), label);
    }
  }

  #[test]
  #[ignore = "times this machine's desktop menu steps (Explorer's view, the New menu handler)"]
  fn times_the_desktop_menu_steps() {
    unsafe {
      let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    for round in 0..3 {
      let t = std::time::Instant::now();
      let desk = Desktop::open();
      let open = t.elapsed();
      let t = std::time::Instant::now();
      let _ = desk.as_ref().map(|d| (d.item_at(POINT { x: 400, y: 400 }), d.icon_size(), d.flags(), d.sorted_by()));
      let view = t.elapsed();
      let t = std::time::Instant::now();
      let new = NewMenu::open(desk.as_ref()).map_or(0, |(_, items)| items.len());
      let new_menu = t.elapsed();
      println!("round {round}: Desktop::open {open:?}, view reads {view:?}, NewMenu::open {new_menu:?} ({new} items)");
    }
  }

  #[test]
  #[ignore = "reads this machine's New menu (Explorer's handler)"]
  fn lists_windows_new_menu() {
    unsafe {
      let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    let (_menu, items) = NewMenu::open(None).expect("New menu handler");
    for i in &items {
      println!("{} | {} | {}", i.id, i.label, i.verb);
    }
    assert!(items.len() >= 2, "a folder and a text document at least");
  }
}

/// The file's Properties window.
pub fn properties(path: &str) {
  unsafe {
    let _ = SHObjectProperties(None, SHOP_FILEPATH, &windows::core::HSTRING::from(path), PCWSTR::null());
  }
}
