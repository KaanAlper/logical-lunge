//! Brightness (WMI on laptops, DDC/CI on monitors, through
//! `scripts\brightness.ps1`) and gamma (`lunge.exe --gamma`) of one monitor,
//! with the web bar's single axis: gamma 0..100 below brightness 0..100.
//! Reads and writes run on their own threads; writes are debounced.

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
      let script = install_dir().map(|d| d.join("scripts").join("brightness.ps1"));
      let value = script.and_then(|s| {
        core(&["--ps", &s.to_string_lossy(), "get", &format!("{}\\Monitor0", device)])
      });
      send(Update::Brightness(device, value.and_then(|v| v.parse().ok())));
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
      std::thread::sleep(Duration::from_millis(120));
      if counter.load(Ordering::SeqCst) == gen {
        if let Some(s) = install_dir().map(|d| d.join("scripts").join("brightness.ps1")) {
          core(&["--ps", &s.to_string_lossy(), "set", &v.to_string(), &format!("{}\\Monitor0", device)]);
        }
      }
    });
  }
}
