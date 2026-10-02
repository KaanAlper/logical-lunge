//! The one context menu of Logical Lunge: every right click we own (the
//! Super menu's apps, the desktop, the Dock, the bar, notification cards,
//! the sidebar's galleries) opens this menu, drawn like our popups.
//!
//! ```ignore
//! ui.menu_open(at, MenuFocus::Take, vec![
//!   Item::new("open", Some("open_in_new"), tr("Aç")),
//!   Item::sep(),
//!   Item::new("pin", Some("keep"), tr("Dock'a sabitle")).checked(pinned),
//! ], move |ui, id| match id { "open" => ..., _ => {} });
//! ```
//!
//! Items have an id, an optional Material Symbols icon, a label, and may be
//! disabled, checked, a separator or open a submenu. The chosen id goes to
//! the callback (nothing is called when the menu is dismissed). Arrows,
//! Enter, Esc and Right/Left (submenus) work; a click outside or the focus
//! moving away closes it.

use windows::Win32::{
  Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
  Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST},
  UI::{
    HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
    WindowsAndMessaging::*,
  },
};

use super::{
  fonts::TextStyle,
  gfx::{Rect, Rgba},
  popup::{self, Motion, PopWin, PAD},
  view::{Align, Painter, Theme},
  Ui,
};

const ROW_H: f32 = 34.0;
const SEP_H: f32 = 9.0;
const V_PAD: f32 = 6.0;
const RADIUS: f32 = 12.0;
const ICON_COL: f32 = 40.0;
const TEXT_PAD: f32 = 14.0;
const SUB_COL: f32 = 26.0;
const MIN_W: f32 = 190.0;
const MAX_W: f32 = 360.0;
const LABEL: TextStyle = TextStyle { size: 13.5, weight: 450.0 };
/// hides a closed menu's windows once their fade is over
const TIMER_GONE: usize = 0x4D4E;

/// One row of a menu.
#[derive(Clone, Debug, Default)]
pub struct Item {
  pub id: String,
  pub icon: Option<&'static str>,
  pub label: String,
  pub enabled: bool,
  pub checked: bool,
  pub separator: bool,
  pub submenu: Vec<Item>,
}

impl Item {
  pub fn new(id: &str, icon: Option<&'static str>, label: impl Into<String>) -> Self {
    Item { id: id.to_string(), icon, label: label.into(), enabled: true, ..Default::default() }
  }

  pub fn sep() -> Self {
    Item { separator: true, ..Default::default() }
  }

  pub fn enabled(mut self, on: bool) -> Self {
    self.enabled = on;
    self
  }

  pub fn checked(mut self, on: bool) -> Self {
    self.checked = on;
    self
  }

  pub fn submenu(mut self, items: Vec<Item>) -> Self {
    self.submenu = items;
    self
  }

  fn pickable(&self) -> bool {
    !self.separator && self.enabled
  }
}

/// Whether the menu takes the keyboard focus.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MenuFocus {
  /// Opened from a window of ours that has the focus (the Super menu): it
  /// keeps it, passes its keys with `menu_key` and closes the menu on its
  /// own clicks and when it hides.
  Keep,
  /// Opened from a window that never takes the focus (the bar, the Dock,
  /// the desktop): the menu takes it like a Windows menu and gives it back
  /// when it closes.
  Take,
}

type Pick = Box<dyn FnOnce(&mut Ui, &str)>;

/// One open level (the root menu, then its open submenus).
struct Level {
  win: PopWin,
  items: Vec<Item>,
  hot: Option<usize>,
  /// row rectangles in DIPs from the window's top-left
  rows: Vec<(Rect, usize)>,
  /// the box in the window (DIPs)
  size: (f32, f32),
}

pub struct MenuState {
  levels: Vec<Level>,
  focus: MenuFocus,
  /// the window that had the focus before a `Take` menu
  prev_focus: HWND,
  pick: Option<Pick>,
}

/// Windows of closed menus, until their fade-out ends.
#[derive(Default)]
pub struct MenuGone {
  wins: Vec<PopWin>,
}

fn monitor_at(at: POINT) -> (RECT, f32) {
  unsafe {
    let mon = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(mon, &mut info);
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    (info.rcWork, dx as f32 / 96.0)
  }
}

/// Where a `w` x `h` pixel window goes so its box starts at `at` (opens
/// leftwards or upwards when it would leave the work area). `alt_x`: the x to
/// open leftwards from (a submenu flips to the left of its parent).
fn place(at: POINT, alt_x: i32, w: i32, h: i32, margin: i32, work: RECT) -> (i32, i32) {
  let mut x = at.x - margin;
  if x + w - margin > work.right {
    x = alt_x - w + margin;
  }
  let mut y = at.y - margin;
  if y + h - margin > work.bottom {
    y = (at.y - h + margin).min(work.bottom - h + margin);
  }
  x = x.max(work.left - margin);
  y = y.max(work.top - margin);
  (x, y)
}

/// The next pickable row from `from` in `dir` (wraps around).
fn step(items: &[Item], from: Option<usize>, dir: i32) -> Option<usize> {
  let n = items.len() as i32;
  if n == 0 {
    return None;
  }
  let mut i = match from {
    Some(i) => i as i32,
    None if dir > 0 => -1,
    None => n,
  };
  for _ in 0..n {
    i = (i + dir).rem_euclid(n);
    if items[i as usize].pickable() {
      return Some(i as usize);
    }
  }
  None
}

fn box_size(p: &mut Painter, items: &[Item]) -> anyhow::Result<(f32, f32)> {
  let icons = items.iter().any(|i| i.icon.is_some() || i.checked);
  let subs = items.iter().any(|i| !i.submenu.is_empty());
  let mut w: f32 = 0.0;
  let mut h = 2.0 * V_PAD;
  for it in items {
    if it.separator {
      h += SEP_H;
      continue;
    }
    w = w.max(p.measure(&it.label, LABEL)?);
    h += ROW_H;
  }
  let left = if icons { ICON_COL } else { TEXT_PAD };
  let right = if subs { SUB_COL } else { TEXT_PAD };
  Ok(((left + w.ceil() + right + 6.0).clamp(MIN_W, MAX_W), h))
}

fn paint(p: &mut Painter, t: &Theme, items: &[Item], hot: Option<usize>, size: (f32, f32)) -> anyhow::Result<Vec<(Rect, usize)>> {
  let bx = Rect::new(PAD, PAD, size.0, size.1);
  popup::frame_shadow(p, bx, RADIUS)?;
  p.fill_round(bx, RADIUS, t.surface_container)?;
  p.stroke_round(bx, RADIUS, t.border, 1.0)?;
  let icons = items.iter().any(|i| i.icon.is_some() || i.checked);
  let mut rows = Vec::new();
  let mut y = bx.y + V_PAD;
  for (i, it) in items.iter().enumerate() {
    if it.separator {
      p.fill(Rect::new(bx.x + 12.0, y + SEP_H / 2.0, bx.w - 24.0, 1.0), t.outline_variant)?;
      y += SEP_H;
      continue;
    }
    let r = Rect::new(bx.x + 4.0, y, bx.w - 8.0, ROW_H);
    if hot == Some(i) && it.enabled {
      p.fill_round(r, 8.0, t.surface_container_high)?;
    }
    let fg = if it.enabled { t.on_layer0 } else { Rgba(t.on_layer0.0, t.on_layer0.1, t.on_layer0.2, 0.38) };
    let soft = if it.enabled { t.on_surface_variant } else { Rgba(t.on_surface_variant.0, t.on_surface_variant.1, t.on_surface_variant.2, 0.38) };
    let icon = if it.checked { Some("check") } else { it.icon };
    if let Some(name) = icon {
      p.icon(name, r.x + ICON_COL / 2.0 - 2.0, r.y + ROW_H / 2.0, 19.0, it.checked, if it.checked { t.primary } else { soft })?;
    }
    let tx = r.x + if icons { ICON_COL - 4.0 } else { TEXT_PAD - 4.0 };
    let right = if it.submenu.is_empty() { TEXT_PAD } else { SUB_COL };
    p.text(&it.label, Rect::new(tx, r.y, r.right() - right - tx, ROW_H), LABEL, fg, Align::Left, false)?;
    if !it.submenu.is_empty() {
      p.icon("chevron_right", r.right() - SUB_COL / 2.0, r.y + ROW_H / 2.0, 18.0, false, soft)?;
    }
    rows.push((r, i));
    y += ROW_H;
  }
  Ok(rows)
}

impl Ui {
  /// Opens a menu with its top-left corner at `at` (screen pixels, the
  /// pointer for a right click), on that point's monitor. `pick` gets the
  /// chosen item's id. An open menu is replaced.
  pub(super) fn menu_open(&mut self, at: POINT, focus: MenuFocus, items: Vec<Item>, pick: impl FnOnce(&mut Ui, &str) + 'static) {
    self.menu_close();
    if items.iter().all(|i| i.separator) {
      return;
    }
    let prev_focus = unsafe { GetForegroundWindow() };
    self.menu = Some(MenuState { levels: Vec::new(), focus, prev_focus, pick: Some(Box::new(pick)) });
    if !self.menu_push(items, at, at.x) {
      self.menu = None;
      return;
    }
    if focus == MenuFocus::Take {
      if let Some(level) = self.menu.as_ref().and_then(|m| m.levels.first()) {
        let hwnd = level.win.hwnd;
        unsafe {
          // a menu takes the keys (its windows are created never to)
          let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
          SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE.0 as isize));
          let _ = SetForegroundWindow(hwnd);
        }
      }
    }
  }

  /// Opens a level: the root, or a submenu next to its row.
  fn menu_push(&mut self, items: Vec<Item>, at: POINT, alt_x: i32) -> bool {
    let (work, scale) = monitor_at(at);
    let size = match self.menu_measure(&items) {
      Ok(s) => s,
      Err(err) => {
        tracing::warn!("Menu: measure: {:?}", err);
        return false;
      }
    };
    let motion = if self.model.animations { Motion::Fade } else { Motion::None };
    let mut win = match PopWin::new(&self.gfx, "Logical Lunge · menu", scale, motion) {
      Ok(w) => w,
      Err(err) => {
        tracing::warn!("Menu: window: {:?}", err);
        return false;
      }
    };
    let (w, h) = (size.0 + 2.0 * PAD, size.1 + 2.0 * PAD);
    if win.resize(&self.gfx, w, h).is_err() {
      return false;
    }
    let theme = self.theme();
    let mut rows = Vec::new();
    {
      let Ui { gfx, fonts, res, icons, .. } = self;
      let mut requests = Vec::new();
      let _ = win.draw(|dc| {
        let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
        match paint(&mut p, &theme, &items, None, size) {
          Ok(r) => rows = r,
          Err(err) => tracing::warn!("Menu: paint: {:?}", err),
        }
        Ok(())
      });
    }
    let margin = (PAD * scale).round() as i32;
    let (x, y) = place(at, alt_x, (w * scale).ceil() as i32, (h * scale).ceil() as i32, margin, work);
    if win.show_at(&self.gfx, x, y).is_err() {
      return false;
    }
    let Some(m) = self.menu.as_mut() else { return false };
    m.levels.push(Level { win, items, hot: None, rows, size });
    true
  }

  fn menu_measure(&mut self, items: &[Item]) -> anyhow::Result<(f32, f32)> {
    let mut requests = Vec::new();
    let Ui { gfx, fonts, res, icons, .. } = self;
    let mut p = Painter { dc: &gfx.dc, gfx, fonts, res, icons, requests: &mut requests };
    box_size(&mut p, items)
  }

  fn menu_redraw(&mut self, level: usize) {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, menu, .. } = self;
    let Some(l) = menu.as_mut().and_then(|m| m.levels.get_mut(level)) else { return };
    let mut requests = Vec::new();
    let (items, hot, size) = (&l.items, l.hot, l.size);
    let mut rows = Vec::new();
    let _ = l.win.draw(|dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Ok(r) = paint(&mut p, &theme, items, hot, size) {
        rows = r;
      }
      Ok(())
    });
    l.rows = rows;
    unsafe {
      let _ = gfx.dcomp.Commit();
    }
  }

  pub(super) fn menu_is_open(&self) -> bool {
    self.menu.is_some()
  }

  /// Closes the menu without a choice.
  pub(super) fn menu_close(&mut self) {
    let Some(m) = self.menu.take() else { return };
    self.menu_retire(m);
  }

  /// Fades the levels out; a `Take` menu gives the focus back.
  fn menu_retire(&mut self, m: MenuState) {
    let gfx = &self.gfx;
    let mut wait = 0;
    for mut l in m.levels {
      let _ = l.win.close(gfx);
      wait = wait.max(l.win.close_ms());
      if wait == 0 {
        l.win.hide();
      }
      self.menu_gone.wins.push(l.win);
    }
    unsafe {
      if let Some(first) = self.menu_gone.wins.last() {
        let _ = SetTimer(first.hwnd, TIMER_GONE, wait.max(1), None);
      }
      if m.focus == MenuFocus::Take && !m.prev_focus.is_invalid() && IsWindow(m.prev_focus).as_bool() {
        let _ = SetForegroundWindow(m.prev_focus);
      }
    }
  }

  /// The row `i` of the deepest level was chosen.
  fn menu_choose(&mut self, level: usize, i: usize) {
    let Some(m) = self.menu.as_mut() else { return };
    let Some(item) = m.levels.get(level).and_then(|l| l.items.get(i)).cloned() else { return };
    if !item.pickable() {
      return;
    }
    if !item.submenu.is_empty() {
      self.menu_open_sub(level, i);
      return;
    }
    let Some(mut m) = self.menu.take() else { return };
    let pick = m.pick.take();
    self.menu_retire(m);
    if let Some(pick) = pick {
      pick(self, &item.id);
    }
  }

  /// Opens row `i`'s submenu of `level` beside it (closing deeper ones).
  fn menu_open_sub(&mut self, level: usize, i: usize) {
    self.menu_truncate(level + 1);
    let Some(m) = self.menu.as_ref() else { return };
    let Some(l) = m.levels.get(level) else { return };
    let Some((r, _)) = l.rows.iter().find(|(_, j)| *j == i) else { return };
    let items = l.items[i].submenu.clone();
    let s = l.win.scale;
    let (ox, oy) = l.win.origin;
    let at = POINT { x: ox + ((r.right() + 2.0) * s).round() as i32, y: oy + ((r.y - V_PAD) * s).round() as i32 };
    let alt_x = ox + ((r.x - 2.0) * s).round() as i32;
    if self.menu_push(items, at, alt_x) {
      if let Some(sub) = self.menu.as_mut().and_then(|m| m.levels.last_mut()) {
        sub.hot = step(&sub.items, None, 1);
      }
      let last = self.menu.as_ref().map_or(0, |m| m.levels.len() - 1);
      self.menu_redraw(last);
    }
  }

  fn menu_truncate(&mut self, keep: usize) {
    let Some(m) = self.menu.as_mut() else { return };
    if m.levels.len() <= keep {
      return;
    }
    let extra: Vec<Level> = m.levels.drain(keep..).collect();
    for mut l in extra {
      l.win.hide();
    }
  }

  fn menu_set_hot(&mut self, level: usize, hot: Option<usize>) {
    let Some(l) = self.menu.as_mut().and_then(|m| m.levels.get_mut(level)) else { return };
    if l.hot == hot {
      return;
    }
    l.hot = hot;
    self.menu_redraw(level);
  }

  /// A key for the open menu (from its own window, or passed on by a
  /// `Keep` owner). Returns false when no menu is open.
  pub(super) fn menu_key(&mut self, vk: u16) -> bool {
    let Some(m) = self.menu.as_ref() else { return false };
    let level = m.levels.len().saturating_sub(1);
    let Some(l) = m.levels.get(level) else { return false };
    let hot = l.hot;
    match vk {
      0x1B => {
        // Esc: a submenu closes back to its parent, the root closes
        if level > 0 {
          self.menu_truncate(level);
        } else {
          self.menu_close();
        }
      }
      0x25 if level > 0 => self.menu_truncate(level),
      0x26 | 0x28 => {
        let next = step(&l.items, hot, if vk == 0x26 { -1 } else { 1 });
        self.menu_set_hot(level, next);
      }
      0x24 => {
        let first = step(&l.items, None, 1);
        self.menu_set_hot(level, first);
      }
      0x23 => {
        let last = step(&l.items, None, -1);
        self.menu_set_hot(level, last);
      }
      0x27 => {
        if let Some(i) = hot.filter(|&i| !l.items[i].submenu.is_empty()) {
          self.menu_open_sub(level, i);
        }
      }
      0x0D | 0x20 => {
        if let Some(i) = hot {
          self.menu_choose(level, i);
        }
      }
      _ => {}
    }
    true
  }

  /// Messages of the menu's windows (None: not one of them).
  pub(super) fn menu_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    if let Some(i) = self.menu_gone.wins.iter().position(|w| w.hwnd == hwnd) {
      if msg == WM_TIMER && wp.0 == TIMER_GONE {
        unsafe {
          let _ = KillTimer(hwnd, TIMER_GONE);
        }
        // every closed window has faded by now
        let _ = i;
        self.menu_gone.wins.clear();
        return Some(Some(LRESULT(0)));
      }
      return Some(None);
    }
    let level = self.menu.as_ref()?.levels.iter().position(|l| l.win.hwnd == hwnd)?;
    let (s, size, rows, kinds, prev_hot, depth, keep) = {
      let m = self.menu.as_ref()?;
      let l = &m.levels[level];
      let kinds: Vec<(bool, bool)> = l.items.iter().map(|i| (i.pickable(), !i.submenu.is_empty())).collect();
      (l.win.scale, l.size, l.rows.clone(), kinds, l.hot, m.levels.len(), m.focus == MenuFocus::Keep)
    };
    let at = || ((lp.0 & 0xFFFF) as i16 as f32 / s, ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / s);
    let row_at = |x: f32, y: f32| rows.iter().find(|(r, _)| r.contains(x, y)).map(|(_, i)| *i);
    match msg {
      WM_PAINT => {
        unsafe {
          let _ = ValidateRect(hwnd, None);
        }
        Some(Some(LRESULT(0)))
      }
      WM_MOUSEACTIVATE if keep => Some(Some(LRESULT(MA_NOACTIVATE as isize))),
      WM_MOUSEMOVE => {
        let (x, y) = at();
        let Some(i) = row_at(x, y).filter(|&i| kinds[i].0) else { return Some(Some(LRESULT(0))) };
        let open_here = depth > level + 1 && prev_hot == Some(i);
        self.menu_set_hot(level, Some(i));
        // a row with a submenu opens it; another row closes the open one
        if kinds[i].1 {
          if !open_here {
            self.menu_open_sub(level, i);
          }
        } else if depth > level + 1 {
          self.menu_truncate(level + 1);
        }
        Some(Some(LRESULT(0)))
      }
      WM_LBUTTONUP | WM_RBUTTONUP => {
        let (x, y) = at();
        if let Some(i) = row_at(x, y) {
          self.menu_choose(level, i);
        }
        Some(Some(LRESULT(0)))
      }
      WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
        // inside the frame's shadow margin: outside the menu
        let (x, y) = at();
        let inside = x >= PAD && y >= PAD && x <= PAD + size.0 && y <= PAD + size.1;
        if !inside {
          self.menu_close();
        }
        Some(Some(LRESULT(0)))
      }
      WM_KEYDOWN | WM_SYSKEYDOWN => {
        self.menu_key(wp.0 as u16);
        Some(Some(LRESULT(0)))
      }
      WM_ACTIVATE => {
        // a `Take` menu: the focus went elsewhere (a click outside)
        if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE && level == 0 {
          let to = HWND(lp.0 as _);
          let ours = self.menu.as_ref().is_some_and(|m| m.levels.iter().any(|l| l.win.hwnd == to));
          if !ours {
            if let Some(mut m) = self.menu.take() {
              // the focus already moved on: do not take it back
              m.prev_focus = HWND::default();
              self.menu_retire(m);
            }
          }
        }
        Some(Some(LRESULT(0)))
      }
      WM_SETCURSOR => {
        unsafe {
          if let Ok(arrow) = LoadCursorW(None, IDC_ARROW) {
            SetCursor(arrow);
          }
        }
        Some(Some(LRESULT(1)))
      }
      _ => Some(None),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn items() -> Vec<Item> {
    vec![
      Item::new("a", None, "A"),
      Item::sep(),
      Item::new("b", None, "B").enabled(false),
      Item::new("c", None, "C"),
    ]
  }

  #[test]
  fn arrows_skip_separators_and_disabled_rows() {
    let it = items();
    assert_eq!(step(&it, None, 1), Some(0));
    assert_eq!(step(&it, Some(0), 1), Some(3));
    assert_eq!(step(&it, Some(3), 1), Some(0));
    assert_eq!(step(&it, None, -1), Some(3));
    assert_eq!(step(&[Item::sep()], None, 1), None);
  }

  #[test]
  fn a_menu_stays_inside_the_work_area() {
    let work = RECT { left: 0, top: 0, right: 1000, bottom: 800 };
    // fits: the box's corner at the pointer
    assert_eq!(place(POINT { x: 100, y: 100 }, 100, 200, 300, 10, work), (90, 90));
    // too far right and down: opens left of and above the pointer
    assert_eq!(place(POINT { x: 950, y: 700 }, 950, 200, 300, 10, work), (760, 410));
    // a submenu flips to the left of its parent
    assert_eq!(place(POINT { x: 990, y: 100 }, 700, 200, 300, 10, work), (510, 90));
  }
}
