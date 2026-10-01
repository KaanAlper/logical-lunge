//! Brightness (WMI on laptops, DDC/CI on monitors, through the core's
//! `/brightness`) and gamma (`lunge.exe --gamma`) of one monitor, with the web
//! bar's single axis: gamma 0..100 below brightness 0..100. Reads and writes
//! run on their own threads; writes are debounced. An older core without
//! `/brightness` falls back to `scripts\brightness.ps1`.

use std::{
  os::windows::process::CommandExt,
  path::PathBuf,
  process::Command,
  sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
  },
  time::{Duration, Instant},
};

const NO_WINDOW: u32 = 0x0800_0000;

pub struct Display {
  /// `\\.\DISPLAY1`
  pub device: String,
  /// None: not read yet or not adjustable (then only gamma moves)
  pub brightness: Option<i32>,
  pub read_done: bool,
  pub gamma: i32,
  pub last_set: Instant,
  last_read: std::cell::Cell<Instant>,
  brightness_gen: Arc<AtomicU64>,
  gamma_gen: Arc<AtomicU64>,
}

pub enum Update {
  Brightness(String, Option<i32>),
  Gamma(String, i32),
}

/// What a wheel step did, for the OSD.
pub enum Step {
  Gamma(i32),
  Brightness(i32),
  None,
}

fn install_dir() -> Option<PathBuf> {
  Some(std::env::current_exe().ok()?.parent()?.to_path_buf())
}

fn core(args: &[&str]) -> Option<String> {
  let exe = install_dir()?.join("lunge.exe");
  let out = Command::new(exe).args(args).creation_flags(NO_WINDOW).output().ok()?;
  Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The core answers in-process (one WMI or DDC/CI call); the script it
/// replaces started PowerShell and two WMI queries per wheel step, which
/// lagged by up to a second on laptops.
fn brightness_path(device: &str) -> String {
  format!("/brightness?dev={}", device.replace('\\', "%5C"))
}

/// Ok(None): this monitor's brightness cannot be set. Err: the core did not
/// answer (an older core), so the script is used instead.
fn core_read(device: &str) -> Result<Option<i32>, ()> {
  let (status, body) =
    super::core_api::post(&brightness_path(device)).ok_or(())?;
  if status != 200 {
    return Err(());
  }
  let v: serde_json::Value =
    serde_json::from_slice(&body).map_err(|_| ())?;
  Ok(v["value"].as_i64().map(|n| n as i32))
}

fn core_write(device: &str, value: i32) -> bool {
  let path = format!("{}&v={}", brightness_path(device), value);
  matches!(super::core_api::post(&path), Some((204, _)))
}

fn script_brightness(args: &[&str]) -> Option<String> {
  let script = install_dir()?.join("scripts").join("brightness.ps1");
  let script = script.to_string_lossy().into_owned();
  let mut all = vec!["--ps", script.as_str()];
  all.extend_from_slice(args);
  core(&all)
}

impl Display {
  pub fn new(device: String) -> Self {
    Self {
      device,
      brightness: None,
      read_done: false,
      gamma: 100,
      last_set: Instant::now() - Duration::from_secs(60),
      last_read: std::cell::Cell::new(Instant::now()),
      brightness_gen: Arc::default(),
      gamma_gen: Arc::default(),
    }
  }

  /// Worth reading again: not read for 30 s and not just set by us.
  pub fn stale(&self) -> bool {
    self.last_read.get().elapsed() > Duration::from_secs(30) && self.last_set.elapsed() > Duration::from_secs(5)
  }

  /// Reads both values (DDC takes ~0.5 s: done ahead of the first wheel).
  pub fn read(&self, send: impl Fn(Update) + Send + Clone + 'static) {
    self.last_read.set(Instant::now());
    let device = self.device.clone();
    let send2 = send.clone();
    let dev2 = device.clone();
    std::thread::spawn(move || {
      let value = core_read(&device).unwrap_or_else(|()| {
        script_brightness(&["get", &format!("{}\\Monitor0", device)])
          .and_then(|v| v.parse().ok())
      });
      send(Update::Brightness(device, value));
    });
    std::thread::spawn(move || {
      if let Some(json) = core(&["--gamma", &dev2]) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
          if let Some(g) = v["gamma"].as_i64() {
            send2(Update::Gamma(dev2, g as i32));
          }
        }
      }
    });
  }

  /// One wheel notch (web bar `onLeftWheel`).
  pub fn wheel(&mut self, up: bool) -> Step {
    if self.gamma < 100 && up {
      self.set_gamma((self.gamma + 5).min(100));
      return Step::Gamma(self.gamma);
    }
    let Some(cur) = self.brightness else {
      if !self.read_done {
        return Step::None;
      }
      // not adjustable (no DDC): gamma only
      self.set_gamma((self.gamma + if up { 5 } else { -5 }).clamp(0, 100));
      return Step::Gamma(self.gamma);
    };
    if !up && cur == 0 {
      self.set_gamma((self.gamma - 5).max(0));
      return Step::Gamma(self.gamma);
    }
    let next = (cur + if up { 5 } else { -5 }).clamp(0, 100);
    self.set_brightness(next);
    Step::Brightness(next)
  }

  fn set_gamma(&mut self, v: i32) {
    self.gamma = v;
    let gen = self.gamma_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let counter = self.gamma_gen.clone();
    let device = self.device.clone();
    std::thread::spawn(move || {
      std::thread::sleep(Duration::from_millis(40));
      if counter.load(Ordering::SeqCst) == gen {
        core(&["--gamma", &device, &v.to_string()]);
      }
    });
  }

  fn set_brightness(&mut self, v: i32) {
    self.brightness = Some(v);
    self.last_set = Instant::now();
    let gen = self.brightness_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let counter = self.brightness_gen.clone();
    let device = self.device.clone();
    std::thread::spawn(move || {
      // short: the core applies the latest value of a burst on its own
      std::thread::sleep(Duration::from_millis(40));
      if counter.load(Ordering::SeqCst) == gen && !core_write(&device, v) {
        let monitor = format!("{}\\Monitor0", device);
        script_brightness(&["set", &v.to_string(), &monitor]);
      }
    });
  }
}
