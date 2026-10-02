use windows::{
  core::PWSTR,
  Win32::{
    Foundation::CloseHandle,
    System::Threading::{
      OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
      PROCESS_QUERY_LIMITED_INFORMATION,
    },
  },
};

/// Full path of a process's exe (`C:\...\app.exe`); None if the process is
/// gone or not readable. Works for elevated processes too (limited query).
pub fn process_image_path(pid: u32) -> Option<String> {
  unsafe {
    let process =
      OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(
      process,
      PROCESS_NAME_WIN32,
      PWSTR(buf.as_mut_ptr()),
      &mut len,
    )
    .is_ok();
    let _ = CloseHandle(process);
    ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
  }
}
