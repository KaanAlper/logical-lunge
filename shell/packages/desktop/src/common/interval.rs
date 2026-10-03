use std::time::{Duration, Instant};

/// An interval timer for synchronous contexts using crossbeam.
///
/// For use with crossbeam's `select!` macro.
pub struct SyncInterval {
  interval: Duration,
  next_tick: Instant,
  is_first: bool,
}

impl SyncInterval {
  pub fn new(interval_ms: u64) -> Self {
    Self {
      interval: Duration::from_millis(interval_ms),
      next_tick: Instant::now(),
      is_first: true,
    }
  }

  /// Returns a receiver that will get a message at the next tick time.
  pub fn tick(&mut self) -> crossbeam::channel::Receiver<Instant> {
    if self.is_first {
      // Emit immediately on the first tick.
      self.is_first = false;
      crossbeam::channel::after(Duration::from_secs(0))
    } else if let Some(wait_duration) =
      self.next_tick.checked_duration_since(Instant::now())
    {
      // Wait normally until the next tick.
      let timer = crossbeam::channel::after(wait_duration);
      self.next_tick += self.interval;
      timer
    } else {
      // We're behind - skip missed ticks to catch up.
      while self.next_tick <= Instant::now() {
        self.next_tick += self.interval;
      }

      crossbeam::channel::after(self.next_tick - Instant::now())
    }
  }
}
