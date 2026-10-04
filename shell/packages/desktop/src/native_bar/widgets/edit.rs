//! Typing into a note widget (several lines).
//! The widget takes the keyboard while it is edited and gives it back on
//! Esc, a click elsewhere or another window taking the focus.

use super::*;

impl Ui {
  // ------------------------------------------------------------ note editing

  pub(super) fn widgets_begin_edit(&mut self, id: u64, target: Target) {
    if self.widgets.editor.as_ref().is_some_and(|e| e.id == id && e.target == target) {
      return;
    }
    self.widgets_end_edit(true);
    let Some(s) = self.spec(id) else { return };
    let mut edit = Edit::default();
    edit.start(&s.note);
    self.widgets.editor = Some(Editor { id, target, edit, high: None });
    let Some(w) = self.widgets.wins.iter().find(|w| w.id == id) else { return };
    let hwnd = w.hwnd;
    unsafe {
      let _ = SetWindowTextW(hwnd, &HSTRING::from(EDIT_TITLE));
      let _ = SetForegroundWindow(hwnd);
      let _ = SetFocus(hwnd);
    }
    // the shell may not be allowed to take the foreground: the core raises it
    std::thread::spawn(|| core_api::run_core(&["--raise", EDIT_TITLE]));
    self.widget_paint(id, true);
  }

  /// Notes always keep their text when editing ends, including on Escape.
  pub(super) fn widgets_end_edit(&mut self, _commit: bool) {
    let Some(e) = self.widgets.editor.take() else { return };
    let text = e.edit.text();
    if let Some(s) = self.widgets.store.widgets.iter_mut().find(|s| s.id == e.id) {
      s.note = text;
    }
    save(&self.widgets.store);
    if let Some(w) = self.widgets.wins.iter().find(|w| w.id == e.id) {
      unsafe {
        let _ = SetWindowTextW(w.hwnd, TITLE);
      }
    }
    self.widget_paint(e.id, true);
  }

  pub(super) fn widget_text_click(&mut self, id: u64, x: f32, y: f32, double: bool) {
    let Some(kind) = self.spec(id).map(|s| s.kind) else { return };
    if kind != Kind::Note {
      return;
    }
    let editing = self.widgets.editor.as_ref().is_some_and(|e| e.id == id && e.target == Target::Note);
    if !editing {
      self.widgets_begin_edit(id, Target::Note);
    }
    let index = self.widget_index_at(id, x, y);
    let extend = key_down(VK_SHIFT.0);
    if let Some(e) = self.widgets.editor.as_mut().filter(|e| e.id == id) {
      match index {
        Some(i) if double => e.edit.select_word(i),
        Some(i) => e.edit.place(i, extend),
        None => e.edit.place(e.edit.chars.len(), extend),
      }
    }
    self.widget_paint(id, false);
  }

  /// The character position under a point of the note (DIP in the window).
  pub(super) fn widget_index_at(&self, id: u64, x: f32, y: f32) -> Option<usize> {
    let w = self.widgets.wins.iter().find(|w| w.id == id)?;
    let layout = w.note_layout.as_ref()?;
    let text = self.widgets.editor.as_ref().filter(|e| e.id == id).map(|e| e.edit.text())?;
    let s = self.spec(id)?;
    let inner = shape::plan(&s.shape,w.px.0 as f32 / w.scale,w.px.1 as f32 / w.scale).inner;
    let (ox,oy) = (inner.x,inner.y);
    let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    unsafe { layout.HitTestPoint(x - ox, y - oy, &mut trailing, &mut inside, &mut m) }.ok()?;
    let pos = m.textPosition + if trailing.as_bool() { m.length } else { 0 };
    Some(paint::char_at_utf16(&text, pos))
  }

  /// Up / down a line in a note: the position at the same x on the line
  /// above / below.
  pub(super) fn widget_line_move(&mut self, id: u64, down: bool, extend: bool) {
    let Some(w) = self.widgets.wins.iter().find(|w| w.id == id) else { return };
    let Some(layout) = w.note_layout.clone() else { return };
    let Some(e) = self.widgets.editor.as_ref().filter(|e| e.id == id) else { return };
    let text = e.edit.text();
    let (mut x, mut y) = (0f32, 0f32);
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    if unsafe { layout.HitTestTextPosition(paint::utf16_at(&text, e.edit.caret), false, &mut x, &mut y, &mut m) }.is_err() {
      return;
    }
    let line = if m.height > 0.0 { m.height } else { 18.0 };
    let ty = if down { y + line * 1.5 } else { y - line * 0.5 };
    let index = if ty < 0.0 {
      0
    } else {
      let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
      let mut hm = DWRITE_HIT_TEST_METRICS::default();
      if unsafe { layout.HitTestPoint(x, ty, &mut trailing, &mut inside, &mut hm) }.is_err() {
        return;
      }
      paint::char_at_utf16(&text, hm.textPosition + if trailing.as_bool() { hm.length } else { 0 })
    };
    if let Some(e) = self.widgets.editor.as_mut() {
      e.edit.place(index, extend);
    }
  }

  /// A key while editing; false: not ours (the system handles it).
  pub(super) fn widget_key_down(&mut self, id: u64, vk: u16) -> bool {
    let ctrl = key_down(VK_CONTROL.0);
    let shift = key_down(VK_SHIFT.0);
    let Some(target) = self.widgets.editor.as_ref().map(|e| e.target) else { return false };
    let hwnd = self.widgets.wins.iter().find(|w| w.id == id).map(|w| w.hwnd).unwrap_or_default();
    match vk {
      v if v == VK_ESCAPE.0 => {
        self.widgets_end_edit(target == Target::Note);
        return true;
      }
      v if v == VK_RETURN.0 => {
        if ctrl {
          self.widgets_end_edit(true);
        } else if let Some(e) = self.widgets.editor.as_mut() {
          e.edit.insert_lines("\n");
        }
      }
      v if v == VK_UP.0 || v == VK_DOWN.0 => {
        if target == Target::Note {
          self.widget_line_move(id, v == VK_DOWN.0, shift);
        }
      }
      _ => {
        let Some(e) = self.widgets.editor.as_mut() else { return false };
        match vk {
          v if v == VK_BACK.0 => e.edit.backspace(ctrl),
          v if v == VK_DELETE.0 => e.edit.delete(ctrl),
          v if v == VK_LEFT.0 => e.edit.left(ctrl, shift),
          v if v == VK_RIGHT.0 => e.edit.right(ctrl, shift),
          v if v == VK_HOME.0 => e.edit.home(shift),
          v if v == VK_END.0 => e.edit.end(shift),
          0x41 if ctrl => e.edit.select_all(),
          0x43 if ctrl => {
            let _ = set_clipboard(hwnd, &e.edit.selected());
          }
          0x58 if ctrl => {
            if set_clipboard(hwnd, &e.edit.selected()) {
              e.edit.insert("");
            }
          }
          0x56 if ctrl => {
            if let Some(t) = clipboard_text(hwnd) {
              if target == Target::Note {
                e.edit.insert_lines(&t);
              } else {
                e.edit.insert(&t);
              }
            }
          }
          0x5A if ctrl && shift => {
            e.edit.redo();
          }
          0x5A if ctrl => {
            e.edit.undo();
          }
          0x59 if ctrl => {
            e.edit.redo();
          }
          _ => return false,
        }
      }
    }
    self.widget_paint(id, false);
    true
  }

  pub(super) fn widget_char(&mut self, id: u64, unit: u16) {
    let Some(e) = self.widgets.editor.as_mut() else { return };
    let text = match unit {
      0xD800..=0xDBFF => {
        e.high = Some(unit);
        return;
      }
      0xDC00..=0xDFFF => match e.high.take() {
        Some(hi) => String::from_utf16_lossy(&[hi, unit]),
        None => return,
      },
      // control characters come as keys
      u if u < 0x20 || u == 0x7F => return,
      u => String::from_utf16_lossy(&[u]),
    };
    let limit = 4000;
    if e.edit.chars.len() < limit {
      e.edit.insert(&text);
    }
    self.widget_paint(id, false);
  }
}
