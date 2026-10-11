//! Logical Lunge: what the window manager's thread is doing, watched from
//! a thread of its own.
//!
//! The WM runs every event and IPC command on one thread. When that thread
//! was stuck (a v1.5.2 report: minutes without an IPC answer, at full
//! processor use, until the core killed it), its log had nothing: only
//! warnings are kept, and the thread that would write them was the stuck
//! one. Now the watch thread writes, while the WM thread is still inside
//! one step:
//! - after `STALL_LEVELS_MS`: which step (an event of which window, an IPC
//!   message), whether the thread is spinning or waiting (its processor use
//!   over a moment), and its call stack (`module+offset`, resolved with the
//!   build's symbols);
//! - every step that took longer than `SLOW` once it ends;
//! - an abnormal rate of window events from the system (a storm), per kind.

use std::{
  sync::{
    atomic::{AtomicIsize, AtomicU64, AtomicU8, Ordering},
    Mutex, OnceLock, PoisonError,
  },
  time::{Duration, Instant},
};

use wm_platform::{window_events_received, ThreadSampler, WindowEvent};

/// A step that took longer than this is logged when it ends.
const SLOW: Duration = Duration::from_secs(1);

/// How long one step may run before the watch reports it (again at each).
const STALL_LEVELS_MS: [u64; 4] = [3_000, 10_000, 30_000, 120_000];

/// Window events per second (all kinds) from which arrivals are a storm.
const STORM_RATE: f64 = 1_500.0;

/// How often the event rate is looked at, and the least time between two
/// storm reports.
const RATE_PERIOD: Duration = Duration::from_secs(5);
const STORM_REPORT_GAP: Duration = Duration::from_secs(30);

/// Frames of the stuck thread's stack written to the log.
const STACK_FRAMES: usize = 32;

/// What the WM thread is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Activity {
  Idle = 0,
  WindowEvent = 1,
  MouseEvent = 2,
  DisplayChange = 3,
  Ipc = 4,
  WmEvent = 5,
  Cleanup = 6,
  MouseDrag = 7,
  Activation = 8,
}

impl Activity {
  fn from_u8(value: u8) -> Self {
    match value {
      1 => Self::WindowEvent,
      2 => Self::MouseEvent,
      3 => Self::DisplayChange,
      4 => Self::Ipc,
      5 => Self::WmEvent,
      6 => Self::Cleanup,
      7 => Self::MouseDrag,
      8 => Self::Activation,
      _ => Self::Idle,
    }
  }
}

static KIND: AtomicU8 = AtomicU8::new(Activity::Idle as u8);
/// For a window event: its kind (`WindowEvent::kind_index`).
static EVENT_KIND: AtomicU8 = AtomicU8::new(0);
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// When the current step started (ms since `epoch`).
static STARTED_MS: AtomicU64 = AtomicU64::new(0);
/// Increases with each step, so a report is made once per level per step.
static SEQ: AtomicU64 = AtomicU64::new(0);
/// Window events the WM thread handled (after coalescing).
static HANDLED: AtomicU64 = AtomicU64::new(0);
/// The IPC message being handled (only set for IPC steps, which are rare).
static IPC_MESSAGE: Mutex<String> = Mutex::new(String::new());

fn epoch() -> Instant {
  static EPOCH: OnceLock<Instant> = OnceLock::new();
  *EPOCH.get_or_init(Instant::now)
}

#[allow(clippy::cast_possible_truncation)]
fn now_ms() -> u64 {
  epoch().elapsed().as_millis() as u64
}

/// One step of the WM thread; ends when dropped.
pub struct Step {
  started: Instant,
}

/// Marks the start of a step of the WM thread.
pub fn begin(activity: Activity) -> Step {
  begin_with(activity, 0, 0)
}

/// Marks the start of handling a window event.
pub fn begin_window_event(event: &WindowEvent) -> Step {
  HANDLED.fetch_add(1, Ordering::Relaxed);
  let window = event.window().map_or(0, |w| window_number(w.id()));
  #[allow(clippy::cast_possible_truncation)]
  begin_with(Activity::WindowEvent, event.kind_index() as u8, window)
}

/// Marks the start of handling an IPC message.
pub fn begin_ipc(message: &str) -> Step {
  {
    let mut text = IPC_MESSAGE.lock().unwrap_or_else(PoisonError::into_inner);
    text.clear();
    text.extend(message.chars().take(160));
  }
  begin_with(Activity::Ipc, 0, 0)
}

#[allow(clippy::cast_possible_wrap)]
fn window_number(id: wm_platform::WindowId) -> isize {
  id.0 as isize
}

fn begin_with(activity: Activity, event_kind: u8, window: isize) -> Step {
  EVENT_KIND.store(event_kind, Ordering::Relaxed);
  WINDOW.store(window, Ordering::Relaxed);
  STARTED_MS.store(now_ms(), Ordering::Relaxed);
  SEQ.fetch_add(1, Ordering::Relaxed);
  KIND.store(activity as u8, Ordering::Release);
  Step { started: Instant::now() }
}

impl Drop for Step {
  fn drop(&mut self) {
    let took = self.started.elapsed();
    if took >= SLOW {
      tracing::warn!(
        "Slow step of the window manager: {} took {} ms.",
        describe_current(),
        took.as_millis()
      );
    }
    KIND.store(Activity::Idle as u8, Ordering::Release);
  }
}

/// The current step in words ("window event moved (window 0x1a2b)").
fn describe_current() -> String {
  let kind = Activity::from_u8(KIND.load(Ordering::Acquire));
  describe(
    kind,
    EVENT_KIND.load(Ordering::Relaxed),
    WINDOW.load(Ordering::Relaxed),
    &IPC_MESSAGE.lock().unwrap_or_else(PoisonError::into_inner),
  )
}

fn describe(kind: Activity, event_kind: u8, window: isize, ipc: &str) -> String {
  match kind {
    Activity::Idle => "nothing".to_string(),
    Activity::WindowEvent => format!(
      "window event {} (window {window:#x})",
      WindowEvent::KIND_NAMES
        .get(usize::from(event_kind))
        .unwrap_or(&"?")
    ),
    Activity::MouseEvent => "mouse event".to_string(),
    Activity::DisplayChange => "display change".to_string(),
    Activity::Ipc => format!("IPC message \"{ipc}\""),
    Activity::WmEvent => "WM event broadcast".to_string(),
    Activity::Cleanup => "invalid window cleanup".to_string(),
    Activity::MouseDrag => "mouse drag".to_string(),
    Activity::Activation => "pending activation".to_string(),
  }
}

/// Which report (1-based level of `STALL_LEVELS_MS`) a step running for
/// `elapsed_ms` is due, given the levels already reported for it.
fn stall_level_due(elapsed_ms: u64, reported: usize) -> Option<usize> {
  let next = STALL_LEVELS_MS.get(reported)?;
  (elapsed_ms >= *next).then_some(reported + 1)
}

/// The storm line for the events that arrived between two counts over
/// `secs`, or `None` when the rate is normal.
#[allow(clippy::cast_precision_loss)]
fn storm_summary(
  before: &[u64],
  after: &[u64],
  secs: f64,
  handled: u64,
) -> Option<String> {
  if secs <= 0.0 {
    return None;
  }
  let deltas: Vec<u64> = after
    .iter()
    .zip(before)
    .map(|(a, b)| a.saturating_sub(*b))
    .collect();
  let total: u64 = deltas.iter().sum();
  let rate = total as f64 / secs;
  if rate < STORM_RATE {
    return None;
  }
  let mut kinds: Vec<(usize, u64)> = deltas
    .iter()
    .copied()
    .enumerate()
    .filter(|(_, n)| *n > 0)
    .collect();
  kinds.sort_by(|a, b| b.1.cmp(&a.1));
  let parts: Vec<String> = kinds
    .iter()
    .map(|(i, n)| {
      format!(
        "{} {:.0}",
        WindowEvent::KIND_NAMES.get(*i).unwrap_or(&"?"),
        *n as f64 / secs
      )
    })
    .collect();
  Some(format!(
    "Window event storm: {rate:.0}/s from the system ({}), {:.0}/s handled by the window manager.",
    parts.join(", "),
    handled as f64 / secs
  ))
}

/// Starts the watch. Called on the WM thread (the thread that is watched).
pub fn start() {
  let _ = epoch();
  let sampler = ThreadSampler::for_current_thread();
  let spawned = std::thread::Builder::new()
    .name("wm-stall-watch".into())
    .spawn(move || watch(sampler.as_ref()));
  if let Err(err) = spawned {
    tracing::warn!("Failed to start the stall watch: {err}");
  }
}

#[allow(clippy::needless_pass_by_value)]
fn watch(sampler: Option<&ThreadSampler>) {
  let mut seen_seq = 0u64;
  let mut reported = 0usize;
  let mut counts = window_events_received();
  let mut handled = HANDLED.load(Ordering::Relaxed);
  let mut counted_at = Instant::now();
  let mut last_storm: Option<Instant> = None;

  loop {
    std::thread::sleep(Duration::from_millis(500));

    let kind = Activity::from_u8(KIND.load(Ordering::Acquire));
    let seq = SEQ.load(Ordering::Relaxed);
    if seq != seen_seq {
      seen_seq = seq;
      reported = 0;
    }
    if kind != Activity::Idle {
      let elapsed = now_ms().saturating_sub(STARTED_MS.load(Ordering::Relaxed));
      if let Some(level) = stall_level_due(elapsed, reported) {
        reported = level;
        report_stall(elapsed, sampler);
      }
    }

    if counted_at.elapsed() >= RATE_PERIOD {
      let now_counts = window_events_received();
      let now_handled = HANDLED.load(Ordering::Relaxed);
      let secs = counted_at.elapsed().as_secs_f64();
      if let Some(line) = storm_summary(
        &counts,
        &now_counts,
        secs,
        now_handled.saturating_sub(handled),
      ) {
        if last_storm.is_none_or(|at| at.elapsed() >= STORM_REPORT_GAP) {
          last_storm = Some(Instant::now());
          tracing::warn!("{line}");
        }
      }
      counts = now_counts;
      handled = now_handled;
      counted_at = Instant::now();
    }
  }
}

/// Writes where the WM thread is stuck: the step, whether it spins (its
/// processor use over a moment) and its stack.
fn report_stall(elapsed_ms: u64, sampler: Option<&ThreadSampler>) {
  // The step is read before the thread is paused (its lock may be taken).
  let step = describe_current();
  let Some(sampler) = sampler else {
    tracing::warn!(
      "Window manager thread busy for {:.1} s in {step}.",
      elapsed_ms as f64 / 1000.0
    );
    return;
  };
  const MOMENT: Duration = Duration::from_millis(250);
  let cpu_before = sampler.cpu_ms();
  std::thread::sleep(MOMENT);
  let cpu_after = sampler.cpu_ms();
  let use_text = match (cpu_before, cpu_after) {
    (Some(a), Some(b)) => format!(
      "thread processor use {}% (spinning when near 100, waiting when near 0)",
      (b.saturating_sub(a) * 100 / MOMENT.as_millis() as u64).min(100)
    ),
    _ => "thread processor use unknown".to_string(),
  };
  let stack = sampler.stack(STACK_FRAMES);
  tracing::warn!(
    "Window manager thread busy for {:.1} s in {step}; {use_text}; stack: {}",
    elapsed_ms as f64 / 1000.0,
    if stack.is_empty() {
      "unavailable".to_string()
    } else {
      stack.join(" < ")
    }
  );
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_stuck_step_is_reported_once_per_level() {
    assert_eq!(stall_level_due(2_999, 0), None);
    assert_eq!(stall_level_due(3_000, 0), Some(1));
    // reported at 3 s: nothing more until 10 s
    assert_eq!(stall_level_due(9_000, 1), None);
    assert_eq!(stall_level_due(10_500, 1), Some(2));
    assert_eq!(stall_level_due(200_000, 4), None);
  }

  #[test]
  fn a_normal_event_rate_is_no_storm() {
    let before = [0u64; 9];
    let mut after = [0u64; 9];
    after[3] = 2_000; // 400 moves a second over 5 s
    assert_eq!(storm_summary(&before, &after, 5.0, 2_000), None);
  }

  #[test]
  fn a_storm_names_its_kinds_busiest_first() {
    let before = [10u64; 9];
    let mut after = [10u64; 9];
    after[3] += 40_000;
    after[7] += 5_000;
    let line = storm_summary(&before, &after, 5.0, 1_000).unwrap();
    assert!(line.starts_with("Window event storm: 9000/s"), "{line}");
    assert!(line.contains("moved 8000, title 1000"), "{line}");
    assert!(line.contains("200/s handled"), "{line}");
  }

  #[test]
  fn steps_are_described_with_their_subject() {
    assert_eq!(
      describe(Activity::WindowEvent, 3, 0x1a2b, ""),
      "window event moved (window 0x1a2b)"
    );
    assert_eq!(
      describe(Activity::Ipc, 0, 0, "query monitors"),
      "IPC message \"query monitors\""
    );
  }
}
