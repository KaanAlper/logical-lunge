use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

use crate::post_message_w;
use crate::utils::WM_APP_ANIMATE;

/// A border's animation ticks: WM_APP_ANIMATE posted to the border window every `interval` until dropped.
///
/// Every timer runs on one shared thread (`border-anim`). A thread per timer was started and ended each time an
/// animation started (a fade on every focus change, every new border): hundreds of threads a minute while windows
/// open and close, each start and exit announced to every loaded DLL.
#[derive(Debug)]
pub struct AnimationTimer {
    id: u64,
}

struct Entry {
    hwnd: isize,
    interval: Duration,
    due: Instant,
}

struct Ticker {
    timers: Mutex<HashMap<u64, Entry>>,
    changed: Condvar,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn ticker() -> &'static Ticker {
    static TICKER: OnceLock<&'static Ticker> = OnceLock::new();
    TICKER.get_or_init(|| {
        let t: &'static Ticker = Box::leak(Box::new(Ticker { timers: Mutex::new(HashMap::new()), changed: Condvar::new() }));
        crate::utils::spawn_named("border-anim", move || run(t));
        t
    })
}

fn run(t: &'static Ticker) {
    loop {
        crate::wait_while_idle();
        let mut timers = t.timers.lock().unwrap_or_else(PoisonError::into_inner);
        while timers.is_empty() {
            timers = t.changed.wait(timers).unwrap_or_else(PoisonError::into_inner);
        }
        let (fire, _) = due(&mut timers, Instant::now());
        drop(timers);
        for (id, hwnd) in fire {
            if let Err(err) = post_message_w(Some(HWND(hwnd as _)), WM_APP_ANIMATE, WPARAM(0), LPARAM(0)) {
                error!("could not send animation timer message for {hwnd:?}: {err:#}");
                t.timers.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
            }
        }
        // until the next tick is due (timers added or dropped meanwhile count), or a timer is added
        let timers = t.timers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(next) = timers.values().map(|e| e.due).min() {
            let wait = next.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                let _ = t.changed.wait_timeout(timers, wait);
            }
        }
    }
}

/// The timers due at `now` (each moved to its next tick; one that fell behind starts over from `now` rather than
/// firing a burst) and when the next one is due.
fn due(timers: &mut HashMap<u64, Entry>, now: Instant) -> (Vec<(u64, isize)>, Option<Instant>) {
    let mut fire = Vec::new();
    let mut next: Option<Instant> = None;
    for (id, e) in timers.iter_mut() {
        if e.due <= now {
            fire.push((*id, e.hwnd));
            e.due += e.interval;
            if e.due <= now {
                e.due = now + e.interval;
            }
        }
        next = Some(next.map_or(e.due, |n| n.min(e.due)));
    }
    (fire, next)
}

impl AnimationTimer {
    pub fn new(hwnd: HWND, interval: Duration) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let t = ticker();
        // the first tick right away, as the thread per timer did
        t.timers.lock().unwrap_or_else(PoisonError::into_inner).insert(
            id,
            Entry { hwnd: hwnd.0 as isize, interval: interval.max(Duration::from_millis(1)), due: Instant::now() },
        );
        t.changed.notify_one();
        Self { id }
    }
}

impl Drop for AnimationTimer {
    fn drop(&mut self) {
        ticker().timers.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(hwnd: isize, ms: u64, due: Instant) -> Entry {
        Entry { hwnd, interval: Duration::from_millis(ms), due }
    }

    #[test]
    fn fires_the_due_timers_and_wakes_for_the_nearest_next_one() {
        let t0 = Instant::now();
        let mut timers = HashMap::from([(1, entry(10, 16, t0)), (2, entry(20, 33, t0 + Duration::from_millis(5)))]);
        let (fire, next) = due(&mut timers, t0);
        assert_eq!(fire, vec![(1, 10)]);
        assert_eq!(next, Some(t0 + Duration::from_millis(5)));
        assert_eq!(timers[&1].due, t0 + Duration::from_millis(16));
    }

    #[test]
    fn a_timer_that_fell_behind_ticks_once_and_starts_over() {
        let t0 = Instant::now();
        let mut timers = HashMap::from([(1, entry(10, 16, t0))]);
        let late = t0 + Duration::from_millis(100);
        let (fire, next) = due(&mut timers, late);
        assert_eq!(fire, vec![(1, 10)]);
        assert_eq!(next, Some(late + Duration::from_millis(16)));
    }

    #[test]
    fn nothing_due_fires_nothing() {
        let t0 = Instant::now();
        let mut timers = HashMap::from([(1, entry(10, 16, t0 + Duration::from_millis(8)))]);
        let (fire, next) = due(&mut timers, t0);
        assert!(fire.is_empty());
        assert_eq!(next, Some(t0 + Duration::from_millis(8)));
    }
}
