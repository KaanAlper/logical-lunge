//! Logical Lunge: how often a window's own move may make the WM correct it.
//!
//! Several answers to a window moving or resizing itself put it back (into
//! its tile, its maximized area, its fullscreen state). A window that
//! never accepts the place (a minimum size, a size it keeps restoring, one
//! being destroyed) answered each correction with another move: the WM and
//! the app fought without end, every round a full sync. Each window gets a
//! few corrections a second; past that it is left where it put itself for
//! a while (logged once), and corrections resume once it is quiet.

use std::{
  collections::HashMap,
  time::{Duration, Instant},
};

/// Corrections a window may get within `WINDOW`.
const MAX_CORRECTIONS: u32 = 6;
const WINDOW: Duration = Duration::from_secs(1);
/// How long a window that used up its corrections is left alone.
const REST: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
struct Entry {
  started: Option<Instant>,
  count: u32,
  resting_until: Option<Instant>,
}

/// The correction allowance of each window (by handle).
#[derive(Debug, Default)]
pub struct CorrectionBudget {
  entries: HashMap<isize, Entry>,
}

/// Whether a correction may be made.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
  Allow,
  /// Not now; `first` for the first refusal of this rest (to log once).
  Refuse { first: bool },
}

impl CorrectionBudget {
  /// Asks for one correction of `window` at `now`.
  pub fn ask(&mut self, window: isize, now: Instant) -> Verdict {
    // entries of windows that are quiet again are dropped
    self.entries.retain(|_, entry| {
      entry.resting_until.is_some_and(|until| now < until)
        || entry.started.is_some_and(|start| now.duration_since(start) < WINDOW)
    });

    let entry = self.entries.entry(window).or_default();
    if let Some(until) = entry.resting_until {
      if now < until {
        return Verdict::Refuse { first: false };
      }
      entry.resting_until = None;
    }
    match entry.started {
      Some(start) if now.duration_since(start) < WINDOW => entry.count += 1,
      _ => {
        entry.started = Some(now);
        entry.count = 1;
      }
    }
    if entry.count > MAX_CORRECTIONS {
      entry.resting_until = Some(now + REST);
      entry.started = None;
      entry.count = 0;
      return Verdict::Refuse { first: true };
    }
    Verdict::Allow
  }

  /// Forgets a window (closed).
  pub fn forget(&mut self, window: isize) {
    self.entries.remove(&window);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_few_corrections_a_second_are_allowed() {
    let mut budget = CorrectionBudget::default();
    let t = Instant::now();
    for i in 0..MAX_CORRECTIONS {
      assert_eq!(budget.ask(1, t + Duration::from_millis(u64::from(i) * 10)), Verdict::Allow);
    }
  }

  #[test]
  fn a_window_that_keeps_moving_itself_is_left_alone_then_corrected_again() {
    let mut budget = CorrectionBudget::default();
    let t = Instant::now();
    for _ in 0..MAX_CORRECTIONS {
      assert_eq!(budget.ask(1, t), Verdict::Allow);
    }
    assert_eq!(budget.ask(1, t), Verdict::Refuse { first: true });
    assert_eq!(budget.ask(1, t + Duration::from_secs(1)), Verdict::Refuse { first: false });
    // another window is not affected
    assert_eq!(budget.ask(2, t + Duration::from_secs(1)), Verdict::Allow);
    // after the rest
    assert_eq!(budget.ask(1, t + REST + Duration::from_millis(1)), Verdict::Allow);
  }

  #[test]
  fn corrections_spread_over_time_never_run_out() {
    let mut budget = CorrectionBudget::default();
    let t = Instant::now();
    for i in 0..100u64 {
      assert_eq!(budget.ask(1, t + Duration::from_millis(i * 300)), Verdict::Allow);
    }
  }

  #[test]
  fn quiet_windows_are_dropped() {
    let mut budget = CorrectionBudget::default();
    let t = Instant::now();
    budget.ask(1, t);
    budget.ask(2, t + Duration::from_secs(3));
    assert_eq!(budget.entries.len(), 1);
    budget.forget(2);
    assert!(budget.entries.is_empty());
  }
}
