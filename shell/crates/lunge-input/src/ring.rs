//! A single-producer, single-consumer ring of small records: the hook thread
//! produces, the core's input thread consumes. No locks, no allocation.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};

use crate::{ffi, kind};

/// One input record for the core (24 bytes, the same layout as the core's
/// `NativeInput.Ev`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ev {
  pub kind: u16,
  pub vk: u16,
  pub mods: u16,
  pub flag: u16,
  pub id: i32,
  pub x: i32,
  pub y: i32,
  pub time: u32,
}

const SIZE: u32 = 1024; // a power of two

struct Slots(UnsafeCell<[Ev; SIZE as usize]>);
// One producer writes a slot before publishing it with `HEAD` (Release); the
// one consumer reads it after seeing `HEAD` (Acquire) and frees it with
// `TAIL`. A slot is never written and read at the same time.
unsafe impl Sync for Slots {}

static SLOTS: Slots = Slots(UnsafeCell::new([Ev { kind: 0, vk: 0, mods: 0, flag: 0, id: 0, x: 0, y: 0, time: 0 }; SIZE as usize]));
static HEAD: AtomicU32 = AtomicU32::new(0); // next slot to write (producer)
static TAIL: AtomicU32 = AtomicU32::new(0); // next slot to read (consumer)
static DROPPED: AtomicU32 = AtomicU32::new(0);
static EVENT: AtomicIsize = AtomicIsize::new(0);

pub(crate) fn set_event(handle: isize) {
  EVENT.store(handle, Ordering::Release);
}

pub fn event() -> isize {
  EVENT.load(Ordering::Acquire)
}

fn put(ev: Ev) -> bool {
  let head = HEAD.load(Ordering::Relaxed);
  let tail = TAIL.load(Ordering::Acquire);
  if head.wrapping_sub(tail) >= SIZE {
    return false;
  }
  unsafe { (*SLOTS.0.get())[(head % SIZE) as usize] = ev };
  HEAD.store(head.wrapping_add(1), Ordering::Release);
  true
}

/// Hook thread only. A full ring (the core stopped reading) drops records
/// and reports how many once there is room again.
pub fn push(ev: Ev) {
  let lost = DROPPED.load(Ordering::Relaxed);
  if lost > 0 && put(Ev { kind: kind::DROPPED, id: lost as i32, ..Ev::default() }) {
    DROPPED.store(0, Ordering::Relaxed);
  }
  if !put(ev) {
    DROPPED.store(DROPPED.load(Ordering::Relaxed).saturating_add(1), Ordering::Relaxed);
    return;
  }
  ffi::signal(event());
}

/// Core thread only.
pub fn pop() -> Option<Ev> {
  let tail = TAIL.load(Ordering::Relaxed);
  let head = HEAD.load(Ordering::Acquire);
  if tail == head {
    return None;
  }
  let ev = unsafe { (*SLOTS.0.get())[(tail % SIZE) as usize] };
  TAIL.store(tail.wrapping_add(1), Ordering::Release);
  if ev.kind == kind::MOVE {
    crate::mouse::move_taken();
  }
  Some(ev)
}

/// The address and size of the ring's storage (locked in memory).
pub(crate) fn storage() -> (*const u8, usize) {
  (SLOTS.0.get() as *const u8, std::mem::size_of::<[Ev; SIZE as usize]>())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn keeps_order_and_reports_losses() {
    // one test touches the global ring (tests run in parallel otherwise)
    while pop().is_some() {}
    for i in 0..5 {
      push(Ev { kind: kind::BIND, id: i, ..Ev::default() });
    }
    for i in 0..5 {
      assert_eq!(pop().map(|e| e.id), Some(i));
    }
    assert_eq!(pop(), None);
    for i in 0..(SIZE + 3) {
      push(Ev { kind: kind::BIND, id: i as i32, ..Ev::default() });
    }
    let mut n = 0;
    while pop().is_some() {
      n += 1;
    }
    assert_eq!(n, SIZE);
    push(Ev { kind: kind::BIND, id: 7, ..Ev::default() });
    assert_eq!(pop().map(|e| (e.kind, e.id)), Some((kind::DROPPED, 3)));
    assert_eq!(pop().map(|e| e.id), Some(7));
  }
}
