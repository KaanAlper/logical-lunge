//! WEB desktop widget host. Integration (owned by main.rs):
//! `mod desktop_widgets;` register the six `desktop_widgets_*` commands below;
//! call `desktop_widgets::start(&app_handle)` once after factory creation.
//! Open transparent/unfocused/non-decorated `desktop-widgets` factory presets
//! on ALL monitors; keep the factory's monitor-change relaunch behavior.
//! Top-level windows are owned by Explorer's icon host, above the icon view;
//! an HRGN contains only cards/popups, leaving the remaining desktop untouched.
//! JS rectangles are CLIENT physical pixels, including its CSS zoom.
//! No widget is ever made topmost and maintenance never takes focus.

use std::{collections::HashMap, path::PathBuf, sync::{Mutex, OnceLock}, time::Duration};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopMonitor {
  pub device: String, pub primary: bool,
  pub x: i32, pub y: i32, pub width: i32, pub height: i32,
  pub scale: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Region { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }
#[derive(Clone)]
struct Host { device: String, regions: Vec<Region>, editing: bool, geometry: String, active: bool }
static HOSTS: OnceLock<Mutex<HashMap<String, Host>>> = OnceLock::new();
static STORE_LOCK: Mutex<()> = Mutex::new(());
fn hosts() -> &'static Mutex<HashMap<String, Host>> { HOSTS.get_or_init(|| Mutex::new(HashMap::new())) }
fn state_path() -> Result<PathBuf, String> {
  let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?;
  Ok(PathBuf::from(base).join("LogicalLunge/state/desktop-widgets.json"))
}
fn ui_scale() -> f64 {
  let prefs = std::env::var_os("USERPROFILE").map(PathBuf::from)
    .and_then(|p| std::fs::read_to_string(p.join(".config/logical-lunge/prefs.json")).ok())
    .and_then(|s| serde_json::from_str::<Value>(&s).ok());
  let n = prefs.as_ref().and_then(|p| p["uiScale"].as_u64()).unwrap_or(100);
  if [85, 90, 100, 110, 125, 150].contains(&n) { n as f64 / 100.0 } else { 1.0 }
}
fn sizes(kind: &str) -> Option<((f64, f64), (f64, f64))> {
  Some(match kind {
    "clock" => ((264., 120.), (136., 72.)), "media" => ((336., 112.), (240., 96.)),
    "system" => ((288., 128.), (200., 96.)), "weather" => ((248., 128.), (176., 96.)),
    "agenda" => ((264., 232.), (176., 136.)), "note" => ((248., 200.), (144., 96.)), _ => return None,
  })
}
fn number(v: &Value, k: &str, default: f64) -> f64 { v[k].as_f64().filter(|n| n.is_finite()).unwrap_or(default) }
fn normalize(v: &Value) -> Option<Value> {
  let id = v["id"].as_u64().filter(|n| *n > 0 && *n <= 9_007_199_254_740_991)?;
  let kind = v["kind"].as_str()?;
  let ((w, h), _) = sizes(kind)?;
  let clock = v["clock"].as_str().filter(|s| ["digital", "large", "analog"].contains(s)).unwrap_or("digital");
  Some(json!({"id": id, "kind": kind, "monitor": v["monitor"].as_str().unwrap_or(""),
    "x": number(v, "x", 24.), "y": number(v, "y", 24.),
    "w": if number(v,"w",w) > 0. { number(v,"w",w) } else { w },
    "h": if number(v,"h",h) > 0. { number(v,"h",h) } else { h },
    "clock": clock, "seconds": v["seconds"].as_bool().unwrap_or(false),
    "date": v["date"].as_bool().unwrap_or(true), "temps": v["temps"].as_bool().unwrap_or(true),
    "city": v["city"].as_str().unwrap_or(""), "fahrenheit": v["fahrenheit"].as_bool().unwrap_or(false),
    "note": v["note"].as_str().unwrap_or("")}))
}
fn parse_store(text: &str) -> Result<Value, String> {
  let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
  if v["version"].as_u64().unwrap_or(1) > 1 { return Err("Widget layout is from a newer version".into()); }
  let widgets = v["widgets"].as_array().ok_or("Unreadable widget layout")?;
  let mut seen = std::collections::HashSet::new();
  let widgets: Vec<Value> = widgets.iter().filter_map(normalize).filter(|w| seen.insert(w["id"].as_u64().unwrap())).collect();
  Ok(json!({"version":1, "revision":v["revision"].as_u64().unwrap_or(0), "widgets":widgets}))
}
fn read_store() -> Result<Value, String> {
  let path = state_path()?;
  match std::fs::read_to_string(&path) {
    Ok(text) => match parse_store(&text) {
      Ok(store) => Ok(store),
      Err(e) => {
        // Future schemas must not be downgraded. Corrupt layouts are retained.
        if e.contains("newer version") { return Err(e); }
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
        std::fs::rename(&path, path.with_extension(format!("json.bad.{stamp}"))).map_err(|err| err.to_string())?;
        tracing::warn!("Desktop widgets: {e}; layout retained as .json.bad.{stamp}");
        Ok(json!({"version":1,"revision":0,"widgets":[],"warning":"Unreadable layout retained as .json.bad"}))
      }
    },
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({"version":1,"revision":0,"widgets":[]})),
    Err(e) => Err(e.to_string()),
  }
}
fn write_store(store: &Value) -> Result<(), String> {
  let path = state_path()?;
  std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
  let tmp = path.with_extension("json.tmp");
  let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
  use std::io::Write;
  file.write_all(serde_json::to_string_pretty(store).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
  file.sync_all().map_err(|e| e.to_string())?;
  drop(file);
  // Replace existing files atomically on Windows; std::fs::rename cannot.
  #[cfg(windows)] unsafe {
    use std::os::windows::ffi::OsStrExt;
    #[link(name="kernel32")] extern "system" { fn MoveFileExW(from:*const u16,to:*const u16,flags:u32)->i32; }
    let from:Vec<u16>=tmp.as_os_str().encode_wide().chain(Some(0)).collect();
    let to:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();
    if MoveFileExW(from.as_ptr(),to.as_ptr(),1|8)==0 { return Err(std::io::Error::last_os_error().to_string()); }
  }
  #[cfg(not(windows))] std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
  Ok(())
}
fn monitor_for<'a>(monitors: &'a [DesktopMonitor], device: &str) -> Option<&'a DesktopMonitor> {
  monitors.iter().find(|m| !device.is_empty() && m.device.eq_ignore_ascii_case(device))
    .or_else(|| monitors.iter().find(|m| m.primary)).or(monitors.first())
}
fn clamp(spec: &mut Value, mon: &DesktopMonitor) {
  let (_, (mw, mh)) = sizes(spec["kind"].as_str().unwrap()).unwrap();
  let (aw, ah) = (mon.width as f64 / mon.scale, mon.height as f64 / mon.scale);
  let w = number(spec,"w",mw).max(mw).min(aw); let h = number(spec,"h",mh).max(mh).min(ah);
  let x = number(spec,"x",0.).min(aw-w).max(0.); let y = number(spec,"y",0.).min(ah-h).max(0.);
  spec["x"] = json!(x); spec["y"] = json!(y); spec["w"] = json!(w); spec["h"] = json!(h);
}
fn overlaps(a: &Value, b: &Value, gap: f64) -> bool {
  let (x,y,w,h) = (number(a,"x",0.),number(a,"y",0.),number(a,"w",0.),number(a,"h",0.));
  let (bx,by,bw,bh) = (number(b,"x",0.),number(b,"y",0.),number(b,"w",0.),number(b,"h",0.));
  x-gap < bx+bw && bx < x+w+gap && y-gap < by+bh && by < y+h+gap
}
fn apply_operation(mut store: Value, op: &Value, mons: &[DesktopMonitor]) -> Result<Value,String> {
  let widgets = store["widgets"].as_array_mut().ok_or("Invalid store")?;
  match op["action"].as_str().unwrap_or("") {
    "add" => {
      let wire_kind = op["kind"].as_str().ok_or("Unknown widget kind")?;
      let kind=if wire_kind=="notes" { "note" } else { wire_kind };
      if sizes(kind).is_none() { return Err("Unknown widget kind".into()); }
      let m = monitor_for(mons, op["monitor"].as_str().unwrap_or("")).ok_or("No monitor")?;
      let id = widgets.iter().filter_map(|w| w["id"].as_u64()).max().unwrap_or(0) + 1;
      let mut s = normalize(&json!({"id":id,"kind":kind,"monitor":m.device})).ok_or("Invalid widget")?;
      clamp(&mut s, m);
      let right = ((m.width as f64/m.scale-24.-number(&s,"w",0.)).max(0.)/8.).round()*8.;
      let right = right.min((m.width as f64/m.scale-number(&s,"w",0.)).max(0.));
      let top = 24f64.min((m.height as f64/m.scale-number(&s,"h",0.)).max(0.));
      let mut x = right; let mut spot = None;
      while x >= 0. && spot.is_none() {
        let mut y = top;
        while y + number(&s,"h",0.) <= m.height as f64/m.scale {
          s["x"] = json!(x); s["y"] = json!(y);
          let occupied = widgets.iter().any(|w| {
            if monitor_for(mons,w["monitor"].as_str().unwrap_or("")).map(|n| &n.device) != Some(&m.device) { return false; }
            let mut projected = w.clone(); clamp(&mut projected,m); overlaps(&s,&projected,8.)
          });
          if !occupied { spot = Some((x,y)); break; }
          y += 16.;
        }
        x -= 32.;
      }
      let (x,y) = spot.unwrap_or((right,top)); s["x"] = json!(x); s["y"] = json!(y); widgets.push(s);
    },
    "remove" => widgets.retain(|w| w["id"] != op["id"]),
    "patch" => {
      let s = widgets.iter_mut().find(|w| w["id"] == op["id"]).ok_or("Widget removed")?;
      let p = op["patch"].as_object().ok_or("Invalid patch")?;
      for (k,v) in p { if !["id","kind"].contains(&k.as_str()) && s.get(k).is_some() { s[k] = v.clone(); } }
      *s = normalize(s).ok_or("Invalid patch")?;
      let device=s["monitor"].as_str().unwrap_or("");
      if let Some(m) = mons.iter().find(|m| m.device.eq_ignore_ascii_case(device) || (device.is_empty() && m.primary)) { clamp(s,m); }
    },
    _ => return Err("Unknown widget operation".into()),
  }
  store["revision"] = json!(store["revision"].as_u64().unwrap_or(0)+1);
  if let Some(o) = store.as_object_mut() { o.remove("warning"); }
  Ok(store)
}

#[tauri::command]
pub fn desktop_widgets_load() -> Result<Value, String> {
  let _lock = STORE_LOCK.lock().map_err(|e| e.to_string())?;
  read_store()
}
#[tauri::command]
pub fn desktop_widgets_update(app: AppHandle, operation: Value) -> Result<Value, String> {
  let _lock = STORE_LOCK.lock().map_err(|e| e.to_string())?;
  let store = apply_operation(read_store()?, &operation, &platform::monitors(ui_scale()))?;
  write_store(&store)?;
  app.emit("ll:desktop-widgets-store", &store).map_err(|e| e.to_string())?;
  Ok(store)
}
#[tauri::command]
pub fn desktop_widgets_bootstrap(window: WebviewWindow) -> Result<Value, String> {
  window.set_title("Logical Lunge · widget").map_err(|e| e.to_string())?;
  let mons = platform::monitors(ui_scale());
  let position = window.outer_position().map_err(|e| e.to_string())?;
  let m = mons.iter().find(|m| position.x >= m.x && position.x < m.x+m.width && position.y >= m.y-128 && position.y < m.y+m.height)
    .or_else(|| mons.iter().min_by_key(|m| (position.x as i64-m.x as i64).pow(2)+(position.y as i64-m.y as i64).pow(2)))
    .ok_or("No desktop monitor")?;
  // Empty region FIRST: a loading host never blocks the desktop.
  platform::set_regions(&window, &[])?;
  platform::attach(&window,m,false)?;
  let geometry = serde_json::to_string(m).map_err(|e| e.to_string())?;
  hosts().lock().map_err(|e| e.to_string())?.insert(window.label().into(), Host {
    device:m.device.clone(),regions:Vec::new(),editing:false,geometry,active:true,
  });
  Ok(json!({"monitor":m.device,"monitors":mons,"uiScale":ui_scale(),"store":desktop_widgets_load()?}))
}
#[tauri::command]
pub fn desktop_widgets_regions(window: WebviewWindow, regions: Vec<Region>) -> Result<(),String> {
  if regions.len() > 512 || regions.iter().any(|r| ![r.x,r.y,r.w,r.h].iter().all(|n| n.is_finite()) || r.w < 0. || r.h < 0.) { return Err("Invalid desktop regions".into()); }
  let mut map = hosts().lock().map_err(|e| e.to_string())?;
  let host = map.get_mut(window.label()).ok_or("Desktop host not initialized")?;
  platform::set_regions(&window,&regions)?; host.regions = regions;
  Ok(())
}
#[tauri::command]
pub fn desktop_widgets_editing(window: WebviewWindow, editing: bool) -> Result<(),String> {
  let mut map = hosts().lock().map_err(|e| e.to_string())?;
  let host = map.get_mut(window.label()).ok_or("Desktop host not initialized")?;
  platform::editing(&window, editing)?; host.editing = editing;
  Ok(())
}
#[tauri::command]
pub fn desktop_widgets_cursor() -> Result<Value,String> { platform::cursor() }

/// Retain this handle and abort it on shutdown. Factory owns creation/closure,
/// so this task only repairs parenting and signals work-area/DPI changes.
pub fn start(app: &AppHandle) -> tokio::task::JoinHandle<()> {
  let app = app.clone();
  tokio::spawn(async move {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
      tick.tick().await;
      let app = app.clone();
      let dispatch = app.clone();
      let _ = dispatch.run_on_main_thread(move || {
        let mons = platform::monitors(ui_scale());
        if mons.is_empty() { return; }
        let Ok(mut map) = hosts().lock() else { return; };
        map.retain(|label, host| {
          let Some(win) = app.get_webview_window(label) else { return false; };
          let Some(m) = mons.iter().find(|m| m.device == host.device) else { return true; };
          let geometry = serde_json::to_string(m).unwrap_or_default();
          let active = !platform::paused(m);
          let changed = geometry != host.geometry || active != host.active;
          let repaired = platform::needs_attach(&win);
          if geometry != host.geometry || repaired {
            if let Err(e) = platform::attach(&win,m,host.editing) { tracing::warn!("Desktop parenting: {e}"); }
            let _ = platform::set_regions(&win,&host.regions);
          }
          if changed || repaired {
            let _ = win.emit("ll:desktop-widgets-host",json!({"monitor":host.device,"monitors":mons,"uiScale":ui_scale(),"active":active}));
          }
          host.geometry = geometry; host.active = active;
          true
        });
      });
    }
  })
}

#[cfg(windows)]
mod platform {
  use super::*;
  use windows::{core::{w, PCWSTR}, Win32::{
    Foundation::{BOOL, HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::*, UI::{HiDpi::{GetDpiForMonitor,MDT_EFFECTIVE_DPI}, WindowsAndMessaging::*},
  }};
  pub fn monitors(scale: f64) -> Vec<DesktopMonitor> {
    unsafe extern "system" fn each(m: HMONITOR,_:HDC,_:*mut RECT,lp:LPARAM)->BOOL {
      let list = &mut *(lp.0 as *mut Vec<DesktopMonitor>);
      let mut info = MONITORINFOEXW::default(); info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
      if GetMonitorInfoW(m,&mut info as *mut _ as *mut _).as_bool() {
        let (mut x,mut y) = (96,96); let _ = GetDpiForMonitor(m,MDT_EFFECTIVE_DPI,&mut x,&mut y);
        let end = info.szDevice.iter().position(|c| *c == 0).unwrap_or(info.szDevice.len());
        let r = info.monitorInfo.rcWork;
        list.push(DesktopMonitor { device:String::from_utf16_lossy(&info.szDevice[..end]),primary:info.monitorInfo.dwFlags & 1 != 0,
          x:r.left,y:r.top,width:r.right-r.left,height:r.bottom-r.top,scale:x.max(1) as f64/96. });
      }
      BOOL(1)
    }
    let mut list: Vec<DesktopMonitor> = Vec::new();
    unsafe { let _ = EnumDisplayMonitors(None,None,Some(each),LPARAM(&mut list as *mut _ as isize)); }
    for m in &mut list { m.scale *= scale; }
    list
  }
  fn hwnd(win: &WebviewWindow)->Result<HWND,String> { win.hwnd().map(|h| HWND(h.0)).map_err(|e|e.to_string()) }
  fn icons_host()->HWND {
    unsafe extern "system" fn each(h:HWND,lp:LPARAM)->BOOL {
      if IsWindowVisible(h).as_bool() && FindWindowExW(h,None,w!("SHELLDLL_DefView"),PCWSTR::null()).is_ok_and(|w| !w.is_invalid()) {
        *(lp.0 as *mut HWND) = h; return BOOL(0);
      } BOOL(1)
    }
    let mut host = HWND::default(); unsafe { let _ = EnumWindows(Some(each),LPARAM(&mut host as *mut _ as isize)); } host
  }
  pub fn needs_attach(win:&WebviewWindow)->bool {
    unsafe { hwnd(win).is_ok_and(|h| GetWindowLongPtrW(h,GWLP_HWNDPARENT) != icons_host().0 as isize || !layer_ok(h)) }
  }
  fn layer_ok(h:HWND)->bool {
    unsafe {
      let desktop=icons_host(); if desktop.is_invalid() { return false; }
      let mut next=GetWindow(desktop,GW_HWNDPREV).unwrap_or_default();
      // Hidden/cloaked windows and our sibling hosts do not cover the cards.
      while !next.is_invalid() {
        if next==h { return true; }
        let mut title=[0u16;80]; let n=GetWindowTextW(next,&mut title).max(0) as usize;
        let mut cloaked=0u32;
        let _=windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(next,windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,&mut cloaked as *mut _ as *mut _,4);
        if IsWindowVisible(next).as_bool() && cloaked==0 && String::from_utf16_lossy(&title[..n])!="Logical Lunge · widget" { return false; }
        next=GetWindow(next,GW_HWNDPREV).unwrap_or_default();
      }
      false
    }
  }
  pub fn attach(win:&WebviewWindow,mon:&DesktopMonitor,edit:bool)->Result<(),String> {
    unsafe {
      let h = hwnd(win)?; let parent = icons_host();
      if parent.is_invalid() { return Err("Explorer desktop unavailable; host remains empty".into()); }
      // A genuine desktop owner preserves top-level DWM thumbnail eligibility.
      // WS_CHILD/SetParent would remove it from EnumWindows and from transitions.
      let previous = GetWindowLongPtrW(h,GWL_STYLE);
      SetWindowLongPtrW(h,GWL_STYLE,(previous & !(WS_CHILD.0 as isize)) | WS_POPUP.0 as isize);
      SetWindowLongPtrW(h,GWLP_HWNDPARENT,parent.0 as isize);
      let old_ex = GetWindowLongPtrW(h,GWL_EXSTYLE);
      let ex=old_ex;
      let ex=(ex & !((WS_EX_APPWINDOW | WS_EX_TOPMOST).0 as isize)) | WS_EX_TOOLWINDOW.0 as isize;
      SetWindowLongPtrW(h,GWL_EXSTYLE,if edit { ex & !(WS_EX_NOACTIVATE.0 as isize) } else { ex | WS_EX_NOACTIVATE.0 as isize });
      let flags=SWP_NOACTIVATE|SWP_NOOWNERZORDER;
      if old_ex & WS_EX_TOPMOST.0 as isize != 0 { let _=SetWindowPos(h,HWND_NOTOPMOST,0,0,0,0,flags|SWP_NOMOVE|SWP_NOSIZE); }
      let above=GetWindow(parent,GW_HWNDPREV).unwrap_or_default();
      let after=if above.is_invalid() || GetWindowLongPtrW(above,GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0 { HWND_TOP } else { above };
      SetWindowPos(h,after,mon.x,mon.y,mon.width,mon.height,flags|SWP_FRAMECHANGED|SWP_SHOWWINDOW|if above==h { SWP_NOZORDER } else { SET_WINDOW_POS_FLAGS(0) }).map_err(|e|e.to_string())?;
      Ok(())
    }
  }
  pub fn editing(win:&WebviewWindow,edit:bool)->Result<(),String> {
    unsafe {
      let h = hwnd(win)?; let ex = GetWindowLongPtrW(h,GWL_EXSTYLE);
      SetWindowLongPtrW(h,GWL_EXSTYLE,if edit { ex & !(WS_EX_NOACTIVATE.0 as isize) } else { ex | WS_EX_NOACTIVATE.0 as isize });
    }
    // Only an explicit click/tab in a card requests keyboard focus.
    if edit { win.set_focus().map_err(|e|e.to_string())?; }
    Ok(())
  }
  pub fn set_regions(win:&WebviewWindow,regions:&[Region])->Result<(),String> {
    unsafe {
      let all = CreateRectRgn(0,0,0,0);
      if all.is_invalid() { return Err("Region allocation failed".into()); }
      for r in regions {
        let part = CreateRectRgn(r.x.floor() as i32,r.y.floor() as i32,(r.x+r.w).ceil() as i32,(r.y+r.h).ceil() as i32);
        if part.is_invalid() { let _=DeleteObject(all); return Err("Region allocation failed".into()); }
        let result = CombineRgn(all,all,part,RGN_OR); let _ = DeleteObject(part);
        if result == RGN_ERROR { let _ = DeleteObject(all); return Err("Region union failed".into()); }
      }
      let h = match hwnd(win) { Ok(h)=>h,Err(e)=>{ let _=DeleteObject(all); return Err(e); } };
      if SetWindowRgn(h,all,true) == 0 { let _ = DeleteObject(all); return Err("Desktop region rejected".into()); }
      // Windows owns `all` after successful SetWindowRgn.
      Ok(())
    }
  }
  pub fn cursor()->Result<Value,String> {
    let mut p = POINT::default(); unsafe { GetCursorPos(&mut p).map_err(|e|e.to_string())?; } Ok(json!({"x":p.x,"y":p.y}))
  }
  pub fn paused(m:&DesktopMonitor)->bool {
    // Desktop-owned HWNDs stay attached to Explorer. Pause drawing
    // behind fullscreen and on secure desktops without hiding/re-showing HWNDs.
    #[link(name="user32")] extern "system" {
      fn OpenInputDesktop(flags:u32,inherit:i32,access:u32)->*mut std::ffi::c_void;
      fn CloseDesktop(handle:*mut std::ffi::c_void)->i32;
    }
    unsafe {
      let desktop = OpenInputDesktop(0,0,1);
      if desktop.is_null() { return true; } CloseDesktop(desktop);
      let fg = GetForegroundWindow(); if fg.is_invalid() { return false; }
      let mut class = [0u16;64]; let n = GetClassNameW(fg,&mut class).max(0) as usize;
      if ["Progman","WorkerW","Shell_TrayWnd"].contains(&String::from_utf16_lossy(&class[..n]).as_str()) { return false; }
      let mut r = RECT::default(); if GetWindowRect(fg,&mut r).is_err() { return false; }
      let monitor = MonitorFromWindow(fg,MONITOR_DEFAULTTONULL); let mut info = MONITORINFO::default(); info.cbSize=std::mem::size_of::<MONITORINFO>() as u32;
      if !GetMonitorInfoW(monitor,&mut info).as_bool() { return false; }
      let full = info.rcMonitor;
      r.left<=full.left && r.top<=full.top && r.right>=full.right && r.bottom>=full.bottom && m.x>=full.left && m.y>=full.top && m.x+m.width<=full.right && m.y+m.height<=full.bottom
    }
  }
}
#[cfg(not(windows))]
mod platform {
  use super::*;
  pub fn monitors(_:f64)->Vec<DesktopMonitor> { Vec::new() }
  pub fn attach(_: &WebviewWindow,_:&DesktopMonitor,_:bool)->Result<(),String> { Err("Windows desktop required".into()) }
  pub fn needs_attach(_: &WebviewWindow)->bool { false }
  pub fn set_regions(_: &WebviewWindow,_:&[Region])->Result<(),String> { Err("Windows desktop required".into()) }
  pub fn editing(_: &WebviewWindow,_:bool)->Result<(),String> { Err("Windows desktop required".into()) }
  pub fn cursor()->Result<Value,String> { Err("Windows desktop required".into()) }
  pub fn paused(_:&DesktopMonitor)->bool { false }
}

#[cfg(test)] mod tests {
  use super::*;
  fn monitors()->Vec<DesktopMonitor> { vec![DesktopMonitor{device:"DISPLAY1".into(),primary:true,x:0,y:40,width:1920,height:1040,scale:1.}] }
  #[test] fn persistence_and_partial_updates() {
    let mut s = json!({"version":1,"widgets":[]});
    s=apply_operation(s,&json!({"action":"add","kind":"note"}),&monitors()).unwrap();
    s=apply_operation(s,&json!({"action":"add","kind":"clock"}),&monitors()).unwrap();
    assert!(!overlaps(&s["widgets"][0],&s["widgets"][1],0.));
    s=apply_operation(s,&json!({"action":"patch","id":1,"patch":{"note":"süt\n📝","id":99,"kind":"clock"}}),&monitors()).unwrap();
    assert_eq!(s["widgets"][0]["id"],1); assert_eq!(s["widgets"][0]["kind"],"note");
    assert_eq!(parse_store(&s.to_string()).unwrap(),s);
  }
  #[test] fn corrupt_and_future_layouts_are_rejected() {
    assert!(parse_store("bad").is_err()); assert!(parse_store(r#"{"version":3,"widgets":[]}"#).is_err());
    let s=parse_store(r#"{"widgets":[{"id":1,"kind":"clock"},{"id":1,"kind":"note"},{"id":0,"kind":"media"}]}"#).unwrap();
    assert_eq!(s["widgets"].as_array().unwrap().len(),1);
  }
}
