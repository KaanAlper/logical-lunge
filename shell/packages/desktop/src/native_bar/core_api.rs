//! The core's local HTTP API (`lunge.exe`, 127.0.0.1:6131). Every call runs
//! off the UI thread; a missing core never blocks the bar.

use std::{
  io::{Read, Write},
  net::{SocketAddr, TcpStream},
  path::PathBuf,
  time::Duration,
};

/// POST `path`; returns the status code and body.
pub fn post(path: &str) -> Option<(u16, Vec<u8>)> {
  let addr: SocketAddr = "127.0.0.1:6131".parse().ok()?;
  let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
  s.set_read_timeout(Some(Duration::from_secs(4))).ok()?;
  s.set_write_timeout(Some(Duration::from_secs(1))).ok()?;
  write!(
    s,
    "POST {} HTTP/1.1\r\nHost: 127.0.0.1:6131\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    path
  )
  .ok()?;
  let mut raw = Vec::new();
  s.read_to_end(&mut raw).ok()?;
  let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
  let head = String::from_utf8_lossy(&raw[..head_end]);
  let status = head.split_whitespace().nth(1)?.parse().ok()?;
  Some((status, raw[head_end + 4..].to_vec()))
}

/// Fire and forget.
pub fn post_async(path: String) {
  std::thread::spawn(move || {
    post(&path);
  });
}

/// `lunge.exe` next to the shell (the install folder).
pub fn core_exe() -> Option<PathBuf> {
  let exe = std::env::current_exe().ok()?.parent()?.join("lunge.exe");
  exe.exists().then_some(exe)
}

/// Workspace switch with the core's slide animation; if the core does not
/// answer, `lunge.exe --slide`; if that fails, the plain WM command
/// (`fallback`). Same chain as the web bar.
pub fn slide(target: String, fallback: impl FnOnce() + Send + 'static) {
  std::thread::spawn(move || {
    if matches!(post(&format!("/cmd?a=ws-{}", target)), Some((204, _))) {
      return;
    }
    if let Some(exe) = core_exe() {
      use std::os::windows::process::CommandExt;
      if std::process::Command::new(exe)
        .args(["--slide", &target])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .is_ok()
      {
        return;
      }
    }
    fallback();
  });
}

/// `lunge.exe <args>` without a console window.
pub fn run_core(args: &[&str]) {
  if let Some(exe) = core_exe() {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new(exe).args(args).creation_flags(0x0800_0000).spawn();
  }
}
