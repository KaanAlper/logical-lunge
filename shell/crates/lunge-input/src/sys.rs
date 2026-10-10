//! What the hook decisions need from the system, behind a trait: Windows on
//! the hook thread, a scripted fake in the tests (the decisions are tested on
//! any host).

use crate::Ev;

pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;

/// Modifier bits of a combination (the core's `Binds.SUPER` ...).
pub const SUPER: u16 = 1;
pub const CTRL: u16 = 2;
pub const SHIFT: u16 = 4;
pub const ALT: u16 = 8;

pub trait Sys {
  /// the key is down right now (GetAsyncKeyState)
  fn down(&self, vk: u32) -> bool;
  /// GetTickCount
  fn now_ms(&self) -> u32;
  fn flag(&self, id: u32) -> bool;
  fn set_flag(&self, id: u32, on: bool);
  /// (id, flag) of the core's table entry for this combination
  fn lookup(&self, kind: u8, mods: u16, vk: u32) -> Option<(i32, u8)>;
  /// the shortcut editor can name this key
  fn capturable(&self, vk: u32) -> bool;
  fn push(&self, ev: Ev);

  /// the foreground window is the desktop (its root's class)
  fn desktop_in_front(&self) -> bool;
  /// ... and its keyboard focus is in an edit box (an icon being renamed)
  fn desktop_focus_is_edit(&self) -> bool;
  /// the foreground window belongs to the shell or the desktop (Windows
  /// keeps Alt+F4 there): the "close" shortcut leaves it alone
  fn foreground_is_shell(&self) -> bool;
  /// an exclusive fullscreen Direct3D app is in front
  fn exclusive_fullscreen(&self) -> bool;
  /// the window under this point is the desktop
  fn desktop_at(&self, x: i32, y: i32) -> bool;
  /// the window under this point is an edit box
  fn edit_at(&self, x: i32, y: i32) -> bool;
  /// a second press within Windows' double-click time and rectangle
  fn double_click(&self, t0: u32, x0: i32, y0: i32, t1: u32, x1: i32, y1: i32) -> bool;
  /// past Windows' drag threshold
  fn dragged(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> bool;

  /// an unassigned key down and up (marked as ours): Windows opens neither
  /// Start nor a menu bar for a lone Win or Alt
  fn suppress_start(&self);
  /// a key release sent to Windows (marked as ours)
  fn release_key(&self, vk: u32, extended: bool);
  /// the held left press given back to Explorer at its own place, then the
  /// pointer moved on to where the drag is
  fn replay_left_down(&self, x: i32, y: i32, to_x: i32, to_y: i32);
}

pub fn is_modifier(vk: u32) -> bool {
  vk == VK_CONTROL || vk == VK_SHIFT || vk == VK_MENU || (0xA0..=0xA5).contains(&vk)
}

/// A set of virtual keys (256 bits, no allocation).
#[derive(Clone, Copy, Default)]
pub struct KeySet([u32; 8]);

impl KeySet {
  pub const EMPTY: KeySet = KeySet([0; 8]);

  pub fn contains(&self, vk: u32) -> bool {
    vk < 256 && self.0[(vk / 32) as usize] & (1 << (vk % 32)) != 0
  }

  /// true when it was not in the set
  pub fn insert(&mut self, vk: u32) -> bool {
    if vk >= 256 {
      return false;
    }
    let had = self.contains(vk);
    self.0[(vk / 32) as usize] |= 1 << (vk % 32);
    !had
  }

  /// true when it was in the set
  pub fn remove(&mut self, vk: u32) -> bool {
    let had = self.contains(vk);
    if had {
      self.0[(vk / 32) as usize] &= !(1 << (vk % 32));
    }
    had
  }

  pub fn clear(&mut self) {
    self.0 = [0; 8];
  }
}

#[cfg(test)]
pub mod fake {
  use std::cell::{Cell, RefCell};
  use std::collections::{HashMap, HashSet};

  use super::*;
  use crate::flag;

  #[derive(Default)]
  pub struct Fake {
    pub down: RefCell<HashSet<u32>>,
    pub now: Cell<u32>,
    pub flags: RefCell<[bool; flag::COUNT]>,
    pub table: RefCell<HashMap<(u8, u16, u32), (i32, u8)>>,
    pub events: RefCell<Vec<Ev>>,
    pub desktop_front: Cell<bool>,
    pub focus_edit: Cell<bool>,
    pub fg_shell: Cell<bool>,
    pub fullscreen: Cell<bool>,
    pub desktop_point: Cell<bool>,
    pub edit_point: Cell<bool>,
    pub injected: RefCell<Vec<String>>,
  }

  impl Fake {
    pub fn new() -> Self {
      let f = Fake::default();
      f.flags.borrow_mut()[flag::SHELL_UP as usize] = true;
      f.flags.borrow_mut()[flag::SWITCHER_READY as usize] = true;
      f.now.set(10_000);
      f
    }

    /// Event kinds, without the Super down/up notices (most tests are
    /// about what a combination does, not about Super itself).
    pub fn kinds(&self) -> Vec<u16> {
      self
        .events
        .borrow()
        .iter()
        .map(|e| e.kind)
        .filter(|k| !matches!(*k, crate::kind::WIN_DOWN | crate::kind::WIN_UP))
        .collect()
    }

    pub fn all_kinds(&self) -> Vec<u16> {
      self.events.borrow().iter().map(|e| e.kind).collect()
    }

    /// The events so far (without the Super notices, as `kinds`).
    pub fn take(&self) -> Vec<Ev> {
      std::mem::take(&mut *self.events.borrow_mut())
        .into_iter()
        .filter(|e| !matches!(e.kind, crate::kind::WIN_DOWN | crate::kind::WIN_UP))
        .collect()
    }
  }

  impl Sys for Fake {
    fn down(&self, vk: u32) -> bool {
      self.down.borrow().contains(&vk)
    }
    fn now_ms(&self) -> u32 {
      self.now.get()
    }
    fn flag(&self, id: u32) -> bool {
      self.flags.borrow()[id as usize]
    }
    fn set_flag(&self, id: u32, on: bool) {
      self.flags.borrow_mut()[id as usize] = on;
    }
    fn lookup(&self, kind: u8, mods: u16, vk: u32) -> Option<(i32, u8)> {
      self.table.borrow().get(&(kind, mods, vk)).copied()
    }
    fn capturable(&self, vk: u32) -> bool {
      vk != 0xFF
    }
    fn push(&self, ev: Ev) {
      self.events.borrow_mut().push(ev);
    }
    fn desktop_in_front(&self) -> bool {
      self.desktop_front.get()
    }
    fn desktop_focus_is_edit(&self) -> bool {
      self.focus_edit.get()
    }
    fn foreground_is_shell(&self) -> bool {
      self.fg_shell.get()
    }
    fn exclusive_fullscreen(&self) -> bool {
      self.fullscreen.get()
    }
    fn desktop_at(&self, _x: i32, _y: i32) -> bool {
      self.desktop_point.get()
    }
    fn edit_at(&self, _x: i32, _y: i32) -> bool {
      self.edit_point.get()
    }
    fn double_click(&self, t0: u32, x0: i32, y0: i32, t1: u32, x1: i32, y1: i32) -> bool {
      t1.wrapping_sub(t0) <= 500 && (x1 - x0).abs() * 2 <= 4 && (y1 - y0).abs() * 2 <= 4
    }
    fn dragged(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
      (x1 - x0).abs() * 2 > 4 || (y1 - y0).abs() * 2 > 4
    }
    fn suppress_start(&self) {
      self.injected.borrow_mut().push("dummy".into());
    }
    fn release_key(&self, vk: u32, extended: bool) {
      self.injected.borrow_mut().push(format!("up {vk:X}{}", if extended { " ext" } else { "" }));
    }
    fn replay_left_down(&self, x: i32, y: i32, to_x: i32, to_y: i32) {
      self.injected.borrow_mut().push(format!("left {x},{y} to {to_x},{to_y}"));
    }
  }
}
