//! Brightness (WMI on laptops, DDC/CI on monitors) and gamma of one monitor,
//! both through the core's local HTTP routes (`/brightness`, `/gamma`; one
//! in-process call each), with the web bar's single axis: gamma 0..100 below
//! brightness 0..100. A wheel burst goes to one background worker per
//! monitor that applies the latest value of each kind; the worker ends after
//! a while without work, so no thread waits while nobody scrolls.

use std::{
  sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
  time::{Duration, Instant},
};

pub struct Display {
  /// `\\.\DISPLAY1`
  pub device: String,
  /// None: not read yet or not adjustable (then only gamma moves)
  pub brightness: Option<i32>,
  pub read_done: bool,
  pub gamma: i32,
  pub last_set: Instant,
  last_read: std::cell::Cell<Instant>,
  work: Arc<Work>,
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

type Reply = Box<dyn Fn(Update) + Send>;

/// What the monitor's worker still has to do: only the latest value of each
/// kind (a burst of wheel steps becomes one write per kind).
#[derive(Default)]
struct Pending {
  brightness: Option<i32>,
  gamma: Option<i32>,
  read: Option<Reply>,
  running: bool,
}

impl Pending {
  fn is_empty(&self) -> bool {
    self.brightness.is_none()
      && self.gamma.is_none()
      && self.read.is_none()
  }
}

#[derive(Default)]
struct Work {
  pending: Mutex<Pending>,
  wake: Condvar,
}

/// The worker ends after this long without work.
const IDLE: Duration = Duration::from_secs(30);
/// A read while the core is not answering yet (the bar can start first).
const READ_ATTEMPTS: u32 = 5;

fn route(name: &str, device: &str) -> String {
  format!("/{name}?dev={}", device.replace('\\', "%5C"))
}

/// Ok(None): the core says this value cannot be read here (brightness of a
/// monitor without DDC/CI). Err: the core did not answer.
fn read_value(
  name: &str,
  key: &str,
  device: &str,
) -> Result<Option<i32>, ()> {
  let (status, body) =
    super::core_api::post(&route(name, device)).ok_or(())?;
  if status != 200 {
    return Err(());
  }
  let v: serde_json::Value =
    serde_json::from_slice(&body).map_err(|_| ())?;
  Ok(v[key].as_i64().map(|n| n as i32))
}

fn write_value(name: &str, device: &str, value: i32) {
  let path = format!("{}&v={value}", route(name, device));
  if !matches!(super::core_api::post(&path), Some((204, _))) {
    tracing::warn!(
      "Native bar: {name} {value} was not applied to {device}"
    );
  }
}

impl Work {
  fn lock(&self) -> MutexGuard<'_, Pending> {
    self.pending.lock().unwrap_or_else(PoisonError::into_inner)
  }

  /// Hands work to the monitor's worker, starting one if none is running.
  fn submit(
    self: &Arc<Self>,
    device: &str,
    change: impl FnOnce(&mut Pending),
  ) {
    let mut pending = self.lock();
    change(&mut pending);
    if pending.running {
      self.wake.notify_one();
      return;
    }
    pending.running = true;
    drop(pending);
    let work = Arc::clone(self);
    let device = device.to_string();
    std::thread::spawn(move || work.run(&device));
  }

  fn run(&self, device: &str) {
    loop {
      let (brightness, gamma, read) = {
        let mut pending = self.lock();
        while pending.is_empty() {
          let (next, wait) = self
            .wake
            .wait_timeout(pending, IDLE)
            .unwrap_or_else(PoisonError::into_inner);
          pending = next;
          if wait.timed_out() && pending.is_empty() {
            pending.running = false;
            return;
          }
        }
        (
          pending.brightness.take(),
          pending.gamma.take(),
          pending.read.take(),
        )
      };
      if let Some(v) = brightness {
        write_value("brightness", device, v);
      }
      if let Some(v) = gamma {
        write_value("gamma", device, v);
      }
      if let Some(reply) = read {
        Self::read(device, &reply);
      }
    }
  }

  /// Both values, after any pending writes. Nothing is sent while the core
  /// does not answer: the bar keeps what it knew and reads again later.
  fn read(device: &str, reply: &Reply) {
    for attempt in 0..READ_ATTEMPTS {
      if attempt > 0 {
        std::thread::sleep(Duration::from_secs(1));
      }
      let Ok(brightness) = read_value("brightness", "value", device)
      else {
        continue;
      };
      reply(Update::Brightness(device.to_string(), brightness));
      if let Ok(Some(gamma)) = read_value("gamma", "gamma", device) {
        reply(Update::Gamma(device.to_string(), gamma));
      }
      return;
    }
  }
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
      work: Arc::default(),
    }
  }

  /// Worth reading again: not read for 30 s and not just set by us.
  pub fn stale(&self) -> bool {
    self.last_read.get().elapsed() > Duration::from_secs(30)
      && self.last_set.elapsed() > Duration::from_secs(5)
  }

  /// Reads both values (DDC takes ~0.5 s: done ahead of the first wheel).
  pub fn read(&self, send: impl Fn(Update) + Send + 'static) {
    self.last_read.set(Instant::now());
    self
      .work
      .submit(&self.device, |p| p.read = Some(Box::new(send)));
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
    self.work.submit(&self.device, |p| p.gamma = Some(v));
  }

  fn set_brightness(&mut self, v: i32) {
    self.brightness = Some(v);
    self.last_set = Instant::now();
    self.work.submit(&self.device, |p| p.brightness = Some(v));
  }
}
