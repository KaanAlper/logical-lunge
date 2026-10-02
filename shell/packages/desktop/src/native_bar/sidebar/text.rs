//! Text fields of the right panel (search, to-do, Wi-Fi password, times,
//! the issue report's description): characters, caret and selection, the
//! keys a browser input understands, a mouse caret, and drawing.

use windows::Win32::Graphics::{
  Direct2D::{D2D1_ANTIALIAS_MODE_ALIASED, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT},
  DirectWrite::{IDWriteTextLayout, DWRITE_HIT_TEST_METRICS, DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_WRAP, DWRITE_LINE_SPACING_METHOD_UNIFORM},
};

use super::super::{
  fonts::TextStyle,
  gfx::{pt, Rect, Rgba},
  view::{Align, Painter, Theme},
};

/// What a key did to a field.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Typed {
  Nothing,
  /// the text changed
  Changed,
  /// only the caret or the selection moved
  Moved,
  /// Enter in a one-line field
  Submit,
  Escape,
  Copy(String),
  Paste,
}

fn is_word(c: char) -> bool {
  c.is_alphanumeric() || c == '_'
}

#[derive(Default)]
pub(super) struct TextField {
  pub chars: Vec<char>,
  pub caret: usize,
  pub anchor: usize,
  pub multiline: bool,
  pub password: bool,
  /// longest text (0: any)
  pub max: usize,
  scroll_x: f32,
  scroll_y: f32,
  surrogate: Option<u16>,
  /// the last drawing: the layout and where its origin was (DIPs)
  layout: Option<(IDWriteTextLayout, f32, f32)>,
  /// the pointer is selecting
  pub dragging: bool,
}

impl TextField {
  pub fn new(multiline: bool) -> Self {
    Self { multiline, ..Default::default() }
  }

  pub fn text(&self) -> String {
    self.chars.iter().collect()
  }

  pub fn is_empty(&self) -> bool {
    self.chars.is_empty()
  }

  pub fn set(&mut self, s: &str) {
    self.chars = s.chars().collect();
    self.caret = self.chars.len();
    self.anchor = self.caret;
    self.scroll_x = 0.0;
    self.scroll_y = 0.0;
  }

  pub fn selection(&self) -> (usize, usize) {
    (self.caret.min(self.anchor), self.caret.max(self.anchor))
  }

  fn selected(&self) -> String {
    let (a, b) = self.selection();
    self.chars[a..b].iter().collect()
  }

  /// Types or pastes over the selection. One-line fields turn line breaks
  /// into spaces.
  pub fn insert(&mut self, s: &str) {
    let (a, b) = self.selection();
    let mut add: Vec<char> = s
      .chars()
      .filter(|c| *c != '\r')
      .map(|c| if (c == '\n' && !self.multiline) || c == '\t' { ' ' } else { c })
      .collect();
    if self.max > 0 {
      let room = self.max.saturating_sub(self.chars.len() - (b - a));
      add.truncate(room);
    }
    self.chars.splice(a..b, add.iter().copied());
    self.caret = a + add.len();
    self.anchor = self.caret;
  }

  fn word_left(&self, mut i: usize) -> usize {
    while i > 0 && !is_word(self.chars[i - 1]) {
      i -= 1;
    }
    while i > 0 && is_word(self.chars[i - 1]) {
      i -= 1;
    }
    i
  }

  fn word_right(&self, mut i: usize) -> usize {
    let n = self.chars.len();
    while i < n && !is_word(self.chars[i]) {
      i += 1;
    }
    while i < n && is_word(self.chars[i]) {
      i += 1;
    }
    i
  }

  fn line_start(&self, i: usize) -> usize {
    self.chars[..i].iter().rposition(|c| *c == '\n').map_or(0, |p| p + 1)
  }

  fn line_end(&self, i: usize) -> usize {
    self.chars[i..].iter().position(|c| *c == '\n').map_or(self.chars.len(), |p| i + p)
  }

  fn remove(&mut self, a: usize, b: usize) {
    self.chars.drain(a..b);
    self.caret = a;
    self.anchor = a;
  }

  fn place(&mut self, to: usize, extend: bool) {
    self.caret = to.min(self.chars.len());
    if !extend {
      self.anchor = self.caret;
    }
  }

  /// WM_KEYDOWN (`ctrl` without AltGr).
  pub fn key(&mut self, vk: u16, ctrl: bool, shift: bool) -> Typed {
    let (a, b) = self.selection();
    match vk {
      0x1B => Typed::Escape,
      0x0D if self.multiline && !ctrl => {
        self.insert("\n");
        Typed::Changed
      }
      0x0D => Typed::Submit,
      0x08 => {
        if a != b {
          self.remove(a, b);
        } else if a > 0 {
          let from = if ctrl { self.word_left(a) } else { a - 1 };
          self.remove(from, a);
        } else {
          return Typed::Nothing;
        }
        Typed::Changed
      }
      0x2E => {
        if a != b {
          self.remove(a, b);
        } else if a < self.chars.len() {
          let to = if ctrl { self.word_right(a) } else { a + 1 };
          self.remove(a, to);
        } else {
          return Typed::Nothing;
        }
        Typed::Changed
      }
      0x25 => {
        let to = if !shift && a != b && !ctrl { a } else if ctrl { self.word_left(self.caret) } else { self.caret.saturating_sub(1) };
        self.place(to, shift);
        Typed::Moved
      }
      0x27 => {
        let to = if !shift && a != b && !ctrl { b } else if ctrl { self.word_right(self.caret) } else { self.caret + 1 };
        self.place(to, shift);
        Typed::Moved
      }
      0x24 => {
        let to = if self.multiline && !ctrl { self.line_start(self.caret) } else { 0 };
        self.place(to, shift);
        Typed::Moved
      }
      0x23 => {
        let to = if self.multiline && !ctrl { self.line_end(self.caret) } else { self.chars.len() };
        self.place(to, shift);
        Typed::Moved
      }
      // up / down: the line above / below at the same column
      0x26 if self.multiline => {
        let start = self.line_start(self.caret);
        if start == 0 {
          self.place(0, shift);
        } else {
          let col = self.caret - start;
          let prev = self.line_start(start - 1);
          self.place((prev + col).min(start - 1), shift);
        }
        Typed::Moved
      }
      0x28 if self.multiline => {
        let end = self.line_end(self.caret);
        if end >= self.chars.len() {
          self.place(self.chars.len(), shift);
        } else {
          let col = self.caret - self.line_start(self.caret);
          let next_end = self.line_end(end + 1);
          self.place((end + 1 + col).min(next_end), shift);
        }
        Typed::Moved
      }
      0x41 if ctrl => {
        self.anchor = 0;
        self.caret = self.chars.len();
        Typed::Moved
      }
      0x43 if ctrl && a != b && !self.password => Typed::Copy(self.selected()),
      0x58 if ctrl && a != b && !self.password => {
        let s = self.selected();
        self.remove(a, b);
        Typed::Copy(s)
      }
      0x56 if ctrl => Typed::Paste,
      _ => Typed::Nothing,
    }
  }

  /// WM_CHAR (UTF-16 units; control characters come as keys)
  pub fn char(&mut self, unit: u16) -> Typed {
    if (0xD800..0xDC00).contains(&unit) {
      self.surrogate = Some(unit);
      return Typed::Nothing;
    }
    let s = if (0xDC00..0xE000).contains(&unit) {
      match self.surrogate.take() {
        Some(high) => String::from_utf16_lossy(&[high, unit]),
        None => return Typed::Nothing,
      }
    } else {
      self.surrogate = None;
      if unit < 0x20 || unit == 0x7F {
        return Typed::Nothing;
      }
      String::from_utf16_lossy(&[unit])
    };
    self.insert(&s);
    Typed::Changed
  }

  /// The character index at a point of the last drawing (DIPs).
  fn index_at(&self, x: f32, y: f32) -> Option<usize> {
    let (layout, ox, oy) = self.layout.as_ref()?;
    let (mut trailing, mut inside) = (Default::default(), Default::default());
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    unsafe {
      layout.HitTestPoint(x - ox, y - oy, &mut trailing, &mut inside, &mut m).ok()?;
    }
    let mut utf16 = m.textPosition as usize + if trailing.as_bool() { m.length as usize } else { 0 };
    // UTF-16 position -> character index
    let mut i = 0;
    for c in self.display_chars() {
      let n = c.len_utf16();
      if utf16 < n {
        break;
      }
      utf16 -= n;
      i += 1;
    }
    Some(i.min(self.chars.len()))
  }

  /// Mouse press: places the caret (Shift: extends), starts a selection.
  pub fn press(&mut self, x: f32, y: f32, shift: bool) {
    if let Some(i) = self.index_at(x, y) {
      self.place(i, shift);
    }
    self.dragging = true;
  }

  /// Mouse move while pressed.
  pub fn drag(&mut self, x: f32, y: f32) -> bool {
    if !self.dragging {
      return false;
    }
    match self.index_at(x, y) {
      Some(i) if i != self.caret => {
        self.caret = i;
        true
      }
      _ => false,
    }
  }

  /// Double click: the word under the caret.
  pub fn select_word(&mut self) {
    let i = self.caret.min(self.chars.len());
    self.anchor = self.word_left(i);
    self.caret = self.word_right(i);
  }

  fn display_chars(&self) -> Vec<char> {
    if self.password {
      vec!['•'; self.chars.len()]
    } else {
      self.chars.clone()
    }
  }

  /// Draws the text in `r` (padding already taken off): placeholder when
  /// empty, the selection, and the caret when focused.
  pub fn paint(&mut self, p: &mut Painter, t: &Theme, r: Rect, style: TextStyle, color: Rgba, placeholder: &str, focused: bool) -> anyhow::Result<()> {
    let shown: String = self.display_chars().into_iter().collect();
    if shown.is_empty() {
      self.scroll_x = 0.0;
      self.scroll_y = 0.0;
      if !placeholder.is_empty() {
        if self.multiline {
          p.text_wrapped(placeholder, r, style, t.on_surface_variant, false)?;
        } else {
          p.text(placeholder, r, style, t.on_surface_variant, Align::Left, false)?;
        }
      }
      // an empty layout still places a caret for clicks
      let layout = p.layout("", style, r.w.max(1.0), r.h, false)?;
      self.layout = Some((layout, r.x, r.y));
      if focused {
        let line = style.size * 1.35;
        let top = if self.multiline { r.y } else { r.y + (r.h - line) / 2.0 };
        p.fill(Rect::new(r.x.round(), top, 1.5, line), t.primary)?;
      }
      return Ok(());
    }
    let layout = if self.multiline {
      let l = p.layout(&shown, style, r.w, 100_000.0, false)?;
      unsafe {
        l.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        l.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
        let line = style.size * 1.45;
        l.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, line * 0.8)?;
      }
      l
    } else {
      p.layout(&shown, style, 100_000.0, r.h, false)?
    };
    let chars = self.display_chars();
    let utf16_at = |i: usize| chars[..i.min(chars.len())].iter().map(|c| c.len_utf16()).sum::<usize>() as u32;
    let at = |i: usize| -> (f32, f32, f32) {
      let (mut x, mut y) = (0f32, 0f32);
      let mut m = DWRITE_HIT_TEST_METRICS::default();
      unsafe {
        let _ = layout.HitTestTextPosition(utf16_at(i), false, &mut x, &mut y, &mut m);
      }
      (x, y, m.height)
    };
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe {
      let _ = layout.GetMetrics(&mut metrics);
    }
    let (cx, cy, ch) = at(self.caret);
    // keep the caret in view
    if self.multiline {
      if cy + ch - self.scroll_y > r.h {
        self.scroll_y = cy + ch - r.h;
      } else if cy < self.scroll_y {
        self.scroll_y = cy;
      }
      self.scroll_y = self.scroll_y.clamp(0.0, (metrics.height - r.h).max(0.0));
      self.scroll_x = 0.0;
    } else {
      if cx - self.scroll_x > r.w - 2.0 {
        self.scroll_x = cx - r.w + 2.0;
      } else if cx < self.scroll_x {
        self.scroll_x = cx;
      }
      self.scroll_x = self.scroll_x.clamp(0.0, (metrics.widthIncludingTrailingWhitespace - r.w + 2.0).max(0.0));
      self.scroll_y = 0.0;
    }
    let ox = r.x - self.scroll_x;
    let oy = if self.multiline { r.y - self.scroll_y } else { r.y };
    unsafe {
      p.dc.PushAxisAlignedClip(&Rect::new(r.x - 1.0, r.y - 1.0, r.w + 3.0, r.h + 2.0).d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
    }
    let (a, b) = self.selection();
    if a != b && focused {
      // one rectangle per line the selection touches
      let mut count = 0u32;
      unsafe {
        let _ = layout.HitTestTextRange(utf16_at(a), utf16_at(b) - utf16_at(a), 0.0, 0.0, None, &mut count);
      }
      let mut ranges = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
      unsafe {
        let _ = layout.HitTestTextRange(utf16_at(a), utf16_at(b) - utf16_at(a), 0.0, 0.0, Some(&mut ranges), &mut count);
      }
      for m in ranges.iter().take(count as usize) {
        p.fill(Rect::new(ox + m.left, oy + m.top, m.width.max(3.0), m.height), t.primary.alpha(0.35))?;
      }
    }
    let brush = p.brush(color)?;
    unsafe {
      p.dc.DrawTextLayout(pt(ox, oy), &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
    }
    if focused {
      let top = if self.multiline { oy + cy } else { r.y + (r.h - ch) / 2.0 };
      p.fill(Rect::new((ox + cx).round(), top, 1.5, ch), t.primary)?;
    }
    unsafe {
      p.dc.PopAxisAlignedClip();
    }
    self.layout = Some((layout, ox, oy));
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn field(s: &str) -> TextField {
    let mut f = TextField::new(false);
    f.set(s);
    f
  }

  #[test]
  fn typing_replaces_the_selection() {
    let mut f = field("merhaba");
    f.key(0x41, true, false);
    f.char('ş' as u16);
    assert_eq!(f.text(), "ş");
    assert_eq!(f.caret, 1);
  }

  #[test]
  fn word_keys_and_backspace() {
    let mut f = field("bir iki üç");
    assert_eq!(f.key(0x08, true, false), Typed::Changed);
    assert_eq!(f.text(), "bir iki ");
    f.key(0x25, true, false);
    assert_eq!(f.caret, 4);
  }

  #[test]
  fn one_line_fields_submit_and_multiline_breaks() {
    let mut f = field("a");
    assert_eq!(f.key(0x0D, false, false), Typed::Submit);
    let mut m = TextField::new(true);
    m.set("a");
    assert_eq!(m.key(0x0D, false, false), Typed::Changed);
    assert_eq!(m.text(), "a\n");
  }

  #[test]
  fn up_and_down_keep_the_column() {
    let mut m = TextField::new(true);
    m.set("abcd\nxy\nlonger");
    m.place(3, false); // "abc|d"
    m.key(0x28, false, false);
    assert_eq!(m.caret, 7, "the shorter line's end");
    m.key(0x28, false, false);
    assert_eq!(m.caret, 10);
    m.key(0x26, false, false);
    assert_eq!(m.caret, 7);
  }

  #[test]
  fn the_limit_holds_while_pasting() {
    let mut f = TextField::new(false);
    f.max = 5;
    f.insert("1234567");
    assert_eq!(f.text(), "12345");
  }

  #[test]
  fn passwords_are_not_copied() {
    let mut f = field("gizli");
    f.password = true;
    f.key(0x41, true, false);
    assert_eq!(f.key(0x43, true, false), Typed::Nothing);
  }
}
