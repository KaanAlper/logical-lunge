//! Whether a monitor is switched off at its own button. Windows keeps such a
//! monitor in the desktop (over HDMI, DVI and VGA it does not notice), so
//! its video would go on decoding for nobody: a 4K video takes about a
//! third of the GPU's video decoder. The monitor itself tells, through
//! DDC/CI (VESA power mode, VCP code D6). It is asked every few seconds on
//! a thread of its own (a reply takes ~50 ms on the bus; a monitor that does
//! not answer, at once), and again at once when the displays wake.
//!
//! A monitor that answered before and goes silent, or answers "off",
//! counts as off; one that never answered (no DDC/CI, a virtual display)
//! keeps playing.

use std::{
  sync::{
    mpsc::{self, RecvTimeoutError, Sender},
    Mutex,
  },
  time::Duration,
};

/// A monitor's answer to "power mode?".
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reply {
  On,
  Off,
  Silent,
}

/// VCP D6: 1 on, 2 standby, 3 suspend, 4 off, 5 off at the power button.
/// Any other value does not pause anything.
pub fn reply_of(value: Option<u32>) -> Reply {
  match value {
    Some(2..=5) => Reply::Off,
    Some(_) => Reply::On,
    None => Reply::Silent,
  }
}

/// Silent replies in a row, after answering before, that make a monitor
/// off: one lost on a busy bus, or the first one after the displays wake,
/// is not enough.
const SILENT_OFF: u32 = 2;

/// What is known of one monitor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Seen {
  answered: bool,
  silent: u32,
}

impl Seen {
  /// Takes the newest reply; true while the monitor counts as off.
  pub fn off_after(&mut self, reply: Reply) -> bool {
    match reply {
      Reply::On | Reply::Off => {
        *self = Seen { answered: true, silent: 0 };
        reply == Reply::Off
      }
      Reply::Silent => {
        if self.answered {
          self.silent += 1;
        }
        self.answered && self.silent >= SILENT_OFF
      }
    }
  }
}

/// How often the monitors are asked.
const POLL: Duration = Duration::from_secs(10);

/// GDI device names (`\\.\DISPLAY2`) of the monitors that are off.
static OFF: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WAKE: Mutex<Option<Sender<()>>> = Mutex::new(None);

pub fn off() -> Vec<String> {
  OFF.lock().map(|o| o.clone()).unwrap_or_default()
}

/// Asks again now (the displays woke, the monitors changed).
pub fn ask_now() {
  if let Ok(w) = WAKE.lock() {
    if let Some(tx) = w.as_ref() {
      let _ = tx.send(());
    }
  }
}

/// Starts the watching thread; `changed` runs on it whenever the set of
/// monitors that are off changes.
#[cfg(windows)]
pub fn watch(changed: impl Fn() + Send + 'static) {
  let (tx, rx) = mpsc::channel();
  if let Ok(mut w) = WAKE.lock() {
    *w = Some(tx);
  }
  let _ = std::thread::Builder::new()
    .name("monitor-power".into())
    .spawn(move || {
      let mut seen = std::collections::HashMap::<String, Seen>::new();
      loop {
        let before = off();
        let mut now_off = Vec::new();
        for (device, value) in ask_all() {
          let off = seen.entry(device.clone()).or_default().off_after(reply_of(value));
          if off {
            now_off.push(device.clone());
          }
          if off != before.contains(&device) {
            crate::log::line(&format!(
              "{device} {}",
              match (off, value) {
                (true, None) => "stopped answering (DDC/CI): switched off, its video pauses",
                (true, Some(_)) => "says it is off (DDC/CI): its video pauses",
                (false, _) => "answers again: its video plays",
              }
            ));
          }
        }
        now_off.sort();
        let differs = OFF
          .lock()
          .map(|mut o| {
            let d = *o != now_off;
            *o = now_off;
            d
          })
          .unwrap_or(false);
        if differs {
          changed();
        }
        match rx.recv_timeout(POLL) {
          Ok(()) => {
            // a burst of wake-ups asks once
            while rx.try_recv().is_ok() {}
          }
          Err(RecvTimeoutError::Timeout) => {}
          Err(RecvTimeoutError::Disconnected) => break,
        }
      }
    });
}

/// (GDI device name, D6 value or None when it did not answer) of every
/// monitor.
#[cfg(windows)]
fn ask_all() -> Vec<(String, Option<u32>)> {
  use windows::Win32::{
    Devices::Display::{
      DestroyPhysicalMonitors, GetNumberOfPhysicalMonitorsFromHMONITOR,
      GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply,
      PHYSICAL_MONITOR,
    },
    Foundation::{BOOL, LPARAM, RECT},
    Graphics::Gdi::{
      EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
      MONITORINFOEXW,
    },
  };

  unsafe extern "system" fn each(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    lp: LPARAM,
  ) -> BOOL {
    let list = &mut *(lp.0 as *mut Vec<HMONITOR>);
    list.push(monitor);
    BOOL(1)
  }
  let mut monitors: Vec<HMONITOR> = Vec::new();
  let mut out = Vec::new();
  unsafe {
    let _ = EnumDisplayMonitors(
      HDC::default(),
      None,
      Some(each),
      LPARAM(&mut monitors as *mut _ as isize),
    );
    for monitor in monitors {
      let mut info = MONITORINFOEXW::default();
      info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
      if !GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        continue;
      }
      let n = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
      let device = String::from_utf16_lossy(&info.szDevice[..n]);
      let mut count = 0u32;
      if GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count).is_err() || count == 0 {
        out.push((device, None));
        continue;
      }
      let mut physical = vec![PHYSICAL_MONITOR::default(); count as usize];
      if GetPhysicalMonitorsFromHMONITOR(monitor, &mut physical).is_err() {
        out.push((device, None));
        continue;
      }
      let mut value = 0u32;
      let answered = GetVCPFeatureAndVCPFeatureReply(physical[0].hPhysicalMonitor, 0xD6, None, &mut value, None) != 0;
      let _ = DestroyPhysicalMonitors(&physical);
      out.push((device, answered.then_some(value)));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Every reply sequence up to five long, against the rule written out
  /// plainly: off after an "off" answer, or after SILENT_OFF silences in a
  /// row once the monitor has answered.
  #[test]
  fn off_follows_the_monitors_answers() {
    let replies = [Reply::On, Reply::Off, Reply::Silent];
    for len in 1..=5u32 {
      for code in 0..3u32.pow(len) {
        let seq: Vec<Reply> = (0..len).map(|i| replies[(code / 3u32.pow(i) % 3) as usize]).collect();
        let mut seen = Seen::default();
        let mut got = false;
        for &r in &seq {
          got = seen.off_after(r);
        }
        let answered = seq.iter().position(|&r| r != Reply::Silent);
        let silences = seq.iter().rev().take_while(|&&r| r == Reply::Silent).count() as u32;
        let want = match (answered, seq.last()) {
          (_, Some(Reply::Off)) => true,
          (Some(_), Some(Reply::Silent)) => silences >= SILENT_OFF,
          _ => false,
        };
        assert_eq!(got, want, "{seq:?}");
      }
    }
  }

  #[test]
  fn power_mode_values() {
    assert_eq!(reply_of(Some(1)), Reply::On);
    for v in 2..=5 {
      assert_eq!(reply_of(Some(v)), Reply::Off, "{v}");
    }
    assert_eq!(reply_of(Some(0)), Reply::On, "unknown values do not pause");
    assert_eq!(reply_of(Some(9)), Reply::On, "unknown values do not pause");
    assert_eq!(reply_of(None), Reply::Silent);
  }
}
