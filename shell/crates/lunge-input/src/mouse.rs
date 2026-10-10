//! The mouse hook's decision. A port of the core's managed
//! `MouseFocus.HookInner`: the desktop's right click opens the bar's menu,
//! its double click (or single click, with Windows' "single-click to open")
//! opens icons our way, every other press closes the shell's open menus, and
//! pointer moves feed focus-follows-mouse. With Super held, the buttons and
//! the wheel are Hyprland's mouse binds (ii): the window manager moves or
//! resizes the window, the wheel switches workspaces; Windows sees none of
//! these presses.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::sys::{Sys, ALT, CTRL, SHIFT, VK_CONTROL, VK_MENU, VK_SHIFT};
use crate::{flag, kind, Ev};

const WM_MOUSEMOVE: u32 = 0x200;
const WM_LBUTTONDOWN: u32 = 0x201;
const WM_LBUTTONUP: u32 = 0x202;
const WM_RBUTTONDOWN: u32 = 0x204;
const WM_RBUTTONUP: u32 = 0x205;
const WM_MBUTTONDOWN: u32 = 0x207;
const WM_MBUTTONUP: u32 = 0x208;
const WM_MOUSEWHEEL: u32 = 0x20A;
const WM_XBUTTONDOWN: u32 = 0x20B;
const WM_XBUTTONUP: u32 = 0x20C;
/// One wheel notch; a high-resolution wheel sends parts of it.
const WHEEL_DELTA: i32 = 120;

/// Super + button: what the press started (`MouseState::super_button`).
const LEFT: u8 = 1;
const RIGHT: u8 = 2;
const MIDDLE: u8 = 3;
const XBUTTON: u8 = 4;

/// The core's mark on its hook probe ("LLPR"): a one-pixel move that tells it
/// the hook is still called. The hook eats it, so neither the cursor nor a
/// game's raw input ever moves (a zero move or wheel turn is not delivered to
/// hooks at all, so the probe has to be a real move).
pub const PROBE_MARK: usize = 0x4C4C_5052;

/// The event is the core's probe: injected and carrying its mark.
pub fn is_probe(injected: bool, extra: usize) -> bool {
  injected && extra == PROBE_MARK
}

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
  /// the button of a Super + button press: its release is ours too, even
  /// when Super was let go first
  super_button: u8,
  /// wheel travel not yet a whole notch
  wheel: i32,
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
      super_button: 0,
      wheel: 0,
    }
  }
}

fn at(kind: u16, x: i32, y: i32) -> Ev {
  Ev { kind, x, y, ..Ev::default() }
}

fn mods<S: Sys>(sys: &S) -> u16 {
  (if sys.down(VK_CONTROL) { CTRL } else { 0 })
    | (if sys.down(VK_SHIFT) { SHIFT } else { 0 })
    | (if sys.down(VK_MENU) { ALT } else { 0 })
}

/// Super + a button or the wheel (Hyprland's mouse binds). `Some(swallow)`
/// when the event is one of them.
fn super_mouse<S: Sys>(sys: &S, st: &mut MouseState, msg: u32, x: i32, y: i32, data: u32) -> Option<bool> {
  if st.super_button != 0 {
    let released = match msg {
      WM_LBUTTONUP => LEFT,
      WM_RBUTTONUP => RIGHT,
      WM_MBUTTONUP => MIDDLE,
      WM_XBUTTONUP => XBUTTON,
      _ => 0,
    };
    if released == st.super_button {
      st.super_button = 0;
      if released != XBUTTON {
        sys.push(at(kind::SUPER_DRAG_END, x, y));
      }
      return Some(true);
    }
    // pointer moves go on as usual (the window manager follows the pointer)
    if msg == WM_MOUSEMOVE {
      return None;
    }
  }
  if !sys.flag(flag::SHELL_UP) || !sys.win_held() {
    return None;
  }
  match msg {
    WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN if st.super_button == 0 => {
      st.super_button = match msg {
        WM_LBUTTONDOWN => LEFT,
        WM_MBUTTONDOWN => MIDDLE,
        _ => RIGHT,
      };
      sys.win_combo();
      let id = if msg == WM_RBUTTONDOWN { 2 } else { 1 };
      sys.push(Ev { kind: kind::SUPER_DRAG, id, x, y, ..Ev::default() });
      Some(true)
    }
    WM_XBUTTONDOWN if st.super_button == 0 => {
      st.super_button = XBUTTON;
      sys.win_combo();
      let id = i32::from((data >> 16) as u16);
      sys.push(Ev { kind: kind::SUPER_XBUTTON, id, x, y, ..Ev::default() });
      Some(true)
    }
    WM_MOUSEWHEEL => {
      sys.win_combo();
      st.wheel += i32::from((data >> 16) as u16 as i16);
      while st.wheel.abs() >= WHEEL_DELTA {
        // the wheel turned towards the user (down): the next workspace
        let id = if st.wheel < 0 { 1 } else { -1 };
        st.wheel += id * WHEEL_DELTA;
        sys.push(Ev { kind: kind::SUPER_WHEEL, id, mods: mods(sys), x, y, ..Ev::default() });
      }
      Some(true)
    }
    // other presses while a Super drag is on: not Windows' either
    WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN | WM_XBUTTONDOWN => Some(true),
    _ => None,
  }
}

/// true: the event is swallowed. `injected`: LLMHF_INJECTED. `data`: the
/// event's mouseData (wheel travel, X button).
#[allow(clippy::too_many_arguments)]
pub fn decide<S: Sys>(sys: &S, st: &mut MouseState, msg: u32, x: i32, y: i32, time: u32, injected: bool, data: u32) -> bool {
  if !injected {
    if let Some(swallow) = super_mouse(sys, st, msg, x, y, data) {
      return swallow;
    }
  }
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
mod probe_tests {
  use super::*;

  #[test]
  fn only_the_injected_marked_move_is_the_probe() {
    assert!(is_probe(true, PROBE_MARK));
    assert!(!is_probe(false, PROBE_MARK)); // a device cannot carry the mark
    assert!(!is_probe(true, 0x4C4C_4B31)); // the core's other injected input (LL_MARK) passes on
    assert!(!is_probe(true, 0));
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
    assert!(decide(&f, &mut st, WM_RBUTTONDOWN, 5, 5, 0, false, 0));
    assert!(decide(&f, &mut st, WM_RBUTTONUP, 5, 5, 0, false, 0));
    assert_eq!(f.kinds(), vec![kind::DESK_MENU]);
  }

  #[test]
  fn right_click_elsewhere_is_an_outside_click() {
    let f = Fake::new();
    let mut st = MouseState::new();
    assert!(!decide(&f, &mut st, WM_RBUTTONDOWN, 5, 5, 0, false, 0));
    assert!(!decide(&f, &mut st, WM_RBUTTONUP, 5, 5, 0, false, 0));
    assert_eq!(f.kinds(), vec![kind::CLICK]);
  }

  #[test]
  fn desktop_double_click_opens_and_swallows_the_second_click() {
    let f = Fake::new();
    f.desktop_point.set(true);
    let mut st = MouseState::new();
    assert!(!decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 100, false, 0));
    assert!(!decide(&f, &mut st, WM_LBUTTONUP, 5, 5, 150, false, 0));
    assert!(decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 300, false, 0));
    assert!(decide(&f, &mut st, WM_LBUTTONUP, 5, 5, 350, false, 0));
    assert_eq!(f.kinds(), vec![kind::CLICK, kind::DESK_OPEN]);
  }

  #[test]
  fn single_click_open_holds_the_press_and_gives_it_back_on_a_drag() {
    let f = Fake::new();
    f.desktop_point.set(true);
    f.set_flag(flag::SINGLE_CLICK_OPEN, true);
    let mut st = MouseState::new();
    assert!(decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 0, false, 0));
    assert!(decide(&f, &mut st, WM_LBUTTONUP, 5, 5, 0, false, 0));
    assert!(decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 0, false, 0));
    // the move is swallowed: the press and then the move are sent again
    assert!(decide(&f, &mut st, WM_MOUSEMOVE, 50, 5, 0, false, 0));
    assert!(!decide(&f, &mut st, WM_LBUTTONUP, 50, 5, 0, false, 0));
    assert_eq!(f.injected.borrow().as_slice(), ["left 5,5 to 50,5"]);
    assert_eq!(f.kinds()[0], kind::DESK_CLICK);
  }

  #[test]
  fn super_left_drag_is_ours_from_press_to_release() {
    let f = Fake::new();
    f.win.set(true);
    let mut st = MouseState::new();
    assert!(decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 0, false, 0));
    // Super let go mid-drag: the release still ends the drag
    f.win.set(false);
    assert!(decide(&f, &mut st, WM_LBUTTONUP, 50, 5, 0, false, 0));
    let evs = f.take();
    assert_eq!(evs.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![kind::SUPER_DRAG, kind::SUPER_DRAG_END]);
    assert_eq!(evs[0].id, 1);
    assert_eq!(f.combos.get(), 1);
  }

  #[test]
  fn super_right_drag_resizes_and_no_desktop_menu_opens() {
    let f = Fake::new();
    f.win.set(true);
    f.desktop_point.set(true);
    let mut st = MouseState::new();
    assert!(decide(&f, &mut st, WM_RBUTTONDOWN, 5, 5, 0, false, 0));
    assert!(decide(&f, &mut st, WM_RBUTTONUP, 5, 5, 0, false, 0));
    let evs = f.take();
    assert_eq!(evs.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![kind::SUPER_DRAG, kind::SUPER_DRAG_END]);
    assert_eq!(evs[0].id, 2);
  }

  #[test]
  fn super_wheel_switches_one_workspace_per_notch() {
    let f = Fake::new();
    f.win.set(true);
    let mut st = MouseState::new();
    let notch = |delta: i16| (delta as u16 as u32) << 16;
    // a high-resolution wheel: two half notches make one
    assert!(decide(&f, &mut st, WM_MOUSEWHEEL, 0, 0, 0, false, notch(-60)));
    assert!(decide(&f, &mut st, WM_MOUSEWHEEL, 0, 0, 0, false, notch(-60)));
    assert!(decide(&f, &mut st, WM_MOUSEWHEEL, 0, 0, 0, false, notch(240)));
    let ids = f.take().iter().map(|e| (e.kind, e.id)).collect::<Vec<_>>();
    assert_eq!(ids, vec![(kind::SUPER_WHEEL, 1), (kind::SUPER_WHEEL, -1), (kind::SUPER_WHEEL, -1)]);
  }

  #[test]
  fn without_super_the_buttons_are_the_apps() {
    let f = Fake::new();
    let mut st = MouseState::new();
    assert!(!decide(&f, &mut st, WM_LBUTTONDOWN, 5, 5, 0, false, 0));
    assert!(!decide(&f, &mut st, WM_MOUSEWHEEL, 5, 5, 0, false, 120 << 16));
    assert_eq!(f.kinds(), vec![kind::CLICK]);
  }

  #[test]
  fn super_back_button_toggles_the_scratchpad() {
    let f = Fake::new();
    f.win.set(true);
    let mut st = MouseState::new();
    assert!(decide(&f, &mut st, WM_XBUTTONDOWN, 0, 0, 0, false, 1 << 16));
    assert!(decide(&f, &mut st, WM_XBUTTONUP, 0, 0, 0, false, 1 << 16));
    let evs = f.take();
    assert_eq!(evs.iter().map(|e| (e.kind, e.id)).collect::<Vec<_>>(), vec![(kind::SUPER_XBUTTON, 1)]);
  }

  #[test]
  fn moves_are_coalesced_until_the_core_takes_one() {
    let f = Fake::new();
    let mut st = MouseState::new();
    move_taken();
    decide(&f, &mut st, WM_MOUSEMOVE, 1, 2, 0, false, 0);
    decide(&f, &mut st, WM_MOUSEMOVE, 3, 4, 0, false, 0);
    decide(&f, &mut st, WM_MOUSEMOVE, 9, 9, 0, true, 0); // injected: ignored
    assert_eq!(f.kinds(), vec![kind::MOVE]);
    assert_eq!(last_pos(), (3, 4));
    move_taken();
    decide(&f, &mut st, WM_MOUSEMOVE, 5, 6, 0, false, 0);
    assert_eq!(f.kinds(), vec![kind::MOVE, kind::MOVE]);
  }
}
