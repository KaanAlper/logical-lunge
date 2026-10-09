//! The key table the core pushes: which combinations it handles. Read by the
//! hook thread without locks (a sequence counter makes a read consistent),
//! written rarely by the core.
//!
//! An entry is packed into a u64: mods (bits 0-7: Super 1, Ctrl 2, Shift 4,
//! Alt 8), virtual key (8-15), kind (16-23), flag (24-31), id (32-63).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

/// a shortcut of the core's table (keybinds.json): id = the core's action
pub const BIND: u8 = 1;
/// a combination Windows keeps (Super+L): id = action, -1 for none
pub const RESERVED: u8 = 2;
/// a window manager binding with Super: id = its commands; flag 1 = repeats
pub const WM: u8 = 3;

const MAX: usize = 1024;

static ENTRIES: [AtomicU64; MAX] = [const { AtomicU64::new(0) }; MAX];
static COUNT: AtomicU32 = AtomicU32::new(0);
static SEQ: AtomicU32 = AtomicU32::new(0);
static WRITER: Mutex<()> = Mutex::new(());
static CAPTURABLE: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];

#[cfg(test)]
pub fn pack(mods: u8, vk: u8, kind: u8, flag: u8, id: i32) -> u64 {
  mods as u64 | (vk as u64) << 8 | (kind as u64) << 16 | (flag as u64) << 24 | (id as u32 as u64) << 32
}

pub fn set(list: &[u64]) {
  let _guard = WRITER.lock().unwrap_or_else(|e| e.into_inner());
  let n = list.len().min(MAX);
  SEQ.fetch_add(1, Ordering::AcqRel); // odd: being written
  for (slot, value) in ENTRIES.iter().zip(&list[..n]) {
    slot.store(*value, Ordering::Relaxed);
  }
  COUNT.store(n as u32, Ordering::Relaxed);
  SEQ.fetch_add(1, Ordering::AcqRel); // even: consistent again
}

/// (id, flag) of the entry for this combination, if any.
pub fn lookup(kind: u8, mods: u8, vk: u8) -> Option<(i32, u8)> {
  let key = mods as u64 | (vk as u64) << 8 | (kind as u64) << 16;
  // a write takes microseconds and happens when a config file changes: a
  // read that overlapped one simply reads again
  for _ in 0..64 {
    let before = SEQ.load(Ordering::Acquire);
    if before % 2 == 1 {
      std::hint::spin_loop();
      continue;
    }
    let n = (COUNT.load(Ordering::Relaxed) as usize).min(MAX);
    let mut found = None;
    for slot in &ENTRIES[..n] {
      let e = slot.load(Ordering::Relaxed);
      if e & 0xFF_FFFF == key {
        found = Some(((e >> 32) as u32 as i32, (e >> 24) as u8));
        break;
      }
    }
    if SEQ.load(Ordering::Acquire) == before {
      return found;
    }
  }
  None
}

pub fn set_capturable(bits: &[u32]) {
  for (slot, value) in CAPTURABLE.iter().zip(bits) {
    slot.store(*value, Ordering::Relaxed);
  }
}

pub fn capturable(vk: u32) -> bool {
  vk < 256 && CAPTURABLE[(vk / 32) as usize].load(Ordering::Relaxed) & (1 << (vk % 32)) != 0
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn finds_entries_by_kind_mods_and_key() {
    set(&[pack(1, 0x46, WM, 0, 3), pack(1, 0x4C, RESERVED, 0, -1), pack(3, 0x25, BIND, 0, 12), pack(1, 0x46, BIND, 0, 9)]);
    assert_eq!(lookup(WM, 1, 0x46), Some((3, 0)));
    assert_eq!(lookup(BIND, 1, 0x46), Some((9, 0)));
    assert_eq!(lookup(RESERVED, 1, 0x4C), Some((-1, 0)));
    assert_eq!(lookup(BIND, 3, 0x25), Some((12, 0)));
    assert_eq!(lookup(BIND, 1, 0x25), None);
    set(&[]);
    assert_eq!(lookup(BIND, 3, 0x25), None);
    set_capturable(&[0, 0, 1 << 5, 0, 0, 0, 0, 0]);
    assert!(capturable(0x45));
    assert!(!capturable(0x46));
  }
}
