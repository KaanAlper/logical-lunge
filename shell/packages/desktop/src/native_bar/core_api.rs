//! The core's local HTTP API (`lunge.exe`, 127.0.0.1:6131). Every call runs
//! off the UI thread; a missing core never blocks the bar.

use std::{
  io::{BufRead, BufReader, Read, Write},
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

/// Writes a user preference (`prefs.json`, validated by the core); if the
/// core does not answer, `lunge.exe --set-pref` does the same.
pub fn set_pref(key: &'static str, value: &'static str) {
  std::thread::spawn(move || {
    if matches!(post(&format!("/pref?k={}&v={}", key, value)), Some((204, _))) {
      return;
    }
    run_core(&["--set-pref", key, value]);
  });
}

/// The core's event stream (`/events`, the one the web widgets get through
/// the toast widget): `on(Some("ll:..."))` for each event, `on(None)` each
/// time the stream (re)connects -- state may have changed while it was down.
/// Reconnects for as long as the shell runs (the core restarts, or starts
/// after the shell). Blocks on the socket: no polling.
pub fn events(on: impl Fn(Option<String>) + Send + 'static) {
  let _ = std::thread::Builder::new().name("core-events".into()).spawn(move || {
    let mut wait = 1;
    loop {
      if let Some(stream) = open_events() {
        wait = 1;
        on(None);
        read_events(stream, &on);
      }
      std::thread::sleep(Duration::from_secs(wait));
      wait = (wait * 2).min(10);
    }
  });
}

fn open_events() -> Option<TcpStream> {
  let addr: SocketAddr = "127.0.0.1:6131".parse().ok()?;
  let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).ok()?;
  // the core pings every 20 s: a longer silence means the connection is gone
  s.set_read_timeout(Some(Duration::from_secs(50))).ok()?;
  s.set_write_timeout(Some(Duration::from_secs(1))).ok()?;
  write!(s, "GET /events HTTP/1.1\r\nHost: 127.0.0.1:6131\r\n\r\n").ok()?;
  Some(s)
}

fn read_events(stream: TcpStream, on: &impl Fn(Option<String>)) {
  let mut reader = BufReader::new(stream);
  let mut line = String::new();
  loop {
    line.clear();
    match reader.read_line(&mut line) {
      Ok(0) | Err(_) => return,
      Ok(_) => {}
    }
    // `data: {"emit":"ll:theme-dark"}`; toasts and pings are not ours
    let Some(json) = line.trim_end().strip_prefix("data: ") else { continue };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { continue };
    if let Some(evt) = v["emit"].as_str().filter(|e| e.starts_with("ll:")) {
      on(Some(evt.to_string()));
    }
  }
}

/// `lunge.exe` next to the shell (the install folder).
pub fn core_exe() -> Option<PathBuf> {
  let exe = std::env::current_exe().ok()?.parent()?.join("lunge.exe");
  exe.exists().then_some(exe)
}

/// Workspace switch with the core's slide animation; if the core does not
/// answer, `lunge.exe --slide`; if that fails, the plain WM command
/// (`fallback`).
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
