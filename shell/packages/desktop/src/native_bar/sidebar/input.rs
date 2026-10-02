//! The panel's mouse and keyboard: hover, presses and drags (text, sliders,
//! tiles, notifications), the wheel (scrolling areas, swipes), keys and
//! text, clicks handed to each part, the cursor, the focus leaving.

use super::*;

/// How far a press on a sideways row moves before it drags the row.
const ROW_DRAG_SLOP: f32 = 6.0;

impl Ui {
  /// A message for the panel's window (None: not its window).
  pub(in crate::native_bar) fn sidebar_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    let w = self.sidebar.win.as_ref()?;
    if w.hwnd != hwnd {
      return None;
    }
    let scale = w.scale;
    let dip = |lp: LPARAM| ((lp.0 & 0xFFFF) as i16 as f32 / scale, ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / scale);
    let r = match msg {
      WM_PAINT => {
        unsafe {
          let _ = ValidateRect(hwnd, None);
        }
        Some(LRESULT(0))
      }
      WM_MOUSEMOVE => {
        let (x, y) = dip(lp);
        if !self.sidebar.tracking {
          crate::native_bar::track_leave(hwnd);
          self.sidebar.tracking = true;
        }
        self.sb_mouse_move(x, y);
        Some(LRESULT(0))
      }
      WM_MOUSELEAVE => {
        self.sidebar.tracking = false;
        if self.sidebar.drag.is_none() {
          self.sidebar.mouse = None;
          if self.sidebar.hover.take().is_some() {
            self.sb_render();
          }
        }
        Some(LRESULT(0))
      }
      WM_LBUTTONDOWN => {
        let (x, y) = dip(lp);
        self.sb_press(x, y);
        Some(LRESULT(0))
      }
      WM_LBUTTONDBLCLK => {
        let (x, y) = dip(lp);
        if let Some(Hit::Field(id)) = self.sidebar.hit_at(x, y) {
          self.sidebar.field(id).select_word();
          self.sb_render();
        } else {
          self.sb_press(x, y);
        }
        Some(LRESULT(0))
      }
      WM_LBUTTONUP => {
        let (x, y) = dip(lp);
        self.sb_release(x, y);
        Some(LRESULT(0))
      }
      WM_RBUTTONUP | WM_MBUTTONUP => {
        let (x, y) = dip(lp);
        if let Some(h) = self.sidebar.hit_at(x, y) {
          self.sb_click(h, if msg == WM_RBUTTONUP { 1 } else { 2 }, x, y);
        }
        Some(LRESULT(0))
      }
      WM_CAPTURECHANGED => {
        if self.sidebar.drag.is_some() {
          self.sb_drag_cancel();
        }
        Some(LRESULT(0))
      }
      WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
        let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as i32;
        let mut p = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
        unsafe {
          let _ = ScreenToClient(hwnd, &mut p);
        }
        self.sb_wheel(p.x as f32 / scale, p.y as f32 / scale, delta, msg == WM_MOUSEHWHEEL);
        Some(LRESULT(0))
      }
      WM_KEYDOWN | WM_SYSKEYDOWN => {
        let handled = self.sb_key(wp.0 as u16);
        if !handled && msg == WM_SYSKEYDOWN {
          None
        } else {
          Some(LRESULT(0))
        }
      }
      WM_CHAR => {
        if let Some(id) = self.sidebar.focus {
          let t = self.sidebar.field(id).char(wp.0 as u16);
          if t == Typed::Changed {
            self.sb_typed(id, t);
          }
        }
        Some(LRESULT(0))
      }
      WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
        let cursor = match &self.sidebar.hover {
          Some(Hit::Field(_)) => Some(IDC_IBEAM),
          Some(Hit::Panel) | Some(Hit::Bottom(BHit::Calendar | BHit::Picker)) | Some(Hit::Quick(QHit::Card)) | None => None,
          Some(Hit::Notif(NHit::Group(_))) => None,
          Some(_) => Some(IDC_HAND),
        };
        match cursor {
          Some(c) => unsafe {
            if let Ok(h) = LoadCursorW(None, c) {
              SetCursor(h);
            }
            Some(LRESULT(1))
          },
          None => None,
        }
      }
      WM_ACTIVATE => {
        if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
          let settled = self.sidebar.shown_at.is_some_and(|t| t.elapsed() > BLUR_GRACE);
          if self.sidebar.open && settled && self.sidebar.modal == 0 && !self.menu_is_open() {
            self.sidebar_close();
          }
        }
        None
      }
      WM_CLOSE => {
        self.sidebar_close();
        Some(LRESULT(0))
      }
      _ => None,
    };
    Some(r)
  }

  pub(in crate::native_bar::sidebar) fn sb_mouse_move(&mut self, x: f32, y: f32) {
    self.sidebar.mouse = Some((x, y));
    match self.sidebar.drag.clone() {
      Some(Drag::Field(id)) => {
        if self.sidebar.field(id).drag(x, y) {
          self.sb_render();
        }
        return;
      }
      Some(Drag::Slider(h)) => {
        self.sb_slide_to(&h, x, y, false);
        return;
      }
      Some(Drag::Tile) => {
        self.sb_tile_drag(x, y, false);
        return;
      }
      Some(Drag::Notif) => {
        if self.sidebar.notifs.drag(x, y) {
          self.sb_render();
        }
        return;
      }
      Some(Drag::Page(h)) => {
        if self.sb_page_drag(&h, x, y) {
          return;
        }
      }
      Some(Drag::Row { id, x0, off0, moved }) => {
        // past a few DIPs the press scrolls the row instead of clicking
        if moved || (x - x0).abs() > ROW_DRAG_SLOP {
          self.sidebar.drag = Some(Drag::Row { id, x0, off0, moved: true });
          if let Some(r) = self.sidebar.regions.iter().find(|r| r.id == id).copied() {
            self.sidebar.scroll.insert(id, (off0 - (x - x0)).clamp(0.0, r.max));
          }
          return self.sb_render();
        }
      }
      None => {}
    }
    let hit = self.sidebar.hit_at(x, y);
    if hit != self.sidebar.hover {
      self.sb_walls_hover(hit.as_ref());
      self.sidebar.hover = hit;
      self.sb_render();
    } else if matches!(hit, Some(Hit::Notif(_))) || matches!(self.sidebar.page, Some(Page::Walls)) {
      // per-item hover (× of a notification, a gallery's moving preview)
      self.sb_render();
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_press(&mut self, x: f32, y: f32) {
    self.menu_close();
    let hit = self.sidebar.hit_at(x, y);
    self.sidebar.pressed = hit.clone();
    unsafe {
      SetCapture(self.sidebar.win.as_ref().map(|w| w.hwnd).unwrap_or_default());
    }
    // a press outside the card and the tiles closes the card; outside the
    // picker and the month title, the picker
    let in_quick = matches!(hit, Some(Hit::Quick(_)) | Some(Hit::Field(FieldId::WifiPw | FieldId::NightFrom | FieldId::NightTo)));
    if self.sidebar.quick.has_card() && !in_quick {
      self.sidebar.quick.set_menu(None, 0.0);
      self.sb_frames();
    }
    let in_picker = matches!(hit, Some(Hit::Bottom(BHit::Picker | BHit::PickPrev | BHit::PickNext | BHit::Month(_) | BHit::Today | BHit::Title)));
    if self.sidebar.bottom.picker_open() && !in_picker {
      self.sidebar.bottom.close_picker();
    }
    // the focus follows the press; a time field left is saved
    let new_focus = match &hit {
      Some(Hit::Field(id)) => Some(*id),
      _ => None,
    };
    if new_focus != self.sidebar.focus {
      if let Some(old) = self.sidebar.focus {
        self.sb_blurred(old);
      }
      self.sidebar.focus = new_focus;
    }
    let (_, shift) = modifiers();
    match hit {
      Some(Hit::Field(id)) => {
        self.sidebar.field(id).press(x, y, shift);
        self.sidebar.drag = Some(Drag::Field(id));
      }
      Some(Hit::Quick(q @ (QHit::AudioSlider | QHit::NightSlider))) => {
        let h = Hit::Quick(q);
        self.sidebar.drag = Some(Drag::Slider(h.clone()));
        self.sb_slide_to(&h, x, y, false);
      }
      Some(Hit::Quick(QHit::Tile(t))) if self.sidebar.quick.edit => {
        let tr = |s: &str| self.model.tr(s);
        self.sidebar.quick.press(&self.model, &tr, t, x, y);
        self.sidebar.drag = Some(Drag::Tile);
        // a long press lifts it: frames watch the time
        self.sb_frames();
      }
      Some(Hit::Notif(NHit::Group(app))) => {
        self.sidebar.notifs.press(&app, x, y);
        self.sidebar.drag = Some(Drag::Notif);
      }
      Some(h @ (Hit::Walls(_) | Hit::Keys(_) | Hit::Bug(_))) => {
        if self.sb_page_press(&h, x, y) {
          self.sidebar.drag = Some(Drag::Page(h));
        }
      }
      _ => {}
    }
    // a sideways row: a press may become a drag of the row
    if self.sidebar.drag.is_none() {
      if let Some(r) = self.sidebar.region_at(x, y, true) {
        let off0 = self.sidebar.scroll.get(&r.id).copied().unwrap_or(0.0);
        self.sidebar.drag = Some(Drag::Row { id: r.id, x0: x, off0, moved: false });
      }
    }
    self.sb_render();
  }

  pub(in crate::native_bar::sidebar) fn sb_release(&mut self, x: f32, y: f32) {
    let drag = self.sidebar.drag.take();
    let pressed = self.sidebar.pressed.take();
    unsafe {
      let _ = ReleaseCapture();
    }
    match drag {
      Some(Drag::Field(id)) => {
        self.sidebar.field(id).dragging = false;
        self.sb_render();
        return;
      }
      Some(Drag::Slider(h)) => {
        self.sb_slide_to(&h, x, y, true);
        return;
      }
      Some(Drag::Tile) => {
        if self.sb_tile_drop(true) {
          return;
        }
      }
      Some(Drag::Notif) => {
        if let Some(app) = self.sidebar.notifs.release() {
          return self.sb_notif_settled(Some(app));
        }
        // released without a drag: a click on the group
        self.sb_notif_settled(None);
      }
      Some(Drag::Page(h)) => {
        if self.sb_page_release(&h, x, y) {
          return;
        }
      }
      Some(Drag::Row { moved: true, .. }) => return self.sb_render(),
      Some(Drag::Row { .. }) | None => {}
    }
    let hit = self.sidebar.hit_at(x, y);
    if let (Some(h), Some(p)) = (hit, pressed) {
      if h == p {
        self.sb_click(h, 0, x, y);
        return;
      }
    }
    self.sb_render();
  }

  pub(in crate::native_bar::sidebar) fn sb_drag_cancel(&mut self) {
    match self.sidebar.drag.take() {
      Some(Drag::Tile) => {
        self.sb_tile_drop(false);
      }
      Some(Drag::Notif) => {
        let app = self.sidebar.notifs.release();
        let _ = app;
      }
      Some(Drag::Field(id)) => self.sidebar.field(id).dragging = false,
      _ => {}
    }
    self.sidebar.pressed = None;
    self.sb_render();
  }

  pub(in crate::native_bar::sidebar) fn sb_slide_to(&mut self, h: &Hit, x: f32, y: f32, done: bool) {
    match h {
      Hit::Quick(q) => self.sb_quick_slide(q, x, done),
      other => {
        let _ = self.sb_page_slider(other, x, y, done);
      }
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_wheel(&mut self, x: f32, y: f32, delta: i32, horizontal: bool) {
    let hit = self.sidebar.hit_at(x, y);
    let up = delta > 0;
    if !horizontal {
      match &hit {
        Some(Hit::Quick(QHit::Tile(t))) if self.sidebar.quick.edit => {
          self.sidebar.quick.edit_move(*t, if up { -1 } else { 1 });
          self.sidebar.store.quick_toggles = Some(self.sidebar.quick.toggles.clone());
          self.sidebar.save_soon();
          return self.sb_render();
        }
        Some(Hit::Quick(QHit::AudioSlider)) => return self.sb_audio_wheel(up),
        Some(Hit::Bottom(b)) => {
          if self.sb_bottom_wheel(b, up) {
            return;
          }
        }
        _ => {}
      }
    } else if let Some(Hit::Notif(NHit::Group(app) | NHit::Expand(app) | NHit::Close(app))) = &hit {
      // a two-finger swipe moves the group with the fingers
      self.sidebar.notifs.hwheel(app, delta);
      unsafe { SetTimer(self.msg_hwnd, TIMER_SB_WHEEL, 140, None) };
      return self.sb_render();
    }
    // a scrolling area: vertical ones by the wheel, rows also by a vertical wheel
    // a sideways row under the pointer takes a vertical wheel too (before the
    // page around it)
    let region = self.sidebar.region_at(x, y, true).or_else(|| if horizontal { None } else { self.sidebar.region_at(x, y, false) });
    let Some(r) = region else { return };
    let step = delta as f32 / 120.0 * if r.horizontal { 90.0 } else { 60.0 };
    let off = self.sidebar.scroll.entry(r.id).or_insert(0.0);
    let dir = if horizontal { step } else { -step };
    *off = (*off + dir).clamp(0.0, r.max);
    self.sb_render();
  }

  pub(in crate::native_bar::sidebar) fn sb_key(&mut self, vk: u16) -> bool {
    if self.menu_key(vk) {
      return true;
    }
    let (ctrl, shift) = modifiers();
    // Esc during a tile drag: it goes back where it was
    if vk == 0x1B && self.sidebar.drag == Some(Drag::Tile) {
      self.sidebar.drag = None;
      unsafe {
        let _ = ReleaseCapture();
      }
      self.sb_tile_drop(false);
      return true;
    }
    // the shortcut editor waits for the core to catch a combination
    if self.sidebar.keys.capturing.is_some() {
      return true;
    }
    if let Some(id) = self.sidebar.focus {
      let t = self.sidebar.field(id).key(vk, ctrl, shift);
      match t {
        Typed::Nothing if vk == 0x09 => {}
        Typed::Nothing => return true,
        Typed::Escape => {
          self.sb_blurred(id);
          self.sidebar.focus = None;
          self.sb_render();
          return true;
        }
        Typed::Copy(s) => {
          if let Some(w) = &self.sidebar.win {
            crate::native_bar::overview::set_clipboard(w.hwnd, &s);
          }
          self.sb_typed(id, Typed::Changed);
          return true;
        }
        Typed::Paste => {
          if let Some(s) = self.sidebar.win.as_ref().and_then(|w| crate::native_bar::overview::clipboard_text(w.hwnd)) {
            self.sidebar.field(id).insert(&s);
            self.sb_typed(id, Typed::Changed);
          }
          return true;
        }
        other => {
          self.sb_typed(id, other);
          return true;
        }
      }
    }
    match vk {
      0x1B => {
        if self.sidebar.quick.has_card() {
          self.sidebar.quick.set_menu(None, 0.0);
          self.sb_frames();
        } else if self.sidebar.bottom.picker_open() {
          self.sidebar.bottom.close_picker();
          self.sb_render();
        } else if self.sb_page_escape() {
        } else if self.sidebar.page_open().is_some() {
          self.sb_page_close();
        } else {
          self.sidebar_close();
        }
        true
      }
      _ => false,
    }
  }

  /// A key changed or submitted a field.
  pub(in crate::native_bar::sidebar) fn sb_typed(&mut self, id: FieldId, t: Typed) {
    match id {
      FieldId::Todo => self.sb_todo_key(t),
      FieldId::WifiPw | FieldId::NightFrom | FieldId::NightTo => self.sb_quick_key(id, t),
      FieldId::KeysSearch => self.sb_keys_typed(t),
      FieldId::SaverMinutes => self.sb_walls_typed(id, t),
      FieldId::BugStart | FieldId::BugEnd | FieldId::BugText => self.sb_bug_typed(id, t),
    }
    self.sb_render();
  }

  /// The focus left a field.
  pub(in crate::native_bar::sidebar) fn sb_blurred(&mut self, id: FieldId) {
    match id {
      FieldId::NightFrom | FieldId::NightTo => self.sb_night_time(id),
      FieldId::SaverMinutes => self.sb_walls_typed(id, Typed::Submit),
      _ => {}
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_click(&mut self, h: Hit, button: u8, x: f32, y: f32) {
    match h {
      Hit::Sys(s) if button == 0 => self.sb_sys(s),
      Hit::Quick(q) => self.sb_quick_click(q, button),
      Hit::Notif(n) => self.sb_notif_click(n, button),
      Hit::Bottom(b) if button == 0 => self.sb_bottom_click(b),
      Hit::Back if button == 0 => self.sb_page_close(),
      Hit::Keys(k) => self.sb_keys_click(k, button),
      Hit::Walls(w) => self.sb_walls_click(w, button, x, y),
      Hit::Bug(b) => self.sb_bug_click(b, button),
      _ => self.sb_render(),
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_sys(&mut self, s: Sys) {
    match s {
      Sys::Bug => self.sb_page_open(Page::Bug),
      Sys::Walls => self.sb_page_open(Page::Walls),
      Sys::Keys => self.sb_page_open(Page::Keys),
      Sys::Update => {
        self.sidebar_close();
        self.update_event(crate::native_bar::update::Event::Check(true));
      }
      Sys::Settings => {
        self.sidebar_close();
        self.settings_toggle();
      }
      Sys::Session => {
        self.sidebar_close();
        self.session_toggle();
      }
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_page_press(&mut self, _h: &Hit, _x: f32, _y: f32) -> bool {
    false
  }

  pub(in crate::native_bar::sidebar) fn sb_page_drag(&mut self, _h: &Hit, _x: f32, _y: f32) -> bool {
    false
  }

  pub(in crate::native_bar::sidebar) fn sb_page_release(&mut self, _h: &Hit, _x: f32, _y: f32) -> bool {
    false
  }

  pub(in crate::native_bar::sidebar) fn sb_page_slider(&mut self, _h: &Hit, _x: f32, _y: f32, _done: bool) -> bool {
    false
  }

  /// Esc on a page: nothing of its own to close first (confirmations are
  /// the shared dialog, which takes its own Esc).
  pub(in crate::native_bar::sidebar) fn sb_page_escape(&mut self) -> bool {
    false
  }
}
