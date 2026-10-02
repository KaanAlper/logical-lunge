//! Explorer's own view of the desktop, driven from our menus. The desktop
//! window's IShellView and IFolderView2 come from Explorer through
//! IShellWindows (SWC_DESKTOP), so every command runs where Windows runs it:
//! in Explorer, unelevated, with its undo, its progress and its rename box.
//! Our process (elevated) only asks; nothing here imitates Explorer.

use windows::{
  core::{w, Interface, BSTR, GUID, PCSTR, PCWSTR, PWSTR, VARIANT},
  Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::{ClientToScreen, ScreenToClient},
    System::Com::{CoCreateInstance, CoTaskMemFree, IDispatch, IServiceProvider, CLSCTX_ALL},
    UI::{
      Shell::{
        Folder, IFolderView2, IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellItem, IShellItem2,
        IShellView, IShellWindows, PropertiesSystem::PROPERTYKEY, SHObjectProperties, ShellWindows, FOLDERFLAGS,
        FVM_ICON, SHOP_FILEPATH, SIGDN_FILESYSPATH, SID_STopLevelBrowser, SORTCOLUMN, SORT_ASCENDING,
        SVGIO_ALLVIEW, SVGIO_BACKGROUND, SVSI_DESELECTOTHERS, SVSI_FOCUSED, SVSI_SELECT, SWC_DESKTOP,
        SWFO_NEEDDISPATCH,
      },
      WindowsAndMessaging::{FindWindowExW, GetAncestor, SetForegroundWindow, GA_ROOT},
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

/// The file's Properties window.
pub fn properties(path: &str) {
  unsafe {
    let _ = SHObjectProperties(None, SHOP_FILEPATH, &windows::core::HSTRING::from(path), PCWSTR::null());
  }
}
