//! Input the hooks send back to Windows (the dummy key that keeps Start and
//! menu bars shut, a Win release, a held click given back to Explorer).
//!
//! Injected from inside a low-level hook procedure, an event has to pass the
//! hook chain again while the procedure that sent it still runs: the call
//! waited for that, and logs showed the hook taking 300-900 ms on the dummy
//! key 0xE8. The procedures now only queue what to send; an injector thread
//! sends it right after they return, in the order it was queued (a Win
//! release still follows its dummy key). Single producer (the hook thread),
//! single consumer (the injector), fixed size, no allocation.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
  /// the unassigned key 0xE8 down and up
  Dummy,
  /// a key release, extended or not (Win, Alt)
  Release(u8, bool),
  /// a left press at this screen point
  LeftDown(i32, i32),
  /// the pointer moved to this screen point
  MoveTo(i32, i32),
}

pub const CAPACITY: usize = 64;

pub struct Queue {
  slots: [UnsafeCell<Op>; CAPACITY],
  /// next slot to write (producer only)
  head: AtomicUsize,
  /// next slot to read (consumer only)
  tail: AtomicUsize,
}

// one producer and one consumer, each touching only its own index's slots
unsafe impl Sync for Queue {}

impl Queue {
  pub const fn new() -> Queue {
    Queue { slots: [const { UnsafeCell::new(Op::Dummy) }; CAPACITY], head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
  }

  /// Producer side. False when full (the op is dropped; 64 pending
  /// injections never happen in practice).
  pub fn push(&self, op: Op) -> bool {
    let head = self.head.load(Ordering::Relaxed);
    let tail = self.tail.load(Ordering::Acquire);
    if head.wrapping_sub(tail) >= CAPACITY {
      return false;
    }
    unsafe { *self.slots[head % CAPACITY].get() = op };
    self.head.store(head.wrapping_add(1), Ordering::Release);
    true
  }

  /// Consumer side.
  pub fn pop(&self) -> Option<Op> {
    let tail = self.tail.load(Ordering::Relaxed);
    let head = self.head.load(Ordering::Acquire);
    if tail == head {
      return None;
    }
    let op = unsafe { *self.slots[tail % CAPACITY].get() };
    self.tail.store(tail.wrapping_add(1), Ordering::Release);
    Some(op)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ops_come_out_in_the_order_they_went_in() {
    let q = Queue::new();
    assert!(q.push(Op::Dummy));
    assert!(q.push(Op::Release(0x5B, true)));
    assert!(q.push(Op::LeftDown(10, -5)));
    assert_eq!(q.pop(), Some(Op::Dummy));
    assert_eq!(q.pop(), Some(Op::Release(0x5B, true)));
    assert_eq!(q.pop(), Some(Op::LeftDown(10, -5)));
    assert_eq!(q.pop(), None);
  }

  #[test]
  fn a_full_queue_refuses_and_wraps_after_draining() {
    let q = Queue::new();
    for i in 0..CAPACITY {
      assert!(q.push(Op::Release(i as u8, false)));
    }
    assert!(!q.push(Op::Dummy));
    for i in 0..CAPACITY {
      assert_eq!(q.pop(), Some(Op::Release(i as u8, false)));
    }
    // indexes past the end wrap around
    for round in 0..3 {
      assert!(q.push(Op::Dummy));
      assert!(q.push(Op::Release(round, false)));
      assert_eq!(q.pop(), Some(Op::Dummy));
      assert_eq!(q.pop(), Some(Op::Release(round, false)));
    }
  }

  #[test]
  fn a_producer_and_a_consumer_thread_keep_the_order() {
    static Q: Queue = Queue::new();
    let producer = std::thread::spawn(|| {
      let mut sent = 0u32;
      while sent < 10_000 {
        if Q.push(Op::LeftDown(sent as i32, 0)) {
          sent += 1;
        }
      }
    });
    let mut next = 0i32;
    while next < 10_000 {
      if let Some(Op::LeftDown(x, _)) = Q.pop() {
        assert_eq!(x, next);
        next += 1;
      }
    }
    producer.join().unwrap();
  }
}
