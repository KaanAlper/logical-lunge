//! The core's game mode (`ll:game-mode-on:<device>` / `ll:game-mode-off`):
//! a fullscreen app is in front on that monitor. Meanwhile the bar draws
//! nothing there, the providers and the desktop widgets stop polling and
//! the tray spy stops its z-order check; when it ends everything catches up
//! at once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crossbeam::channel::{bounded, Receiver, Sender};

static ON: AtomicBool = AtomicBool::new(false);
static DEVICE: Mutex<String> = Mutex::new(String::new());
/// Interval ticks waiting for the end of game mode.
static WAITERS: Mutex<Vec<Sender<Instant>>> = Mutex::new(Vec::new());

pub fn on() -> bool {
  ON.load(Ordering::Acquire)
}

/// Game mode is on for this monitor (`\\.\DISPLAY1`).
pub fn covers(device: &str) -> bool {
  on() && DEVICE.lock().unwrap_or_else(|e| e.into_inner()).eq_ignore_ascii_case(device)
}

/// Turns game mode on for a monitor (Some) or off (None); true when it
/// changed.
pub fn set(device: Option<&str>) -> bool {
  let mut waiters = WAITERS.lock().unwrap_or_else(|e| e.into_inner());
  let mut current = DEVICE.lock().unwrap_or_else(|e| e.into_inner());
  let changed = match device {
    Some(d) => {
      let changed = !on() || !current.eq_ignore_ascii_case(d);
      *current = d.to_string();
      ON.store(true, Ordering::Release);
      changed
    }
    None => {
      let changed = on();
      current.clear();
      ON.store(false, Ordering::Release);
      let now = Instant::now();
      for tx in waiters.drain(..) {
        let _ = tx.try_send(now);
      }
      changed
    }
  };
  drop(current);
  drop(waiters);
  #[cfg(windows)]
  systray_util::set_quiet(device.is_some());
  changed
}

/// Fires when game mode is off: at once when it already is.
pub fn off_signal() -> Receiver<Instant> {
  let (tx, rx) = bounded(1);
  let mut waiters = WAITERS.lock().unwrap_or_else(|e| e.into_inner());
  if on() {
    waiters.push(tx);
  } else {
    let _ = tx.try_send(Instant::now());
  }
  rx
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn waits_until_game_mode_ends() {
    assert!(set(Some(r"\\.\DISPLAY1")));
    assert!(covers(r"\\.\display1") && !covers(r"\\.\DISPLAY2"));
    let rx = off_signal();
    assert!(rx.try_recv().is_err());
    assert!(set(None));
    assert!(rx.try_recv().is_ok());
    assert!(off_signal().try_recv().is_ok());
    assert!(!set(None));
  }
}
