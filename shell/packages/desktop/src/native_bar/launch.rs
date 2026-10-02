//! The shell's one way to start things (apps from the Super menu and the
//! Dock, folders, "run as administrator"): the core's `POST /launch`. The
//! core checks the target first (a missing file, an unknown type, an
//! unregistered address) and says so with our card instead of Windows'
//! box, and starts it as the user even though the shell runs elevated.
//! Without a core (it is restarting) the shell starts it itself with
//! `SEE_MASK_FLAG_NO_UI`, so Windows shows nothing either.

use std::{thread, time::Duration};

use super::core_api;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Verb {
  Open,
  RunAs,
}

impl Verb {
  fn name(self) -> &'static str {
    match self {
      Verb::Open => "open",
      Verb::RunAs => "runas",
    }
  }
}

fn encode(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for b in s.bytes() {
    if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

/// The core's route for `file` (with `args`, `verb`).
fn route(file: &str, args: &str, verb: Verb) -> String {
  let mut path = format!("/launch?file={}", encode(file));
  if !args.is_empty() {
    path.push_str(&format!("&args={}", encode(args)));
  }
  if verb != Verb::Open {
    path.push_str(&format!("&verb={}", verb.name()));
  }
  path
}

/// Starts `file` off the UI thread; failures reach the user as our card.
pub fn open(file: impl Into<String>, args: impl Into<String>, verb: Verb) {
  let (file, args) = (file.into(), args.into());
  if file.trim().is_empty() {
    return;
  }
  // the core may wait on Explorer for a while: a slow answer is not a
  // missing core (starting it here too would start it twice)
  thread::spawn(move || match core_api::post_waiting(&route(&file, &args, verb), Duration::from_secs(20)) {
    Ok(Some((204, _))) | Ok(Some((422, _))) => {}
    Ok(Some((status, _))) => tracing::warn!("Launch: the core answered {} for {}", status, file),
    Ok(None) => tracing::warn!("Launch: no answer from the core for {}", file),
    Err(_) => {
      if let Err(err) = start_here(&file, &args, verb) {
        tracing::warn!("Launch: {} could not start without the core: {:?}", file, err);
      }
    }
  });
}

/// The fallback: ShellExecuteEx with `SEE_MASK_FLAG_NO_UI` (an error comes
/// back instead of Windows' message box).
#[cfg(windows)]
fn start_here(file: &str, args: &str, verb: Verb) -> windows::core::Result<()> {
  use windows::{
    core::{HSTRING, PCWSTR},
    Win32::UI::{
      Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW},
      WindowsAndMessaging::SW_SHOWNORMAL,
    },
  };
  let (file_w, args_w) = (HSTRING::from(file), HSTRING::from(args));
  let verb_w = HSTRING::from(match verb {
    Verb::Open => "",
    v => v.name(),
  });
  let mut info = SHELLEXECUTEINFOW {
    cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
    fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
    lpVerb: if verb == Verb::Open { PCWSTR::null() } else { PCWSTR(verb_w.as_ptr()) },
    lpFile: PCWSTR(file_w.as_ptr()),
    lpParameters: if args.is_empty() { PCWSTR::null() } else { PCWSTR(args_w.as_ptr()) },
    nShow: SW_SHOWNORMAL.0,
    ..Default::default()
  };
  unsafe { ShellExecuteExW(&mut info) }
}

#[cfg(not(windows))]
fn start_here(_: &str, _: &str, _: Verb) -> Result<(), ()> {
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_route_carries_the_target_encoded() {
    assert_eq!(route(r"C:\a b\x.txt", "", Verb::Open), "/launch?file=C%3A%5Ca%20b%5Cx.txt");
    assert_eq!(route("shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", "", Verb::RunAs), "/launch?file=shell%3AAppsFolder%5CMicrosoft.WindowsCalculator_8wekyb3d8bbwe%21App&verb=runas");
    assert_eq!(route("wt.exe", "-d C:\\", Verb::Open), "/launch?file=wt.exe&args=-d%20C%3A%5C");
  }
}
