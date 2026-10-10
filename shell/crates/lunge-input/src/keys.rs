//! The keyboard hook's decision: swallow the key or let it through, and what
//! to tell the core. A port of the core's managed `Keys2.HookInner` and
//! `Switcher.HandleKey`, rule for rule; the actions themselves stay in the
//! core (`NativeInput` there).

use crate::sys::{is_modifier, KeySet, Sys, ALT, CTRL, SHIFT, SUPER, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
use crate::{flag, kind, table, Ev};

const WM_KEYDOWN: u32 = 0x100;
const WM_KEYUP: u32 = 0x101;
const WM_SYSKEYDOWN: u32 = 0x104;
const WM_SYSKEYUP: u32 = 0x105;
const LLKHF_ALTDOWN: u32 = 0x20;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_APPS: u32 = 0x5D;
const VK_F10: u32 = 0x79;

/// A shortcut table entry the core always handles.
pub const BIND_ALWAYS: u8 = 1;
/// ... the "close" action: not when the shell or the desktop is in front
pub const BIND_UNLESS_SHELL: u8 = 2;

/// The Win key's state between events (hook thread only).
#[derive(Default)]
pub struct KeyState {
  win_down: bool,
  other_key_while_win: bool,
  modifier_while_win: bool,
  dock_chord: bool,
  dock_masked: bool,
  last_win_event: u32,
  /// keys whose press we swallowed: their repeats and release are ours too
  held: KeySet,
}

impl KeyState {
  pub const fn new() -> Self {
    KeyState {
      win_down: false,
      other_key_while_win: false,
      modifier_while_win: false,
      dock_chord: false,
      dock_masked: false,
      last_win_event: 0,
      held: KeySet::EMPTY,
    }
  }

  /// Releases that never reach us (the hook was removed, the desktop
  /// switched to the secure one): forget what was held.
  pub fn forget(&mut self) {
    self.win_down = false;
    self.dock_chord = false;
    self.dock_masked = false;
    self.held.clear();
  }

  pub fn win_down(&self) -> bool {
    self.win_down
  }
}

fn ev(kind: u16, vk: u32) -> Ev {
  Ev { kind, vk: vk as u16, ..Ev::default() }
}

fn mods<S: Sys>(sys: &S, win: bool) -> u16 {
  (if win { SUPER } else { 0 })
    | (if sys.down(VK_CONTROL) { CTRL } else { 0 })
    | (if sys.down(VK_SHIFT) { SHIFT } else { 0 })
    | (if sys.down(VK_MENU) { ALT } else { 0 })
}

/// Super+Alt (the dock): Alt goes to the focused app while Win is
/// swallowed; an unassigned key between its press and release keeps the
/// app's menu bar closed. Once per combination.
fn mask_alt_menu<S: Sys>(sys: &S, st: &mut KeyState) {
  if st.dock_masked {
    return;
  }
  st.dock_masked = true;
  sys.suppress_start();
}

/// true: the key is swallowed. `injected_by_us`: injected and carrying the
/// core's mark (passed through untouched).
pub fn decide<S: Sys>(sys: &S, st: &mut KeyState, msg: u32, vk: u32, flags: u32, injected_by_us: bool) -> bool {
  if injected_by_us {
    return false;
  }
  // The shell's bar is down: keys go to Windows as they are (Win opens
  // Start). A held Win or an open switcher finishes first.
  if !sys.flag(flag::SHELL_UP) && !st.win_down && !sys.flag(flag::SWITCHER_ACTIVE) {
    return false;
  }
  let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
  let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

  // Alt+Tab: our switcher; Windows' own never opens
  let alt_held = sys.down(VK_MENU) || flags & LLKHF_ALTDOWN != 0;
  if (sys.flag(flag::SWITCHER_ACTIVE) || (is_down && vk == VK_TAB && alt_held && !st.win_down))
    && switcher(sys, vk, is_down, is_up, sys.down(VK_SHIFT), alt_held, sys.down(VK_CONTROL) || st.win_down)
  {
    return true;
  }

  // The menu key / Shift+F10 with the desktop in front: the bar's menu
  if is_down && (vk == VK_APPS || (vk == VK_F10 && sys.down(VK_SHIFT))) && !st.win_down && sys.desktop_in_front() {
    if !st.held.contains(vk) {
      sys.push(ev(kind::DESK_MENU_KEY, vk));
    }
    st.held.insert(vk);
    return true;
  }

  // Enter with the desktop in front: we open the selected icons. Alt+Enter,
  // Ctrl combinations and Enter in the rename box stay Explorer's.
  if is_down
    && vk == VK_RETURN
    && !st.win_down
    && !sys.down(VK_CONTROL)
    && !sys.down(VK_MENU)
    && sys.desktop_in_front()
    && !sys.desktop_focus_is_edit()
  {
    if !st.held.contains(vk) {
      sys.push(ev(kind::DESK_OPEN_KEY, vk));
    }
    st.held.insert(vk);
    return true;
  }

  // The real Win key never reaches Windows
  if vk == VK_LWIN || vk == VK_RWIN {
    win_key(sys, st, vk, is_down, is_up);
    return true; // press, auto-repeat and release
  }

  let mod_key = is_modifier(vk);
  // The shortcut editor waits for a combination: it gets the first one
  if sys.flag(flag::CAPTURING) && is_down && !mod_key {
    let cm = mods(sys, st.win_down);
    if (vk == VK_ESCAPE && cm == 0) || sys.capturable(vk) {
      sys.set_flag(flag::CAPTURING, false);
      sys.push(Ev { kind: kind::CAPTURE, vk: vk as u16, mods: cm, ..Ev::default() });
      st.held.insert(vk);
      if st.win_down {
        st.other_key_while_win = true;
      }
      return true;
    }
  }

  if st.win_down && is_down {
    if mod_key {
      st.modifier_while_win = true;
      if (vk == VK_MENU || vk == 0xA4 || vk == 0xA5)
        && !st.other_key_while_win
        && !sys.down(VK_CONTROL)
        && !sys.down(VK_SHIFT)
      {
        st.dock_chord = true;
        mask_alt_menu(sys, st);
      }
      if matches!(vk, VK_CONTROL | VK_SHIFT | 0xA0..=0xA3) {
        st.dock_chord = false;
      }
    } else {
      st.other_key_while_win = true;
      st.dock_chord = false;
    }
  }

  // The core's shortcut table (keybinds.json)
  if is_down && !mod_key {
    let m = mods(sys, st.win_down);
    if let Some((id, how)) = sys.lookup(table::BIND, m, vk) {
      // holding a shortcut runs it once per press
      if st.held.contains(vk) {
        return true;
      }
      if how == BIND_ALWAYS || (how == BIND_UNLESS_SHELL && !sys.foreground_is_shell()) {
        st.held.insert(vk);
        sys.push(Ev { kind: kind::BIND, vk: vk as u16, mods: m, flag: st.win_down as u16, id, ..Ev::default() });
        return true;
      }
    }
  }
  if is_up && st.held.remove(vk) {
    return true;
  }
  // The window manager's bindings without Super (Ctrl+Alt+T, a binding
  // mode's keys): the window manager has no keyboard hook of its own
  if !st.win_down && is_down && !mod_key {
    if let Some((id, repeats)) = sys.lookup(table::WM, mods(sys, false), vk) {
      let repeat = !st.held.insert(vk);
      if !repeat || repeats == 1 {
        sys.push(Ev { kind: kind::WM, vk: vk as u16, mods: mods(sys, false), flag: repeat as u16, id, ..Ev::default() });
      }
      return true;
    }
  }
  if st.win_down && is_down && !mod_key {
    let m = SUPER | mods(sys, false);
    let repeat = !st.held.insert(vk);
    // Windows' own lock (Super+L) and the window manager's Super bindings
    if let Some((id, _)) = sys.lookup(table::RESERVED, m, vk) {
      if id >= 0 && !repeat {
        sys.push(Ev { kind: kind::RESERVED, vk: vk as u16, mods: m, id, ..Ev::default() });
      }
      return true;
    }
    let wm = sys.lookup(table::WM, m, vk);
    // only the bindings that repeat (split ratio) run again while held
    if repeat && !matches!(wm, Some((_, 1))) {
      return true;
    }
    if let Some((id, _)) = wm {
      sys.push(Ev { kind: kind::WM, vk: vk as u16, mods: m, flag: repeat as u16, id, ..Ev::default() });
    }
    return true; // in no table: Windows does not get it either
  }
  false
}

fn win_key<S: Sys>(sys: &S, st: &mut KeyState, vk: u32, is_down: bool, is_up: bool) {
  // A release may never come (Win+L): auto-repeat comes every ~30 ms, a
  // press after a long gap is a new press
  let now = sys.now_ms();
  let fresh = !st.win_down || now.wrapping_sub(st.last_win_event) as i32 > 700;
  st.last_win_event = now;
  if is_down && fresh {
    st.win_down = true;
    st.other_key_while_win = false;
    // Ctrl/Shift/Alt held before Win count as a combination too
    st.modifier_while_win = sys.down(VK_CONTROL) || sys.down(VK_SHIFT) || sys.down(VK_MENU);
    st.dock_chord = sys.down(VK_MENU) && !sys.down(VK_CONTROL) && !sys.down(VK_SHIFT);
    st.dock_masked = false;
    if st.dock_chord {
      mask_alt_menu(sys, st);
    }
    sys.push(ev(kind::WIN_DOWN, vk));
  }
  if is_up {
    sys.push(ev(kind::WIN_UP, vk));
    let capturing = sys.flag(flag::CAPTURING);
    let toggle_dock = st.dock_chord && !st.other_key_while_win && !capturing;
    st.win_down = false;
    st.dock_chord = false;
    st.dock_masked = false;
    // the press reached Windows (a late hook): release it behind a dummy key
    if sys.down(vk) {
      sys.suppress_start();
      sys.release_key(vk, true);
    }
    if toggle_dock {
      sys.push(ev(kind::WIN_UP_DOCK, vk));
    } else if !st.other_key_while_win && !st.modifier_while_win && !capturing {
      sys.push(ev(kind::WIN_UP_OVERVIEW, vk));
    }
  }
}

/// The Alt+Tab switcher (`Switcher.HandleKey`): true swallows the key.
fn switcher<S: Sys>(sys: &S, vk: u32, is_down: bool, is_up: bool, shift: bool, alt_down: bool, win_or_ctrl: bool) -> bool {
  if !sys.flag(flag::SWITCHER_READY) {
    return false;
  }
  if !sys.flag(flag::SWITCHER_ACTIVE) {
    if is_down && vk == VK_TAB && alt_down && !win_or_ctrl {
      // an exclusive fullscreen game in front: Windows' Alt+Tab minimises
      // it its own way (ours would open behind the game)
      if sys.exclusive_fullscreen() {
        sys.push(ev(kind::SW_FULLSCREEN, vk));
        return false;
      }
      sys.set_flag(flag::SWITCHER_ACTIVE, true);
      sys.push(Ev { kind: kind::SW_OPEN, vk: vk as u16, flag: shift as u16, ..Ev::default() });
      return true;
    }
    return false;
  }
  // Alt released: open the selection. A dummy key keeps the app's menu bar
  // from activating; it is sent after the hook returns, so the release is
  // swallowed here and sent again behind it (right Alt is an extended key)
  if is_up && (vk == VK_MENU || vk == 0xA4 || vk == 0xA5) {
    sys.suppress_start();
    sys.release_key(vk, vk == 0xA5);
    sys.push(ev(kind::SW_ALT_UP, vk));
    return true;
  }
  // a missed Alt release: never stay locked open
  if !alt_down && !sys.flag(flag::SWITCHER_DEMO) {
    sys.set_flag(flag::SWITCHER_ACTIVE, false);
    sys.push(ev(kind::SW_LOST_ALT, vk));
    return false;
  }
  if is_down {
    let step = |x: i32, y: i32| Ev { kind: kind::SW_MOVE, vk: vk as u16, x, y, ..Ev::default() };
    match vk {
      VK_TAB => sys.push(step(if shift { -1 } else { 1 }, 0)),
      0x27 => sys.push(step(1, 0)),
      0x25 => sys.push(step(-1, 0)),
      0x28 => sys.push(step(0, 1)),
      0x26 => sys.push(step(0, -1)),
      VK_RETURN => sys.push(ev(kind::SW_COMMIT, vk)),
      VK_ESCAPE => sys.push(ev(kind::SW_CLOSE, vk)),
      _ => {}
    }
    return true; // no key reaches the app while it is open
  }
  is_up && matches!(vk, VK_TAB | 0x25..=0x28 | VK_RETURN | VK_ESCAPE)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::sys::fake::Fake;

  fn press(f: &Fake, st: &mut KeyState, vk: u32) -> bool {
    f.down.borrow_mut().insert(vk);
    decide(f, st, WM_KEYDOWN, vk, 0, false)
  }

  fn release(f: &Fake, st: &mut KeyState, vk: u32) -> bool {
    f.down.borrow_mut().remove(&vk);
    decide(f, st, WM_KEYUP, vk, 0, false)
  }

  #[test]
  fn lone_win_opens_the_overview_and_never_reaches_windows() {
    let f = Fake::new();
    let mut st = KeyState::new();
    assert!(press(&f, &mut st, VK_LWIN));
    assert!(release(&f, &mut st, VK_LWIN));
    assert_eq!(f.kinds(), vec![kind::WIN_UP_OVERVIEW]);
  }

  #[test]
  fn super_press_and_release_are_announced_once() {
    let f = Fake::new();
    let mut st = KeyState::new();
    press(&f, &mut st, VK_LWIN);
    press(&f, &mut st, VK_LWIN); // auto-repeat
    release(&f, &mut st, VK_LWIN);
    assert_eq!(f.all_kinds(), vec![kind::WIN_DOWN, kind::WIN_UP, kind::WIN_UP_OVERVIEW]);
  }

  #[test]
  fn win_combination_runs_the_wm_binding_once_per_press() {
    let f = Fake::new();
    f.table.borrow_mut().insert((table::WM, SUPER, 0x46), (5, 0));
    let mut st = KeyState::new();
    press(&f, &mut st, VK_LWIN);
    assert!(press(&f, &mut st, 0x46));
    assert!(decide(&f, &mut st, WM_KEYDOWN, 0x46, 0, false)); // auto-repeat
    assert!(release(&f, &mut st, 0x46));
    release(&f, &mut st, VK_LWIN);
    let e = f.take();
    assert_eq!(e.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![kind::WM]);
    assert_eq!(e[0].id, 5);
  }

  #[test]
  fn repeating_wm_binding_repeats_while_held() {
    let f = Fake::new();
    f.table.borrow_mut().insert((table::WM, SUPER, 0x4C), (2, 1));
    let mut st = KeyState::new();
    press(&f, &mut st, VK_LWIN);
    press(&f, &mut st, 0x4C);
    decide(&f, &mut st, WM_KEYDOWN, 0x4C, 0, false);
    assert_eq!(f.take().iter().map(|e| (e.kind, e.flag)).collect::<Vec<_>>(), vec![(kind::WM, 0), (kind::WM, 1)]);
  }

  #[test]
  fn reserved_lock_runs_once_and_unknown_super_keys_are_swallowed() {
    let f = Fake::new();
    f.table.borrow_mut().insert((table::RESERVED, SUPER, 0x4C), (7, 0));
    let mut st = KeyState::new();
    press(&f, &mut st, VK_LWIN);
    assert!(press(&f, &mut st, 0x4C));
    assert!(decide(&f, &mut st, WM_KEYDOWN, 0x4C, 0, false));
    assert!(press(&f, &mut st, 0x51)); // Super+Q: in no table
    assert_eq!(f.kinds(), vec![kind::RESERVED]);
  }

  #[test]
  fn bind_table_entries_and_the_close_rule() {
    let f = Fake::new();
    f.table.borrow_mut().insert((table::BIND, SUPER, 0x0D), (1, BIND_ALWAYS));
    f.table.borrow_mut().insert((table::BIND, SUPER, 0x51), (2, BIND_UNLESS_SHELL));
    let mut st = KeyState::new();
    press(&f, &mut st, VK_LWIN);
    assert!(press(&f, &mut st, 0x0D));
    assert!(release(&f, &mut st, 0x0D));
    f.fg_shell.set(true);
    // close with the shell in front: not ours, but Win+Q still never reaches Windows
    assert!(press(&f, &mut st, 0x51));
    let e = f.take();
    assert_eq!(e.iter().map(|e| (e.kind, e.id, e.flag)).collect::<Vec<_>>(), vec![(kind::BIND, 1, 1)]);
  }

  #[test]
  fn wm_binding_without_win_is_ours_and_its_release_too() {
    let f = Fake::new();
    f.table.borrow_mut().insert((table::WM, CTRL | ALT, 0x54), (4, 0));
    let mut st = KeyState::new();
    press(&f, &mut st, VK_CONTROL);
    press(&f, &mut st, VK_MENU);
    assert!(press(&f, &mut st, 0x54));
    assert!(decide(&f, &mut st, WM_KEYDOWN, 0x54, 0, false)); // repeat: swallowed, not run again
    assert!(release(&f, &mut st, 0x54));
    assert_eq!(f.take().iter().map(|e| (e.kind, e.id)).collect::<Vec<_>>(), vec![(kind::WM, 4)]);
  }

  #[test]
  fn shortcut_without_win_passes_when_not_in_the_table() {
    let f = Fake::new();
    let mut st = KeyState::new();
    assert!(!press(&f, &mut st, 0x41));
    assert!(!release(&f, &mut st, 0x41));
    assert!(f.kinds().is_empty());
  }

  #[test]
  fn shell_down_leaves_keys_to_windows() {
    let f = Fake::new();
    f.set_flag(flag::SHELL_UP, false);
    let mut st = KeyState::new();
    assert!(!press(&f, &mut st, VK_LWIN));
    assert!(!decide(&f, &mut st, WM_KEYDOWN, 0x41, 0, true));
  }

  #[test]
  fn super_alt_toggles_the_dock_and_masks_the_menu_bar() {
    let f = Fake::new();
    let mut st = KeyState::new();
    press(&f, &mut st, VK_LWIN);
    press(&f, &mut st, VK_MENU);
    release(&f, &mut st, VK_MENU);
    release(&f, &mut st, VK_LWIN);
    assert_eq!(f.kinds(), vec![kind::WIN_UP_DOCK]);
    assert_eq!(f.injected.borrow().as_slice(), ["dummy"]);
  }

  #[test]
  fn win_still_down_in_windows_is_released_behind_a_dummy() {
    let f = Fake::new();
    let mut st = KeyState::new();
    press(&f, &mut st, VK_LWIN);
    // Windows still sees Win down when the release arrives
    assert!(decide(&f, &mut st, WM_KEYUP, VK_LWIN, 0, false));
    assert_eq!(f.injected.borrow().as_slice(), ["dummy", "up 5B ext"]);
  }

  #[test]
  fn alt_tab_opens_moves_and_commits_on_alt_release() {
    let f = Fake::new();
    let mut st = KeyState::new();
    press(&f, &mut st, VK_MENU);
    assert!(press(&f, &mut st, VK_TAB));
    assert!(release(&f, &mut st, VK_TAB));
    assert!(press(&f, &mut st, VK_TAB));
    assert!(press(&f, &mut st, 0x41)); // any key: swallowed while open
    // the release is swallowed and sent again behind the dummy key
    assert!(release(&f, &mut st, VK_MENU));
    assert_eq!(f.kinds(), vec![kind::SW_OPEN, kind::SW_MOVE, kind::SW_ALT_UP]);
    assert_eq!(f.injected.borrow().as_slice(), ["dummy", "up 12"]);
  }

  #[test]
  fn right_alt_is_released_as_an_extended_key() {
    let f = Fake::new();
    f.set_flag(flag::SWITCHER_ACTIVE, true);
    let mut st = KeyState::new();
    f.down.borrow_mut().insert(VK_MENU);
    assert!(decide(&f, &mut st, WM_KEYUP, 0xA5, 0, false));
    assert_eq!(f.injected.borrow().as_slice(), ["dummy", "up A5 ext"]);
  }

  #[test]
  fn alt_tab_over_exclusive_fullscreen_stays_windows() {
    let f = Fake::new();
    f.fullscreen.set(true);
    let mut st = KeyState::new();
    press(&f, &mut st, VK_MENU);
    assert!(!press(&f, &mut st, VK_TAB));
    assert!(!f.flag(flag::SWITCHER_ACTIVE));
  }

  #[test]
  fn switcher_closes_when_alt_is_seen_up() {
    let f = Fake::new();
    f.set_flag(flag::SWITCHER_ACTIVE, true);
    let mut st = KeyState::new();
    assert!(!press(&f, &mut st, 0x41));
    assert!(!f.flag(flag::SWITCHER_ACTIVE));
    assert_eq!(f.kinds(), vec![kind::SW_LOST_ALT]);
  }

  #[test]
  fn desktop_keys_menu_and_enter() {
    let f = Fake::new();
    f.desktop_front.set(true);
    let mut st = KeyState::new();
    assert!(press(&f, &mut st, VK_APPS));
    assert!(decide(&f, &mut st, WM_KEYDOWN, VK_APPS, 0, false));
    assert!(release(&f, &mut st, VK_APPS));
    assert!(press(&f, &mut st, VK_RETURN));
    assert!(release(&f, &mut st, VK_RETURN));
    f.focus_edit.set(true);
    assert!(!press(&f, &mut st, VK_RETURN));
    assert_eq!(f.kinds(), vec![kind::DESK_MENU_KEY, kind::DESK_OPEN_KEY]);
  }

  #[test]
  fn capture_takes_the_first_combination() {
    let f = Fake::new();
    f.set_flag(flag::CAPTURING, true);
    let mut st = KeyState::new();
    press(&f, &mut st, VK_CONTROL);
    assert!(press(&f, &mut st, 0x4B));
    assert!(!f.flag(flag::CAPTURING));
    let e = f.take();
    assert_eq!((e[0].kind, e[0].mods, e[0].vk), (kind::CAPTURE, CTRL, 0x4B));
  }

  #[test]
  fn forget_drops_held_keys() {
    let f = Fake::new();
    let mut st = KeyState::new();
    press(&f, &mut st, VK_LWIN);
    st.forget();
    assert!(!st.win_down());
  }
}
