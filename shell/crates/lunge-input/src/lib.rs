//! Logical Lunge input hooks: the core's low-level keyboard and mouse hooks
//! in native code.
//!
//! The core is a .NET program. Its hooks ran in managed code: every event
//! allocated, and a garbage collection anywhere in the core (or a page fault
//! under memory pressure, with a game filling RAM) stopped the hook thread.
//! Past Windows' LowLevelHooksTimeout the key went through unhandled, and
//! repeated timeouts made Windows remove the hook silently: Super, Alt+Tab
//! and every shortcut died until the next reinstall.
//!
//! Here the hook procedures run on a native thread at TIME_CRITICAL priority
//! that never enters managed code, so the garbage collector never stops it.
//! They allocate nothing: they only decide whether an event is swallowed,
//! from flags and a key table the core pushes in, and put a small record in a
//! lock-free ring the core drains on its own thread. All actions (running a
//! shortcut, opening a menu) stay in the core. The module image, the ring
//! and the hook thread's stack are locked in memory, so a page fault cannot
//! stall a hook either.
//!
//! The decisions mirror the core's managed hooks (Keys2.HookInner,
//! Switcher.HandleKey, MouseFocus.HookInner) exactly; the core keeps those
//! as the fallback when this library cannot be loaded.

#![allow(clippy::missing_safety_doc)]
// the hook side is Windows only; the decisions are tested on any host
#![cfg_attr(not(windows), allow(dead_code))]

mod ffi;
mod inject;
mod keys;
mod mouse;
mod ring;
mod sys;
mod table;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

pub use ring::Ev;

/// Flags the core sets (`li_set_flag`). Read by the hook thread on every
/// event, written by the core whenever its own state changes.
pub mod flag {
  /// the shell's bar is up: Win is ours (false: keys go to Windows)
  pub const SHELL_UP: u32 = 0;
  /// the Alt+Tab switcher is open (the hook sets it when it opens one; the
  /// core clears it when the switcher closes)
  pub const SWITCHER_ACTIVE: u32 = 1;
  /// the shortcut editor waits for a combination
  pub const CAPTURING: u32 = 2;
  /// the switcher's test mode: Alt is not held
  pub const SWITCHER_DEMO: u32 = 3;
  /// Windows' "single-click to open" folder option
  pub const SINGLE_CLICK_OPEN: u32 = 5;
  /// the switcher's window exists (before that Alt+Tab stays Windows')
  pub const SWITCHER_READY: u32 = 6;
  pub const COUNT: usize = 8;
}

pub(crate) static FLAGS: [AtomicI32; flag::COUNT] = [const { AtomicI32::new(0) }; flag::COUNT];

pub(crate) fn flag(id: u32) -> bool {
  FLAGS[id as usize].load(Ordering::Relaxed) != 0
}

/// GetTickCount of the last keyboard / mouse hook call (the core's hook
/// health check: Windows may remove a hook without telling anyone).
pub(crate) static LAST_KEY_TICK: AtomicU32 = AtomicU32::new(0);
pub(crate) static LAST_MOUSE_TICK: AtomicU32 = AtomicU32::new(0);
/// the core asked to forget held keys (desktop switch: their releases went
/// to the secure desktop); taken by the hook thread on its next event
pub(crate) static FORGET: AtomicBool = AtomicBool::new(false);

/// Event kinds of the ring (`Ev::kind`).
pub mod kind {
  pub const SW_OPEN: u16 = 1; // flag: reverse
  pub const SW_MOVE: u16 = 2; // x, y: step
  pub const SW_COMMIT: u16 = 3;
  pub const SW_CLOSE: u16 = 4;
  pub const SW_ALT_UP: u16 = 5; // Alt released: commit (the dummy key is already sent)
  pub const SW_LOST_ALT: u16 = 6; // a key while Alt is not down: closed
  pub const SW_FULLSCREEN: u16 = 7; // Alt+Tab left to Windows (exclusive fullscreen)
  pub const DESK_MENU_KEY: u16 = 10;
  pub const DESK_OPEN_KEY: u16 = 11;
  pub const WIN_UP_DOCK: u16 = 12;
  pub const WIN_UP_OVERVIEW: u16 = 13;
  pub const CAPTURE: u16 = 14; // mods, vk
  pub const BIND: u16 = 15; // id; flag: Win held
  pub const RESERVED: u16 = 16; // id
  pub const WM: u16 = 17; // id; flag: repeat
  pub const SLOW_KEY: u16 = 20; // id: ms, vk
  pub const SLOW_MOUSE: u16 = 21; // id: ms
  pub const DESK_MENU: u16 = 30;
  pub const CLICK: u16 = 31; // x, y
  pub const MOVE: u16 = 32; // position: li_mouse_pos
  pub const DESK_CLICK: u16 = 33;
  pub const DESK_OPEN: u16 = 34;
  pub const DROPPED: u16 = 40; // id: events lost to a full ring
}

// ---- exports (the core loads lunge_input.dll and calls these) ----

/// Interface version: the core refuses a library it does not understand.
#[no_mangle]
pub extern "C" fn li_version() -> u32 {
  1
}

/// Starts the hook thread and installs both hooks. 1: both hooks are in
/// place; 0: they are not (the core falls back to its managed hooks).
#[no_mangle]
pub extern "C" fn li_start() -> i32 {
  ffi::start_thread()
}

/// The auto-reset event signalled when the ring has records.
#[no_mangle]
pub extern "C" fn li_event() -> isize {
  ring::event()
}

/// Takes the oldest record; 1 when one was taken.
#[no_mangle]
pub unsafe extern "C" fn li_pop(out: *mut Ev) -> i32 {
  match ring::pop() {
    Some(ev) if !out.is_null() => {
      *out = ev;
      1
    }
    _ => 0,
  }
}

#[no_mangle]
pub extern "C" fn li_set_flag(id: u32, value: i32) {
  if (id as usize) < flag::COUNT {
    FLAGS[id as usize].store(value, Ordering::Relaxed);
  }
}

/// Replaces the key table: `n` packed entries (see table.rs).
#[no_mangle]
pub unsafe extern "C" fn li_set_table(entries: *const u64, n: u32) {
  let list = if entries.is_null() { &[][..] } else { std::slice::from_raw_parts(entries, n as usize) };
  table::set(list);
}

/// The virtual keys the shortcut editor can name (32 bytes, one bit each).
#[no_mangle]
pub unsafe extern "C" fn li_set_capturable(bits: *const u32) {
  if !bits.is_null() {
    table::set_capturable(std::slice::from_raw_parts(bits, 8));
  }
}

#[no_mangle]
pub extern "C" fn li_forget() {
  FORGET.store(true, Ordering::Release);
}

/// Reinstalls a hook on the hook thread (1 keyboard, 2 mouse, 3 both; +4:
/// Windows removed it, forget the held keys). A held Win keeps its keyboard
/// hook unless forced.
#[no_mangle]
pub extern "C" fn li_reinstall(which: u32) {
  ffi::request_reinstall(which);
}

#[no_mangle]
pub unsafe extern "C" fn li_ticks(key: *mut u32, mouse: *mut u32) {
  if !key.is_null() {
    *key = LAST_KEY_TICK.load(Ordering::Relaxed);
  }
  if !mouse.is_null() {
    *mouse = LAST_MOUSE_TICK.load(Ordering::Relaxed);
  }
}

/// The last real (not injected) pointer position the mouse hook saw.
#[no_mangle]
pub unsafe extern "C" fn li_mouse_pos(x: *mut i32, y: *mut i32) {
  let (px, py) = mouse::last_pos();
  if !x.is_null() {
    *x = px;
  }
  if !y.is_null() {
    *y = py;
  }
}

/// Hook timings since the last call (1 keyboard, 2 mouse): calls, total and
/// longest time inside the hook procedure, in microseconds.
#[no_mangle]
pub unsafe extern "C" fn li_stats(which: u32, calls: *mut u64, total_us: *mut u64, max_us: *mut u64) {
  #[cfg(windows)]
  let (c, t, m) = if which == 1 { ffi::KEY_STATS.take() } else { ffi::MOUSE_STATS.take() };
  #[cfg(not(windows))]
  let (c, t, m) = {
    let _ = which;
    (0, 0, 0)
  };
  for (out, v) in [(calls, c), (total_us, t), (max_us, m)] {
    if !out.is_null() {
      *out = v;
    }
  }
}
