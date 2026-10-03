//! File search over Everything's local Unicode IPC (Everything 1.4.1+).
//! Search integration inspired by srwi/EverythingToolbar; the IPC implementation
//! here follows voidtools' published protocol rather than bundling its SDK DLL.
//! No index is kept by the shell and no SDK DLL is bundled.

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
  pub name: String,
  pub path: String,
  pub full_path: String,
  pub is_dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePage {
  pub hits: Vec<FileHit>,
  pub total: u32,
  pub offset: u32,
}

/// Window class of Everything's IPC window. The unnamed instance uses it as
/// is; a named instance (Everything 1.5 runs as "1.5a" by default, any other
/// via `-instance <name>`) appends "_(<name>)". Every instance answers the
/// same protocol, so any of them will do.
#[cfg_attr(not(windows), allow(dead_code))]
const IPC_CLASS: &str = "EVERYTHING_TASKBAR_NOTIFICATION";

#[cfg_attr(not(windows), allow(dead_code))]
fn is_ipc_class(name: &str) -> bool {
  name == IPC_CLASS
    || name
      .strip_prefix(IPC_CLASS)
      .is_some_and(|rest| rest.starts_with("_(") && rest.ends_with(')'))
}

/// The program of a command line as Windows stores it in a sign-in entry, an
/// App Paths key or an uninstall icon: `"C:\…\Everything.exe" -startup`,
/// `C:\…\Everything.exe,0`.
#[cfg_attr(not(windows), allow(dead_code))]
fn exe_of(command: &str) -> Option<std::path::PathBuf> {
  let command = command.trim();
  let path = match command.strip_prefix('"') {
    Some(rest) => rest.split('"').next()?,
    None => &command[..command.to_ascii_lowercase().find(".exe")? + 4],
  };
  (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

#[cfg(windows)]
mod windows_ipc {
  use super::{exe_of, is_ipc_class, FileHit, FilePage};
  use crate::common::windows::read_reg_string;
  use std::{
    cell::RefCell,
    ffi::c_void,
    mem::size_of,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
  };
  use windows::{
    core::w,
    Win32::{
      Foundation::{BOOL, FALSE, HWND, LPARAM, LRESULT, TRUE, WPARAM},
      System::{
        DataExchange::COPYDATASTRUCT,
        LibraryLoader::GetModuleHandleW,
        Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
      },
      UI::WindowsAndMessaging::{
        ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, FindWindowW,
        GetClassNameW, GetMessageW, GetWindowLongPtrW, PostMessageW, RegisterClassW, SendMessageTimeoutW, SetTimer,
        SetWindowLongPtrW, GWLP_USERDATA, MSG, MSGFLT_ALLOW,
        SMTO_ABORTIFHUNG, WINDOW_EX_STYLE, WINDOW_STYLE,
        WNDCLASSW, WM_APP, WM_COPYDATA, WM_TIMER,
      },
    },
  };

  /// IPC window of any running Everything instance: the unnamed one first,
  /// otherwise the first top-level window whose class names an instance.
  fn ipc_window() -> Option<HWND> {
    if let Ok(hwnd) = unsafe { FindWindowW(w!("EVERYTHING_TASKBAR_NOTIFICATION"), None) } {
      return Some(hwnd);
    }
    let mut found = HWND::default();
    // EnumWindows reports an error when the callback stops it early; that is the success case here.
    let _ = unsafe { EnumWindows(Some(match_ipc_window), LPARAM(&mut found as *mut HWND as isize)) };
    (!found.is_invalid()).then_some(found)
  }

  fn unix_secs() -> u64 {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_secs())
      .unwrap_or(0)
  }

  /// Why the file search cannot reach Everything, in the shell log; at most
  /// once a minute (every keystroke searches again).
  fn log_once(why: &str) {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = unix_secs();
    if now.saturating_sub(LAST.load(Ordering::Relaxed)) >= 60 {
      LAST.store(now, Ordering::Relaxed);
      tracing::warn!("Everything: {}", why);
    }
  }

  /// The IPC window, after starting an installed Everything when none runs
  /// (the user quit it, or the installer only just put it in place). The
  /// shell runs unelevated, so Everything started from here does too, which
  /// its IPC needs: Windows drops messages from the shell to an elevated
  /// window.
  fn ipc_window_or_start() -> Result<HWND, String> {
    if let Some(hwnd) = ipc_window() {
      return Ok(hwnd);
    }
    let Some(exe) = installed_exe() else {
      log_once("no running instance and no installation found");
      return Err("Everything kurulu değil".into());
    };
    start_installed(&exe)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
      std::thread::sleep(Duration::from_millis(100));
      if let Some(hwnd) = ipc_window() {
        return Ok(hwnd);
      }
    }
    log_once(&format!("started {} but no IPC window appeared", exe.display()));
    Err("Everything açılamadı".into())
  }

  /// At most once every 10 s however many searches ask; within that window
  /// the caller just waits for the IPC window of the start under way.
  fn start_installed(exe: &Path) -> Result<(), String> {
    static LAST_START: AtomicU64 = AtomicU64::new(0);
    let now = unix_secs();
    if now.saturating_sub(LAST_START.load(Ordering::Relaxed)) < 10 {
      return Ok(());
    }
    LAST_START.store(now, Ordering::Relaxed);
    // No inherited stdio: a long-lived Everything must not hold the shell's
    // output pipes open after the shell itself exits.
    Command::new(exe)
      .arg("-startup")
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
      .map(|_| ())
      .map_err(|err| {
        log_once(&format!("could not start {}: {}", exe.display(), err));
        "Everything başlatılamadı".to_string()
      })
  }

  /// Where an installed Everything is: our tools folder, what Windows knows
  /// about it (sign-in entry, App Paths, uninstall entry), its standard
  /// folders, then the PATH (package managers put shims there).
  fn installed_exe() -> Option<PathBuf> {
    let own = std::env::current_exe().ok().and_then(|exe| {
      exe.parent().map(|dir| dir.join(r"tools\everything\Everything.exe"))
    });
    let standard = ["ProgramFiles", "ProgramFiles(x86)"]
      .into_iter()
      .filter_map(std::env::var_os)
      .map(|dir| PathBuf::from(dir).join(r"Everything\Everything.exe"));
    let on_path = std::env::var_os("PATH")
      .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
      .unwrap_or_default()
      .into_iter()
      .map(|dir| dir.join("everything.exe"));
    own
      .into_iter()
      .chain(registered_exes())
      .chain(standard)
      .chain(on_path)
      .find(|exe| exe.is_file())
  }

  fn registered_exes() -> Vec<PathBuf> {
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APP: &str =
      r"Software\Microsoft\Windows\CurrentVersion\App Paths\Everything.exe";
    const UNINSTALL: &str =
      r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Everything";
    [
      (HKEY_CURRENT_USER, RUN, "Everything"),
      (HKEY_LOCAL_MACHINE, RUN, "Everything"),
      (HKEY_CURRENT_USER, APP, ""),
      (HKEY_LOCAL_MACHINE, APP, ""),
      (HKEY_LOCAL_MACHINE, UNINSTALL, "DisplayIcon"),
    ]
    .into_iter()
    .filter_map(|(root, path, name)| read_reg_string(root, path, name).ok().flatten())
    .filter_map(|command| exe_of(&command))
    .collect()
  }

  unsafe extern "system" fn match_ipc_window(hwnd: HWND, found: LPARAM) -> BOOL {
    let mut name = [0u16; 128];
    let len = GetClassNameW(hwnd, &mut name).max(0) as usize;
    if is_ipc_class(&String::from_utf16_lossy(&name[..len])) {
      *(found.0 as *mut HWND) = hwnd;
      return FALSE;
    }
    TRUE
  }

  const QUERY2_UNICODE: usize = 18;
  const REPLY_ID: usize = 0x4c4c_4546;
  const REQUEST_NAME_PATH: u32 = 3;
  const SORT_NAME: u32 = 1;
  const WAIT_MS: u32 = 1500;
  const LATE_MS: u32 = 4000;

  // Win32 calls window_proc reentrantly while send_and_wait is reading this
  // state. It must have interior mutability: an outstanding &mut Reply made
  // the callback's raw-pointer write undefined, and optimized builds kept
  // seeing None even after Everything had answered. Only the owning window
  // thread accesses the cell, and no borrow is held over a Win32 call.
  struct Reply { result: RefCell<Option<Result<FilePage, String>>> }

  pub fn query_page(search: &str, limit: u32, offset: u32) -> Result<FilePage, String> {
    let search = search.trim();
    if search.is_empty() { return Ok(FilePage { hits: Vec::new(), total: 0, offset }); }
    if search.encode_utf16().count() > 512 || search.contains('\0') {
      return Err("Arama çok uzun veya geçersiz".into());
    }
    query_window(ipc_window_or_start()?, search, limit, offset)
  }

  fn query_window(everything: HWND, search: &str, limit: u32, offset: u32) -> Result<FilePage, String> {
    let hinst = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
    let class = w!("LogicalLungeEverythingIPC");
    unsafe {
      RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: hinst.into(),
        lpszClassName: class,
        ..Default::default()
      });
    }
    let hwnd = unsafe {
      CreateWindowExW(WINDOW_EX_STYLE::default(), class, w!(""), WINDOW_STYLE::default(),
        0, 0, 0, 0, None, None, hinst, None)
    }.map_err(|e| e.to_string())?;
    // Permit answers across integrity levels if the shell is ever elevated.
    unsafe {
      let _ = ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None);
    }
    let reply = Reply { result: RefCell::new(None) };
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, &reply as *const _ as isize); }
    let outcome = send_and_wait(everything, hwnd, search, limit.clamp(1, 40), offset, &reply);
    unsafe {
      SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
      let _ = DestroyWindow(hwnd);
    }
    outcome
  }

  fn query_bytes(reply_hwnd: u32, search: &str, limit: u32, offset: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(28 + (search.len() + 1) * 2);
    for field in [reply_hwnd, REPLY_ID as u32, 0, offset, limit.clamp(1, 40), REQUEST_NAME_PATH, SORT_NAME] {
      bytes.extend_from_slice(&field.to_le_bytes());
    }
    for ch in search.encode_utf16().chain(std::iter::once(0)) { bytes.extend_from_slice(&ch.to_le_bytes()); }
    bytes
  }

  fn send_and_wait(everything: HWND, hwnd: HWND, search: &str, limit: u32, offset: u32, reply: &Reply) -> Result<FilePage, String> {
    let mut bytes = query_bytes(hwnd.0 as usize as u32, search, limit, offset);
    let cds = COPYDATASTRUCT {
      dwData: QUERY2_UNICODE,
      cbData: bytes.len() as u32,
      lpData: bytes.as_mut_ptr().cast::<c_void>(),
    };
    let mut accepted = 0usize;
    let sent = unsafe {
      SendMessageTimeoutW(everything, WM_COPYDATA, WPARAM(hwnd.0 as usize),
        LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG, WAIT_MS, Some(&mut accepted))
    };
    if sent.0 == 0 || accepted == 0 {
      tracing::warn!("Everything: {:?} was not taken (sent {}, accepted {}, window {:?})", search, sent.0, accepted, everything.0);
      return Err("Everything yanıt vermiyor".into());
    }
    if let Some(result) = reply.result.take() { return result; }
    // The usual answer comes in well under WAIT_MS; a late one is still
    // taken up to LATE_MS (and logged, with how late), only then it is a
    // timeout.
    let asked = Instant::now();
    unsafe { SetTimer(hwnd, 1, WAIT_MS, None); }
    let mut waited_long = false;
    loop {
      let mut msg = MSG::default();
      if unsafe { GetMessageW(&mut msg, hwnd, 0, 0) }.0 <= 0 { return Err("Everything IPC kapandı".into()); }
      // GetMessage dispatches incoming sent messages before returning a queued
      // message; the reply can arrive immediately before our timeout timer.
      if let Some(result) = reply.result.take() {
        if waited_long {
          tracing::warn!("Everything: {:?} answered late, after {} ms", search, asked.elapsed().as_millis());
        }
        return result;
      }
      if msg.message == WM_TIMER {
        if waited_long {
          tracing::warn!("Everything: {:?} got no answer in {} ms (window {:?})", search, asked.elapsed().as_millis(), everything.0);
          return Err("Everything araması zaman aşımına uğradı".into());
        }
        waited_long = true;
        unsafe { SetTimer(hwnd, 1, LATE_MS - WAIT_MS, None); }
        continue;
      }
      unsafe { DispatchMessageW(&msg); }
      if let Some(result) = reply.result.take() { return result; }
    }
  }

  unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA {
      let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Reply;
      let cds = lp.0 as *const COPYDATASTRUCT;
      if !state.is_null() && !cds.is_null() && (*cds).dwData == REPLY_ID {
        let data = if (*cds).lpData.is_null() || (*cds).cbData == 0 {
          &[][..]
        } else {
          std::slice::from_raw_parts((*cds).lpData.cast::<u8>(), (*cds).cbData as usize)
        };
        (*state).result.replace(Some(parse_page(data)));
        let _ = PostMessageW(hwnd, WM_APP + 1, WPARAM(0), LPARAM(0));
        return LRESULT(1);
      }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
  }

  fn number(data: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset.checked_add(size_of::<u32>()).ok_or("IPC uzunluğu geçersiz")?;
    let bytes: [u8; 4] = data.get(offset..end).ok_or("IPC yanıtı eksik")?
      .try_into().map_err(|_| "IPC yanıtı eksik")?;
    Ok(u32::from_le_bytes(bytes))
  }

  fn string(data: &[u8], cursor: &mut usize) -> Result<String, String> {
    let len = number(data, *cursor)? as usize;
    *cursor += 4;
    let bytes = len.checked_add(1).and_then(|v| v.checked_mul(2)).ok_or("IPC metni çok uzun")?;
    let end = cursor.checked_add(bytes).ok_or("IPC metni çok uzun")?;
    let slice = data.get(*cursor..end).ok_or("IPC metni eksik")?;
    if slice[bytes - 2..] != [0, 0] { return Err("IPC metni sonlandırılmamış".into()); }
    *cursor = end;
    let chars = slice[..bytes - 2].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>();
    Ok(String::from_utf16_lossy(&chars))
  }

  #[cfg_attr(not(test), allow(dead_code))]
  pub(super) fn parse_reply(data: &[u8]) -> Result<Vec<FileHit>, String> {
    parse_page(data).map(|page| page.hits)
  }

  pub(super) fn parse_page(data: &[u8]) -> Result<FilePage, String> {
    let total = number(data, 0)?;
    let offset = number(data, 8)?;
    let count = number(data, 4)? as usize;
    let flags = number(data, 12)?;
    if flags & REQUEST_NAME_PATH != REQUEST_NAME_PATH || count > 40 { return Err("IPC sonuç biçimi geçersiz".into()); }
    let items_end = 20usize.checked_add(count.checked_mul(8).ok_or("IPC sonuçları çok uzun")?).ok_or("IPC sonuçları çok uzun")?;
    if data.len() < items_end { return Err("IPC sonuçları eksik".into()); }
    let mut hits = Vec::with_capacity(count);
    for i in 0..count {
      let at = 20 + i * 8;
      let item_flags = number(data, at)?;
      let mut cursor = number(data, at + 4)? as usize;
      if cursor < items_end { return Err("IPC veri konumu geçersiz".into()); }
      let name = string(data, &mut cursor)?;
      let path = string(data, &mut cursor)?;
      let full_path = PathBuf::from(&path).join(&name).to_string_lossy().into_owned();
      hits.push(FileHit { name, path, full_path, is_dir: item_flags & 1 != 0 });
    }
    Ok(FilePage { hits, total, offset })
  }

  #[cfg(test)]
  mod callback_tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{PostQuitMessage, WM_CLOSE};

    #[test]
    fn page_request_preserves_offset_limit_and_unicode_search() {
      for (offset, limit, expected_limit) in [(0, 10, 10), (10, 10, 10), (20, 10, 10), (400, 80, 40), (u32::MAX, 0, 1)] {
        let bytes = query_bytes(123, "ödev", limit, offset);
        assert_eq!(number(&bytes, 0).unwrap(), 123);
        assert_eq!(number(&bytes, 4).unwrap(), REPLY_ID as u32);
        assert_eq!(number(&bytes, 8).unwrap(), 0);
        assert_eq!(number(&bytes, 12).unwrap(), offset);
        assert_eq!(number(&bytes, 16).unwrap(), expected_limit);
        assert_eq!(number(&bytes, 20).unwrap(), REQUEST_NAME_PATH);
        assert_eq!(number(&bytes, 24).unwrap(), SORT_NAME);
        let text = bytes[28..].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>();
        assert_eq!(String::from_utf16(&text).unwrap(), "ödev\0");
      }
    }

    #[test]
    fn page_reply_keeps_total_offset_hits_and_legacy_vector() {
      let mut bytes: Vec<u8> = [137u32, 1, 40, 3, 1, 0, 28].into_iter().flat_map(u32::to_le_bytes).collect();
      for text in ["ödev.txt", "C:\\Türkçe"] {
        let chars = text.encode_utf16().collect::<Vec<_>>();
        bytes.extend_from_slice(&(chars.len() as u32).to_le_bytes());
        for ch in chars.into_iter().chain(std::iter::once(0)) { bytes.extend_from_slice(&ch.to_le_bytes()); }
      }
      let page = parse_page(&bytes).unwrap();
      assert_eq!((page.total, page.offset, page.hits.len()), (137, 40, 1));
      assert_eq!(page.hits[0].name, "ödev.txt");
      assert_eq!(page.hits[0].full_path, "C:\\Türkçe\\ödev.txt");
      assert_eq!(parse_reply(&bytes).unwrap(), page.hits);
      bytes[24..28].copy_from_slice(&5000u32.to_le_bytes());
      assert!(parse_page(&bytes).is_err());
      assert!(parse_reply(&bytes).is_err());
    }

    #[test]
    fn empty_last_page_keeps_total_and_requested_offset() {
      let bytes = [137u32, 0, 137, 3, 1].into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
      assert_eq!(parse_page(&bytes).unwrap(), FilePage { hits: Vec::new(), total: 137, offset: 137 });
      assert_eq!(query_page(" ", 10, 20).unwrap(), FilePage { hits: Vec::new(), total: 0, offset: 20 });
      assert!(parse_page(&bytes[..16]).is_err());
    }

    // A hidden IPC peer, with Everything's real request/reply contract. It
    // answers on a later message, so the query must observe a callback write
    // made inside GetMessage. Run this test optimized too: the former &mut
    // alias passed debug tests but timed out in release.
    unsafe extern "system" fn peer(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
      if msg == WM_COPYDATA {
        let cds = &*(lp.0 as *const COPYDATASTRUCT);
        if cds.dwData != QUERY2_UNICODE { return LRESULT(0); }
        let request = std::slice::from_raw_parts(cds.lpData.cast::<u8>(), cds.cbData as usize);
        let _ = PostMessageW(hwnd, WM_APP + 1,
          WPARAM(number(request, 0).unwrap() as usize), LPARAM(number(request, 12).unwrap() as isize));
        return LRESULT(1);
      }
      if msg == WM_APP + 1 {
        let dest = HWND(wp.0 as _);
        let bytes: Vec<u8> = [137u32, 0, lp.0 as u32, 3, 1].into_iter().flat_map(u32::to_le_bytes).collect();
        let cds = COPYDATASTRUCT { dwData: REPLY_ID, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as _ };
        let _ = SendMessageTimeoutW(dest, WM_COPYDATA, WPARAM(hwnd.0 as usize),
          LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG, WAIT_MS, None);
        return LRESULT(0);
      }
      if msg == WM_CLOSE {
        let _ = DestroyWindow(hwnd);
        PostQuitMessage(0);
        return LRESULT(0);
      }
      DefWindowProcW(hwnd, msg, wp, lp)
    }

    #[test]
    fn observes_the_answer_written_by_a_reentrant_window_callback() {
      let (tx, rx) = std::sync::mpsc::channel();
      let thread = std::thread::spawn(move || unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        let class = w!("LogicalLungeEverythingTestPeer");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(peer), hInstance: hinst.into(), lpszClassName: class, ..Default::default() });
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE::default(), class, w!(""), WINDOW_STYLE::default(),
          0, 0, 0, 0, None, None, hinst, None).unwrap();
        tx.send(hwnd.0 as usize).unwrap();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 { DispatchMessageW(&msg); }
      });
      let hwnd = HWND(rx.recv_timeout(Duration::from_secs(3)).unwrap() as _);
      let result = query_window(hwnd, "ipc-fixture", 8, 40);
      unsafe { let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)); }
      thread.join().unwrap();
      assert_eq!(result, Ok(FilePage { hits: Vec::new(), total: 137, offset: 40 }),
        "the callback's page and offset must reach the waiting query");
    }
  }
}

#[cfg(windows)]
pub fn query(search: &str, limit: u32) -> Result<Vec<FileHit>, String> { query_page(search, limit, 0).map(|page| page.hits) }

#[cfg(windows)]
pub fn query_page(search: &str, limit: u32, offset: u32) -> Result<FilePage, String> { windows_ipc::query_page(search, limit, offset) }

#[cfg(not(windows))]
pub fn query(_search: &str, _limit: u32) -> Result<Vec<FileHit>, String> { Err("Everything yalnızca Windows'ta kullanılabilir".into()) }

#[cfg(not(windows))]
pub fn query_page(_search: &str, _limit: u32, _offset: u32) -> Result<FilePage, String> { Err("Everything yalnızca Windows'ta kullanılabilir".into()) }

#[cfg(test)]
mod exe_tests {
  use super::exe_of;
  use std::path::PathBuf;

  #[test]
  fn takes_the_program_out_of_stored_command_lines() {
    let quoted = r#""C:\Program Files\Everything\Everything.exe" -startup"#;
    assert_eq!(exe_of(quoted), Some(PathBuf::from(r"C:\Program Files\Everything\Everything.exe")));
    let icon = r"C:\Users\Öykü\scoop\apps\everything\current\Everything.exe,0";
    assert_eq!(exe_of(icon), Some(PathBuf::from(r"C:\Users\Öykü\scoop\apps\everything\current\Everything.exe")));
    assert_eq!(exe_of(r"C:\Tools\EVERYTHING.EXE -startup"), Some(PathBuf::from(r"C:\Tools\EVERYTHING.EXE")));
    assert_eq!(exe_of(""), None);
    assert_eq!(exe_of("not a program"), None);
  }
}

#[cfg(test)]
mod class_tests {
  use super::is_ipc_class;

  #[test]
  fn matches_the_unnamed_and_any_named_instance_only() {
    assert!(is_ipc_class("EVERYTHING_TASKBAR_NOTIFICATION"));
    assert!(is_ipc_class("EVERYTHING_TASKBAR_NOTIFICATION_(1.5a)"));
    assert!(is_ipc_class("EVERYTHING_TASKBAR_NOTIFICATION_(ETP Server)"));
    assert!(!is_ipc_class("EVERYTHING_TASKBAR_NOTIFICATION_X"));
    assert!(!is_ipc_class("EVERYTHING_TASKBAR_NOTIFICATION_(1.5a"));
    assert!(!is_ipc_class("EVERYTHING"));
  }
}

#[cfg(all(test, windows))]
mod tests {
  use super::{query, windows_ipc::parse_reply};

  #[test]
  fn unicode_result_and_malformed_offset() {
    let mut reply = Vec::new();
    for n in [1u32, 1, 0, 3, 1, 0, 28] { reply.extend_from_slice(&n.to_le_bytes()); }
    for s in ["ödev.txt", "C:\\Türkçe"] {
      let utf16: Vec<u16> = s.encode_utf16().collect();
      reply.extend_from_slice(&(utf16.len() as u32).to_le_bytes());
      for c in utf16.into_iter().chain(std::iter::once(0)) { reply.extend_from_slice(&c.to_le_bytes()); }
    }
    let result = parse_reply(&reply).unwrap();
    assert_eq!(result[0].full_path, "C:\\Türkçe\\ödev.txt");
    reply[24..28].copy_from_slice(&5000u32.to_le_bytes());
    assert!(parse_reply(&reply).is_err());
  }

  #[test]
  #[ignore = "requires a running Everything service and a local indexed volume"]
  fn running_everything_responds() {
    let hits = query("logical-lunge", 8).expect("Everything IPC query");
    assert!(hits.iter().all(|hit| !hit.name.is_empty() && !hit.full_path.is_empty()));
  }

  #[test]
  #[ignore = "requires a running Everything service monitoring the temp volume"]
  fn new_file_is_indexed_without_full_refresh() {
    let name = format!("ll-index-probe-{}.txt", std::process::id());
    let path = std::env::temp_dir().join(&name);
    std::fs::write(&path, b"index probe").unwrap();
    let found = (0..30).any(|_| {
      let hit = query(&name, 4).unwrap_or_default().iter().any(|hit| hit.name == name);
      if !hit { std::thread::sleep(std::time::Duration::from_millis(100)); }
      hit
    });
    let _ = std::fs::remove_file(&path);
    assert!(found, "Everything did not index a newly created NTFS file within 3 seconds");
  }
}
