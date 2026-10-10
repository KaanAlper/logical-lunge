//! The mouse hook's decision. A port of the core's managed
//! `MouseFocus.HookInner`: the desktop's right click opens the bar's menu,
//! its double click (or single click, with Windows' "single-click to open")
//! opens icons our way, every other press closes the shell's open menus, and
//! pointer moves feed focus-follows-mouse.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::sys::Sys;
use crate::{flag, kind, Ev};

const WM_MOUSEMOVE: u32 = 0x200;
const WM_LBUTTONDOWN: u32 = 0x201;
const WM_LBUTTONUP: u32 = 0x202;
const WM_RBUTTONDOWN: u32 = 0x204;
const WM_RBUTTONUP: u32 = 0x205;
const WM_MBUTTONDOWN: u32 = 0x207;

/// The last real pointer position (x in the high half, y in the low).
static LAST_POS: AtomicU64 = AtomicU64::new(pack(i32::MIN, i32::MIN));
/// A MOVE record waits in the ring: a 1000 Hz mouse sends one per packet,
/// the core only needs the latest position, so one record stands for all.
static MOVE_PENDING: AtomicBool = AtomicBool::new(false);

const fn pack(x: i32, y: i32) -> u64 {
  (x as u32 as u64) << 32 | y as u32 as u64
}

pub fn last_pos() -> (i32, i32) {
  let p = LAST_POS.load(Ordering::Acquire);
  ((p >> 32) as u32 as i32, p as u32 as i32)
}

pub fn move_taken() {
  MOVE_PENDING.store(false, Ordering::Release);
}

#[derive(Default)]
pub struct MouseState {
  desktop_right: bool,
  left_desk: bool,
  swallow_left_up: bool,
  left_time: u32,
  left_x: i32,
  left_y: i32,
  held_down: bool,
  held_x: i32,
  held_y: i32,
}

impl MouseState {
  pub const fn new() -> Self {
    MouseState {
      desktop_right: false,
      left_desk: false,
      swallow_left_up: false,
      left_time: 0,
      left_x: 0,
      left_y: 0,
      held_down: false,
      held_x: 0,
      held_y: 0,
    }
  }
}

fn at(kind: u16, x: i32, y: i32) -> Ev {
  Ev { kind, x, y, ..Ev::default() }
}

/// true: the event is swallowed. `injected`: LLMHF_INJECTED.
pub fn decide<S: Sys>(sys: &S, st: &mut MouseState, msg: u32, x: i32, y: i32, time: u32, injected: bool) -> bool {
  match msg {
    WM_RBUTTONDOWN => {
      // the desktop's right click: Explorer never sees it, the bar's menu
      // opens on the release (as Windows' does)
      st.desktop_right = sys.flag(flag::SHELL_UP) && sys.desktop_at(x, y);
      if st.desktop_right {
        return true;
      }
      sys.push(at(kind::CLICK, x, y));
      false
    }
    WM_RBUTTONUP => {
      if !st.desktop_right {
        return false;
      }
      st.desktop_right = false;
      sys.push(at(kind::DESK_MENU, x, y));
      true
    }
    WM_MOUSEMOVE => {
      // a drag started: Explorer gets the held press back. It is sent after
      // the hook returns, so this move is swallowed and sent again behind it
      let replayed = st.held_down && !injected && sys.dragged(st.held_x, st.held_y, x, y);
      if replayed {
        st.held_down = false;
        sys.replay_left_down(st.held_x, st.held_y, x, y);
      }
      if !injected && last_pos() != (x, y) {
        LAST_POS.store(pack(x, y), Ordering::Release);
        if !MOVE_PENDING.swap(true, Ordering::AcqRel) {
          sys.push(at(kind::MOVE, x, y));
        }
      }
      replayed
    }
    WM_LBUTTONUP if st.swallow_left_up => {
      // the release of a double click's swallowed press
      st.swallow_left_up = false;
      true
    }
    WM_LBUTTONUP if st.held_down => {
      // single-click open: the held press was released without a drag
      st.held_down = false;
      sys.push(at(kind::DESK_CLICK, x, y));
      true
    }
    WM_LBUTTONDOWN | WM_MBUTTONDOWN => {
      if msg == WM_LBUTTONDOWN && !injected {
        let desk = sys.flag(flag::SHELL_UP) && sys.desktop_at(x, y) && !sys.edit_at(x, y);
        if desk && sys.flag(flag::SINGLE_CLICK_OPEN) {
          st.held_down = true;
          st.held_x = x;
          st.held_y = y;
          st.left_desk = false;
          return true;
        }
        if desk && st.left_desk && sys.double_click(st.left_time, st.left_x, st.left_y, time, x, y) {
          st.left_desk = false;
          st.swallow_left_up = true;
          sys.push(at(kind::DESK_OPEN, x, y));
          return true;
        }
        st.left_desk = desk;
        st.left_time = time;
        st.left_x = x;
        st.left_y = y;
      }
      sys.push(at(kind::CLICK, x, y));
      false
    }
    _ => false,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::sys::fake::Fake;

  #[test]
  fn desktop_right_click_opens_the_bar_menu_on_release() {
    let f = Fake::new();
    f.desktop_point.set(true);
    let mut st = MouseState::new();
    assert!(decide(&f, &mut st, WM_RBUTTONDOWN, 5, 5, 0, false));
    assert!(decide(&f, &mut st, WM_RBUTTONUP, 5, 5, 0, false));
    assert_eq!(f.kinds(), vec![kind::DESK_MENU]);
  }

  #[test]
  fn right_click_elsewhere_is_an_outside_click() {
    let f = Fake::new();
    let mut st = MouseState::new();
    assert!(!decide(&f, &mut st, WM_RBUTTONDOWN, 5, 5, 0, false));
    assert!(!decide(&f, &mut st, WM_RBUTTONUP, 5, 5, 0, false));
    assert_eq!(f.kinds(), vec![kind::CLICK]);
  }

  #[test]
  fn desktop_double_click_opens_and_swallows_the_second_click() {
    let f = Fake::new();
    f.desktop_point.set(true);
    let mut st = MouseState::new();
    assert!(!decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 100, false));
    assert!(!decide(&f, &mut st, WM_LBUTTONUP, 5, 5, 150, false));
    assert!(decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 300, false));
    assert!(decide(&f, &mut st, WM_LBUTTONUP, 5, 5, 350, false));
    assert_eq!(f.kinds(), vec![kind::CLICK, kind::DESK_OPEN]);
  }

  #[test]
  fn single_click_open_holds_the_press_and_gives_it_back_on_a_drag() {
    let f = Fake::new();
    f.desktop_point.set(true);
    f.set_flag(flag::SINGLE_CLICK_OPEN, true);
    let mut st = MouseState::new();
    assert!(decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 0, false));
    assert!(decide(&f, &mut st, WM_LBUTTONUP, 5, 5, 0, false));
    assert!(decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 0, false));
    // the move is swallowed: the press and then the move are sent again
    assert!(decide(&f, &mut st, WM_MOUSEMOVE, 50, 5, 0, false));
    assert!(!decide(&f, &mut st, WM_LBUTTONUP, 50, 5, 0, false));
    assert_eq!(f.injected.borrow().as_slice(), ["left 5,5 to 50,5"]);
    assert_eq!(f.kinds()[0], kind::DESK_CLICK);
  }

  #[test]
  fn moves_are_coalesced_until_the_core_takes_one() {
    let f = Fake::new();
    let mut st = MouseState::new();
    move_taken();
    decide(&f, &mut st, WM_MOUSEMOVE, 1, 2, 0, false);
    decide(&f, &mut st, WM_MOUSEMOVE, 3, 4, 0, false);
    decide(&f, &mut st, WM_MOUSEMOVE, 9, 9, 0, true); // injected: ignored
    assert_eq!(f.kinds(), vec![kind::MOVE]);
    assert_eq!(last_pos(), (3, 4));
    move_taken();
    decide(&f, &mut st, WM_MOUSEMOVE, 5, 6, 0, false);
    assert_eq!(f.kinds(), vec![kind::MOVE, kind::MOVE]);
  }
}
