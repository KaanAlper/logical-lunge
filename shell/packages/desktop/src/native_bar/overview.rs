//! The Super menu (ii overview: the search box on top, the workspace grid
//! below), drawn like the bar on its UI thread. Sizes and colours from the
//! web edition's overview.css, results from `search.rs`, the
//! responsibility map in docs/native-overview.md.

use std::{collections::HashMap, os::windows::process::CommandExt, sync::atomic::{AtomicU64, Ordering}, time::{Duration, Instant}};

use serde_json::{json, Value};
use windows::{
  core::{Interface, HSTRING, PCWSTR},
  Foundation::Numerics::Matrix3x2,
  Win32::{
    Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      Direct2D::{
        Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED},
        ID2D1Bitmap1, ID2D1Factory, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
      },
      DirectComposition::{IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3},
      DirectWrite::{DWRITE_HIT_TEST_METRICS, DWRITE_READING_DIRECTION_RIGHT_TO_LEFT, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE},
      Gdi::{GetMonitorInfoW, MonitorFromPoint, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTOPRIMARY},
    },
    System::{
      DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData},
      LibraryLoader::GetModuleHandleW,
      Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    },
    UI::{
      HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
      Input::KeyboardAndMouse::{GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_MENU, VK_SHIFT},
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  anim::{self, POP_IN, SPRING_IN},
  ime,
  core_api,
  fonts::TextStyle,
  gfx::{self, pt, Gfx, Rect, Rgba},
  icons::{data_url_bytes, App},
  model,
  popup,
  search::{self, Act, Clip, Glyph, Item, Prefix},
  view::{Align, Painter, Theme},
  wm::{self, WmState},
  Layer, Msg, Ui, CLASS,
};

/// ii searchWidthCollapsed / searchWidth with the margins and the two buttons
const W_COLLAPSED: f32 = 356.0;
const W_EXPANDED: f32 = 560.0;
static FILE_SEARCH_GENERATION: AtomicU64 = AtomicU64::new(0);
/// bar (40) + elevationMargin (10): ii opens the overview under the bar
const TOP: f32 = 50.0;
const BAR: f32 = 56.0;
const RADIUS: f32 = 28.0;
/// room around the box for its shadow
const SHADOW: f32 = 16.0;
/// the surface holds the widest box with the longest list and its shadow
const SURF_W: f32 = W_EXPANDED + 2.0 * SHADOW;
const SURF_H: f32 = BAR + 1.0 + LIST_MAX + 2.0 * SHADOW;
/// .item: padding 6 + icon 35 + 6, margin-bottom 2
const ROW: f32 = 47.0;
const ROW_GAP: f32 = 2.0;
const LIST_PAD: f32 = 10.0;
const LIST_MAX: f32 = 600.0;
const TOOL: f32 = 40.0;
const GRID_SCALE: f32 = 0.18;
const GRID_COLS: usize = 5;
const GRID_ROWS: usize = 2;
const GRID_GAP: f32 = 5.0;
const GRID_PAD: f32 = 10.0;

/// the web menu's width (300 ms) and shape (path 300 ms, turn 400 ms)
/// transitions, both on the elementMove curve
const WIDTH_MS: f32 = 300.0;
const SHAPE_MS: f32 = 300.0;
const SHAPE_TURN_MS: f32 = 400.0;
/// repaints while they run (a timer of the menu's own window)
const TIMER_MORPH: usize = 0x4C4D;

const CF_UNICODETEXT: u32 = 13;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The core finds the Super menu by this title (Super, Super+V, focus guard).
pub const TITLE: &str = "lunge-overview";
/// test run next to the running menu (the core does not know it)
pub const TITLE_DEMO: &str = "lunge-overview (native demo)";

// ------------------------------------------------------------ text editing

/// A one-line text field: characters, the caret, and the other end of the
/// selection (`anchor == caret`: no selection), with its undo history.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Edit {
  pub chars: Vec<char>,
  pub caret: usize,
  pub anchor: usize,
  undo: Vec<Snap>,
  redo: Vec<Snap>,
  /// the last change, for grouping: typing a word is one undo step
  last: Option<Change>,
}

#[derive(Clone, Debug, PartialEq)]
struct Snap {
  chars: Vec<char>,
  caret: usize,
  anchor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Change {
  Type,
  Space,
  Back,
  Delete,
  Other,
}

const UNDO_MAX: usize = 100;

fn is_word(c: char) -> bool {
  c.is_alphanumeric() || c == '_'
}

/// word, space or punctuation: a double click selects a run of one class
fn class(c: char) -> u8 {
  if is_word(c) {
    0
  } else if c.is_whitespace() {
    1
  } else {
    2
  }
}

fn rtl_char(c: char) -> bool {
  matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

/// The paragraph direction a browser's `dir=auto` picks: the first strong
/// character's (Arabic, Hebrew ... right to left).
pub fn is_rtl(text: &str) -> bool {
  text.chars().find(|c| c.is_alphabetic()).is_some_and(rtl_char)
}

impl Edit {
  pub fn text(&self) -> String {
    self.chars.iter().collect()
  }

  /// The menu opened with `s`: no history.
  pub fn start(&mut self, s: &str) {
    *self = Edit::default();
    self.chars = s.chars().collect();
    self.caret = self.chars.len();
    self.anchor = self.caret;
  }

  /// Replaces the text (Esc, Tab, a prefix chip): one undo step.
  pub fn set(&mut self, s: &str) {
    let chars: Vec<char> = s.chars().collect();
    if chars != self.chars {
      self.record(Change::Other);
    }
    self.chars = chars;
    self.caret = self.chars.len();
    self.anchor = self.caret;
  }

  pub fn selection(&self) -> (usize, usize) {
    (self.caret.min(self.anchor), self.caret.max(self.anchor))
  }

  pub fn selected(&self) -> String {
    let (a, b) = self.selection();
    self.chars[a..b].iter().collect()
  }

  fn snap(&self) -> Snap {
    Snap { chars: self.chars.clone(), caret: self.caret, anchor: self.anchor }
  }

  fn restore(&mut self, s: Snap) {
    self.chars = s.chars;
    self.caret = s.caret;
    self.anchor = s.anchor;
  }

  /// Before a change: a new undo step unless it continues the last one
  /// (letters of a word, a run of spaces, Backspace or Delete held).
  fn record(&mut self, change: Change) {
    let continues = change != Change::Other && self.last == Some(change);
    if !continues {
      self.undo.push(self.snap());
      if self.undo.len() > UNDO_MAX {
        self.undo.remove(0);
      }
    }
    self.redo.clear();
    self.last = Some(change);
  }

  /// Ctrl+Z; false when there is nothing to undo.
  pub fn undo(&mut self) -> bool {
    let Some(s) = self.undo.pop() else { return false };
    self.redo.push(self.snap());
    self.restore(s);
    self.last = None;
    true
  }

  /// Ctrl+Y, Ctrl+Shift+Z
  pub fn redo(&mut self) -> bool {
    let Some(s) = self.redo.pop() else { return false };
    self.undo.push(self.snap());
    self.restore(s);
    self.last = None;
    true
  }

  /// Types or pastes `s` over the selection. Line breaks become spaces.
  pub fn insert(&mut self, s: &str) {
    let (a, b) = self.selection();
    let add: Vec<char> = s.chars().map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect();
    if add.is_empty() && a == b {
      return;
    }
    let change = match add.as_slice() {
      [c] if a == b && c.is_whitespace() => Change::Space,
      [_] if a == b => Change::Type,
      _ => Change::Other,
    };
    self.record(change);
    self.chars.splice(a..b, add.iter().copied());
    self.caret = a + add.len();
    self.anchor = self.caret;
  }

  /// Start of the word left of `i` (Ctrl+Left, Ctrl+Backspace).
  fn word_left(&self, mut i: usize) -> usize {
    while i > 0 && !is_word(self.chars[i - 1]) {
      i -= 1;
    }
    while i > 0 && is_word(self.chars[i - 1]) {
      i -= 1;
    }
    i
  }

  /// End of the word right of `i` (Ctrl+Right, Ctrl+Delete).
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

  fn remove(&mut self, a: usize, b: usize) {
    self.chars.drain(a..b);
    self.caret = a;
    self.anchor = a;
  }

  pub fn backspace(&mut self, word: bool) {
    let (a, b) = self.selection();
    if a != b {
      self.record(Change::Other);
      self.remove(a, b);
    } else if a > 0 {
      self.record(if word { Change::Other } else { Change::Back });
      let from = if word { self.word_left(a) } else { a - 1 };
      self.remove(from, a);
    }
  }

  pub fn delete(&mut self, word: bool) {
    let (a, b) = self.selection();
    if a != b {
      self.record(Change::Other);
      self.remove(a, b);
    } else if a < self.chars.len() {
      self.record(if word { Change::Other } else { Change::Delete });
      let to = if word { self.word_right(a) } else { a + 1 };
      self.remove(a, to);
    }
  }

  /// Arrow keys: by a character or a word; `extend` keeps the anchor. Without
  /// `extend` a selection collapses to its side first.
  pub fn left(&mut self, word: bool, extend: bool) {
    let (a, b) = self.selection();
    self.caret = if !extend && a != b && !word { a } else if word { self.word_left(self.caret) } else { self.caret.saturating_sub(1) };
    if !extend {
      self.anchor = self.caret;
    }
    self.last = None;
  }

  pub fn right(&mut self, word: bool, extend: bool) {
    let (a, b) = self.selection();
    let n = self.chars.len();
    self.caret = if !extend && a != b && !word { b } else if word { self.word_right(self.caret) } else { (self.caret + 1).min(n) };
    if !extend {
      self.anchor = self.caret;
    }
    self.last = None;
  }

  pub fn home(&mut self, extend: bool) {
    self.place(0, extend);
  }

  pub fn end(&mut self, extend: bool) {
    self.place(self.chars.len(), extend);
  }

  /// The caret to `i` (a click; with Shift or a drag the selection grows).
  pub fn place(&mut self, i: usize, extend: bool) {
    self.caret = i.min(self.chars.len());
    if !extend {
      self.anchor = self.caret;
    }
    self.last = None;
  }

  /// Double click: the run of word characters (or spaces, or punctuation)
  /// at `i`.
  pub fn select_word(&mut self, i: usize) {
    let n = self.chars.len();
    if n == 0 {
      return;
    }
    let i = i.min(n);
    // the character the click is on; at the end, the one before
    let at = if i < n && (i == 0 || class(self.chars[i]) == 0 || class(self.chars[i - 1]) != 0) { i } else { i - 1 };
    let k = class(self.chars[at]);
    let (mut a, mut b) = (at, at + 1);
    while a > 0 && class(self.chars[a - 1]) == k {
      a -= 1;
    }
    while b < n && class(self.chars[b]) == k {
      b += 1;
    }
    self.anchor = a;
    self.caret = b;
    self.last = None;
  }

  pub fn select_all(&mut self) {
    self.anchor = 0;
    self.caret = self.chars.len();
    self.last = None;
  }

  pub fn is_rtl(&self) -> bool {
    is_rtl(&self.text())
  }
}

// ------------------------------------------------------------ the window

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tool {
  Lens,
  SongRec,
}

pub struct Overview {
  pub hwnd: HWND,
  pub scale: f32,
  /// monitor rectangle (screen pixels); the window covers it
  monitor: RECT,
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  panel: Layer,
  surface_w: f32,
  surface_h: f32,
  cell_w: f32,
  cell_h: f32,
  pub shown: bool,
  /// the Windows context menu of an app is open (its helper has the focus)
  pub menu_open: bool,
  pub edit: Edit,
  results: Vec<Item>,
  files_query: String,
  files: Option<Result<Vec<crate::everything::FileHit>, String>>,
  sel: usize,
  /// first row shown (the list scrolls past 11 rows)
  first: usize,
  clips: Vec<Clip>,
  /// the clipboard history was asked for since the menu opened (once: not on
  /// every key typed after ";")
  clips_asked: bool,
  clip_images: HashMap<String, Option<ID2D1Bitmap1>>,
  pub songrec: bool,
  /// horizontal scroll of the text field (DIPs)
  scroll_x: f32,
  /// a high surrogate waiting for its pair (WM_CHAR)
  surrogate: Option<u16>,
  /// hover moves the selection only when the pointer really moved
  last_mouse: POINT,
  hover_tool: Option<Tool>,
  hover_chip: Option<&'static str>,
  // layout of the last paint, in DIPs from the window's top-left
  box_rect: Rect,
  tools: [(Rect, Tool); 2],
  /// prefix chips in the empty field (`;` clipboard, `#` files...)
  chips: Vec<(Rect, &'static str)>,
  rows: Vec<(Rect, usize)>,
  workspaces: Vec<(Rect, String)>,
  windows: Vec<(Rect, String, String)>,
  hover_workspace: Option<String>,
  hover_window: Option<String>,
  pressed_window: Option<(String, String, POINT)>,
  dragging: bool,
  drag_workspace: Option<String>,
  /// the text field (DIPs from the window's top-left) and the x of every
  /// caret position in its text layout, from the last paint
  field: Rect,
  xs: Vec<f32>,
  /// a drag in the field selects text
  selecting: bool,
  /// the last double click in the field: a third click selects everything
  last_dbl: Option<(Instant, POINT)>,
  /// the IME's unfinished text, drawn at the caret, and its own caret
  comp: String,
  comp_cursor: usize,
  /// the caret in client pixels (where the IME's candidate list opens)
  caret_px: POINT,
  line_px: i32,
  /// prefs.json "animations"
  pub animations: bool,
  /// the box grows when typing starts (the web menu's 300 ms width transition)
  width_from: f32,
  width_to: f32,
  width_start: Option<Instant>,
  /// the shape left of the field morphs into the next prefix's
  shape_from: Prefix,
  shape_to: Prefix,
  shape_start: Option<Instant>,
}

impl Drop for Overview {
  fn drop(&mut self) {
    unsafe {
      let _ = DestroyWindow(self.hwnd);
    }
  }
}

impl Overview {
  pub fn new(gfx: &Gfx, demo: bool, wm: &WmState) -> anyhow::Result<Self> {
    let (monitor, scale) = overview_monitor(wm);
    let dip_w = (monitor.right - monitor.left) as f32 / scale;
    let dip_h = (monitor.bottom - monitor.top) as f32 / scale;
    let cell_w = (dip_w * GRID_SCALE).round().min((dip_w - 2.0 * GRID_PAD - 4.0 * GRID_GAP - 2.0 * SHADOW) / GRID_COLS as f32);
    let cell_h = (dip_h * GRID_SCALE).round();
    let grid_w = 2.0 * GRID_PAD + GRID_COLS as f32 * cell_w + (GRID_COLS - 1) as f32 * GRID_GAP;
    let grid_h = 2.0 * GRID_PAD + GRID_ROWS as f32 * cell_h + (GRID_ROWS - 1) as f32 * GRID_GAP;
    let surface_w = SURF_W.max(grid_w + 2.0 * SHADOW);
    let surface_h = SURF_H.max(BAR + 10.0 + grid_h + 2.0 * SHADOW);
    unsafe {
      let title = if demo { TITLE_DEMO } else { TITLE };
      let hwnd = CreateWindowExW(
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        CLASS,
        &HSTRING::from(title),
        WS_POPUP,
        monitor.left,
        monitor.top,
        monitor.right - monitor.left,
        monitor.bottom - monitor.top,
        None,
        None,
        GetModuleHandleW(None)?,
        None,
      )?;
      let made = (|| -> windows::core::Result<(IDCompositionTarget, IDCompositionVisual2, Layer)> {
        let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
        let root = gfx.dcomp.CreateVisual()?;
        let panel = Layer::new(gfx, (surface_w * scale).ceil() as u32, (surface_h * scale).ceil() as u32)?;
        root.AddVisual(&panel.visual, false, None)?;
        target.SetRoot(&root)?;
        Ok((target, root, panel))
      })();
      let (target, root, panel) = match made {
        Ok(v) => v,
        Err(err) => {
          let _ = DestroyWindow(hwnd);
          return Err(err.into());
        }
      };
      Ok(Self {
        hwnd,
        scale,
        monitor,
        _target: target,
        _root: root,
        panel,
        surface_w,
        surface_h,
        cell_w,
        cell_h,
        shown: false,
        menu_open: false,
        edit: Edit::default(),
        results: Vec::new(),
        files_query: String::new(),
        files: None,
        sel: 0,
        first: 0,
        clips: Vec::new(),
        clips_asked: false,
        clip_images: HashMap::new(),
        songrec: false,
        scroll_x: 0.0,
        surrogate: None,
        last_mouse: POINT::default(),
        hover_tool: None,
        hover_chip: None,
        box_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        tools: [(Rect::new(0.0, 0.0, 0.0, 0.0), Tool::Lens), (Rect::new(0.0, 0.0, 0.0, 0.0), Tool::SongRec)],
        chips: Vec::new(),
        rows: Vec::new(),
        workspaces: Vec::new(),
        windows: Vec::new(),
        hover_workspace: None,
        hover_window: None,
        pressed_window: None,
        dragging: false,
        drag_workspace: None,
        field: Rect::new(0.0, 0.0, 0.0, 0.0),
        xs: Vec::new(),
        selecting: false,
        last_dbl: None,
        comp: String::new(),
        comp_cursor: 0,
        caret_px: POINT::default(),
        line_px: 0,
        animations: true,
        width_from: W_COLLAPSED,
        width_to: W_COLLAPSED,
        width_start: None,
        shape_from: Prefix::of(""),
        shape_to: Prefix::of(""),
        shape_start: None,
      })
    }
  }

  fn width_dip(&self) -> f32 {
    (self.monitor.right - self.monitor.left) as f32 / self.scale
  }

  /// The box's width for its text: narrow while empty.
  fn target_width(&self) -> f32 {
    if self.edit.chars.is_empty() && self.comp.is_empty() {
      W_COLLAPSED
    } else {
      W_EXPANDED
    }
  }

  /// The width now, part way through its transition (the elementMove curve
  /// overshoots a little; the surface's shadow margin has room for it).
  fn box_width(&self) -> f32 {
    let Some(start) = self.width_start else { return self.width_to };
    let t = start.elapsed().as_secs_f32() * 1000.0 / WIDTH_MS;
    if t >= 1.0 {
      return self.width_to;
    }
    let w = self.width_from + (self.width_to - self.width_from) * SPRING_IN.at(t);
    w.min(self.surface_w - 8.0)
  }

  /// Starts the width and shape transitions when the text calls for a new
  /// width or prefix (repainted on a timer until they end).
  fn follow_text(&mut self) {
    let now = Instant::now();
    let width = self.target_width();
    if width != self.width_to {
      self.width_from = self.box_width();
      self.width_to = width;
      self.width_start = self.animations.then_some(now);
    }
    let prefix = Prefix::of(&self.edit.text());
    if prefix != self.shape_to {
      self.shape_from = if self.animations { self.shape_to } else { prefix };
      self.shape_to = prefix;
      self.shape_start = self.animations.then_some(now);
    }
    if self.animating() {
      unsafe {
        let _ = SetTimer(self.hwnd, TIMER_MORPH, 16, None);
      }
    }
  }

  /// A transition is still running.
  fn animating(&self) -> bool {
    let running = |s: Option<Instant>, ms: f32| s.is_some_and(|s| s.elapsed().as_secs_f32() * 1000.0 < ms);
    running(self.width_start, WIDTH_MS) || running(self.shape_start, SHAPE_TURN_MS)
  }

  /// Snaps the transitions to their end (the menu opens without them).
  fn settle(&mut self) {
    self.width_to = self.target_width();
    self.width_from = self.width_to;
    self.width_start = None;
    self.shape_to = Prefix::of(&self.edit.text());
    self.shape_from = self.shape_to;
    self.shape_start = None;
  }

  /// Rows that fit in the list (max-height 600, padding 10).
  fn visible_rows(&self) -> usize {
    (((LIST_MAX - 2.0 * LIST_PAD) + ROW_GAP) / (ROW + ROW_GAP)).floor() as usize
  }

  fn list_height(&self) -> f32 {
    if self.edit.chars.is_empty() || self.results.is_empty() {
      return 0.0;
    }
    let n = self.results.len().min(self.visible_rows()) as f32;
    1.0 + 2.0 * LIST_PAD + n * ROW + (n - 1.0).max(0.0) * ROW_GAP
  }

  /// New results for the text; the clipboard mode asks for the history.
  /// Returns true when the clipboard list should be (re)loaded.
  pub fn refresh(&mut self, apps: &[App], hour12: bool) -> bool {
    let text = self.edit.text();
    let clip_mode = Prefix::of(&text) == Prefix::Clip;
    if self.files_query != text {
      self.files_query = text.clone();
      self.files = None;
    }
    self.results = search::results(&text, apps, &self.clips, &|t| model::clock_at(t, hour12));
    self.add_file_results();
    self.sel = 0;
    self.first = 0;
    clip_mode
  }

  fn add_file_results(&mut self) {
    let Some(result) = &self.files else { return };
    match result {
      Ok(hits) => {
        if Prefix::of(&self.files_query) == Prefix::File { self.results.clear(); }
        let files = search::file_items(hits);
        let at = self.results.iter().position(|it| it.key == "run").unwrap_or(self.results.len());
        self.results.splice(at..at, files);
        if self.results.is_empty() && Prefix::of(&self.files_query) == Prefix::File {
          let mut item = search::results("#", &[], &[], &|_| String::new()).remove(0);
          item.name = "Dosya bulunamadı".into();
          self.results.push(item);
        }
      }
      Err(error) => {
        // the reason (not installed, did not open, not responding) is shown as is
        if Prefix::of(&self.files_query) == Prefix::File {
          self.results = search::results("#", &[], &[], &|_| String::new());
          self.results[0].name = error.clone();
        }
      }
    }
  }

  fn set_files(&mut self, query: &str, result: Result<Vec<crate::everything::FileHit>, String>) -> bool {
    if !self.shown || self.edit.text() != query { return false; }
    self.files = Some(result);
    self.results.retain(|it| !it.key.starts_with("file:") && it.key != "file-hint");
    self.add_file_results();
    self.sel = self.sel.min(self.results.len().saturating_sub(1));
    self.keep_visible();
    true
  }

  pub fn set_clips(&mut self, clips: Vec<Clip>, apps: &[App], hour12: bool) {
    self.clips = clips;
    self.clip_images.retain(|id, _| self.clips.iter().any(|c| &c.id == id));
    let sel = self.sel;
    self.refresh(apps, hour12);
    self.sel = sel.min(self.results.len().saturating_sub(1));
    self.keep_visible();
  }

  fn keep_visible(&mut self) {
    let n = self.visible_rows();
    if self.sel < self.first {
      self.first = self.sel;
    } else if self.sel >= self.first + n {
      self.first = self.sel + 1 - n;
    }
  }

  fn select(&mut self, i: usize) -> bool {
    if self.results.is_empty() {
      return false;
    }
    let i = i.min(self.results.len() - 1);
    if i == self.sel {
      return false;
    }
    self.sel = i;
    self.keep_visible();
    true
  }

  pub fn selected(&self) -> Option<&Item> {
    self.results.get(self.sel)
  }

  // ---- drawing

  /// Draws the box (search bar and results) into its surface and places it.
  pub fn paint(&mut self, gfx: &Gfx, p: &mut Painter, t: &Theme, wm: &WmState, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
    self.follow_text();
    let w = self.box_width();
    let h = BAR + self.list_height();
    // the box is centred on the monitor; the surface is placed around it
    let left = (self.width_dip() - self.surface_w) / 2.0;
    let top = TOP - SHADOW;
    unsafe {
      self.panel.visual.SetOffsetX2((left * self.scale).round())?;
      self.panel.visual.SetOffsetY2((top * self.scale).round())?;
    }
    let bx = Rect::new((self.surface_w - w) / 2.0, SHADOW, w, h);
    self.box_rect = Rect::new(left + bx.x, top + bx.y, w, h);

    popup::frame_shadow(p, bx, RADIUS)?;
    p.fill_round(bx, RADIUS, t.surface_container)?;

    // search bar: shape, field, Lens, song recognition
    let prefix = Prefix::of(&self.edit.text());
    let shape_c = (bx.x + 10.0 + 20.0, bx.y + BAR / 2.0);
    let progress = |start: Option<Instant>, ms: f32| {
      start.map_or(1.0, |s| SPRING_IN.at(s.elapsed().as_secs_f32() * 1000.0 / ms))
    };
    let (k, turn) = (progress(self.shape_start, SHAPE_MS), progress(self.shape_start, SHAPE_TURN_MS));
    shape(p, self.shape_from, self.shape_to, k, turn, shape_c.0, shape_c.1, t.primary_container)?;
    p.icon(prefix.icon(), shape_c.0, shape_c.1, 22.0, false, t.on_primary_container)?;

    let songrec = Rect::new(bx.right() - 4.0 - TOOL, bx.y + (BAR - TOOL) / 2.0, TOOL, TOOL);
    let lens = Rect::new(songrec.x - 6.0 - TOOL, songrec.y, TOOL, TOOL);
    for (r, tool, icon) in [(lens, Tool::Lens, "image_search"), (songrec, Tool::SongRec, "music_cast")] {
      let on = tool == Tool::SongRec && self.songrec;
      if on {
        p.fill_circle(r.x + TOOL / 2.0, r.y + TOOL / 2.0, TOOL / 2.0, t.primary)?;
      } else if self.hover_tool == Some(tool) {
        p.fill_circle(r.x + TOOL / 2.0, r.y + TOOL / 2.0, TOOL / 2.0, t.surface_container_high)?;
      }
      p.icon(icon, r.x + TOOL / 2.0, r.y + TOOL / 2.0, 22.0, false, if on { t.on_primary } else { t.on_surface_variant })?;
    }
    self.tools = [
      (Rect::new(left + lens.x, top + lens.y, lens.w, lens.h), Tool::Lens),
      (Rect::new(left + songrec.x, top + songrec.y, songrec.w, songrec.h), Tool::SongRec),
    ];

    let field = Rect::new(bx.x + 10.0 + 40.0 + 6.0, bx.y + (BAR - 40.0) / 2.0, lens.x - 6.0 - (bx.x + 56.0), 40.0);
    self.field = Rect::new(left + field.x, top + field.y, field.w, field.h);
    self.paint_field(p, t, field, tr)?;
    self.paint_chips(p, t, field, left, top, tr)?;

    // results
    self.rows.clear();
    if self.list_height() > 0.0 {
      let sep_y = bx.y + BAR;
      p.fill(Rect::new(bx.x, sep_y, w, 1.0), t.outline_variant)?;
      let n = self.visible_rows();
      let last = (self.first + n).min(self.results.len());
      let mut y = sep_y + 1.0 + LIST_PAD;
      for i in self.first..last {
        let r = Rect::new(bx.x + 10.0, y, w - 20.0, ROW);
        self.paint_row(gfx, p, t, r, i, tr)?;
        self.rows.push((Rect::new(left + r.x, top + r.y, r.w, r.h), i));
        y += ROW + ROW_GAP;
      }
      // scroll thumb (6 px, outline-variant) when the list is longer than the box
      if self.results.len() > n {
        let track = Rect::new(bx.right() - 8.0, sep_y + 1.0 + LIST_PAD, 6.0, self.list_height() - 1.0 - 2.0 * LIST_PAD);
        let k = n as f32 / self.results.len() as f32;
        let th = (track.h * k).max(24.0);
        let ty = track.y + (track.h - th) * (self.first as f32 / (self.results.len() - n) as f32);
        p.fill_round(Rect::new(track.x, ty, 6.0, th), 3.0, t.outline_variant)?;
      }
    }
    self.paint_grid(gfx, p, t, wm, left, top)?;
    Ok(())
  }

  /// The empty-search workspace grid (the web menu's overview.html WorkspaceOverview).
  fn paint_grid(&mut self, gfx: &Gfx, p: &mut Painter, t: &Theme, wm: &WmState, left: f32, top: f32) -> anyhow::Result<()> {
    self.workspaces.clear();
    self.windows.clear();
    if !self.edit.chars.is_empty() || !wm.connected {
      return Ok(());
    }
    let grid_w = 2.0 * GRID_PAD + GRID_COLS as f32 * self.cell_w + (GRID_COLS - 1) as f32 * GRID_GAP;
    let grid_h = 2.0 * GRID_PAD + GRID_ROWS as f32 * self.cell_h + (GRID_ROWS - 1) as f32 * GRID_GAP;
    let gx = (self.surface_w - grid_w) / 2.0;
    let gy = SHADOW + BAR + 10.0;
    p.fill_round(Rect::new(gx, gy, grid_w, grid_h), 23.0, t.layer0)?;
    let current = wm.focused_workspace().and_then(|w| w.name.parse::<u32>().ok()).unwrap_or(1);
    let base = (current.saturating_sub(1) / (GRID_COLS * GRID_ROWS) as u32) * (GRID_COLS * GRID_ROWS) as u32;
    for i in 0..GRID_COLS * GRID_ROWS {
      let name = (base + i as u32 + 1).to_string();
      let x = gx + GRID_PAD + (i % GRID_COLS) as f32 * (self.cell_w + GRID_GAP);
      let y = gy + GRID_PAD + (i / GRID_COLS) as f32 * (self.cell_h + GRID_GAP);
      let cell = Rect::new(x, y, self.cell_w, self.cell_h);
      let hot = self.hover_workspace.as_deref() == Some(name.as_str());
      let drop = self.drag_workspace.as_deref() == Some(name.as_str());
      p.fill_round(cell, 12.0, if drop { t.sec_container } else if hot { t.layer1_hover } else { t.layer1 })?;
      if current.to_string() == name {
        p.stroke_round(cell.inset(1.0, 1.0), 11.0, t.primary, 2.0)?;
      }
      p.text(&name, cell, TextStyle { size: 40.0, weight: 600.0 }, t.on_layer1.alpha(0.10), Align::Center, false)?;
      self.workspaces.push((Rect::new(left + x, top + y, cell.w, cell.h), name.clone()));
      let Some((monitor, workspace)) = wm.monitors.iter().find_map(|m| m.workspaces.iter().find(|w| w.name == name).map(|w| (m, w))) else { continue };
      if monitor.width <= 0 || monitor.height <= 0 { continue; }
      let sx = cell.w / monitor.width as f32;
      let sy = cell.h / monitor.height as f32;
      for win in &workspace.windows {
        let wx = ((win.x - monitor.x) as f32 * sx).clamp(0.0, cell.w - 8.0);
        let wy = ((win.y - monitor.y) as f32 * sy).clamp(0.0, cell.h - 8.0);
        let ww = (win.width as f32 * sx).max(8.0).min(cell.w - wx);
        let wh = (win.height as f32 * sy).max(8.0).min(cell.h - wy);
        let wr = Rect::new(x + wx, y + wy, ww, wh);
        let highlighted = self.hover_window.as_deref() == Some(win.id.as_str());
        p.fill_round(wr, 8.0, t.surface_container_high)?;
        p.stroke_round(wr.inset(0.5, 0.5), 8.0, if highlighted || win.has_focus { t.primary } else { t.border }, 1.0)?;
        let icon = Rect::new(wr.x + (wr.w - 28.0) / 2.0, wr.y + (wr.h - 28.0) / 2.0, 28.0, 28.0);
        let (bmp, request) = p.icons.for_window(gfx, &win.process, win.handle);
        if let Some(handle) = request { p.requests.push(handle); }
        if let Some(bmp) = bmp {
          p.image(&bmp, contain(&bmp, icon));
        } else {
          p.icon("web_asset", icon.x + 14.0, icon.y + 14.0, 22.0, false, t.on_surface_variant)?;
        }
        self.windows.push((Rect::new(left + wr.x, top + wr.y, wr.w, wr.h), name.clone(), win.id.clone()));
      }
    }
    Ok(())
  }

  /// The search modes a prefix opens, as chips at the end of the empty field:
  /// typing `#` is not something to know beforehand. A click types it.
  fn paint_chips(&mut self, p: &mut Painter, t: &Theme, field: Rect, left: f32, top: f32, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
    const CHIPS: [(&str, &str); 4] = [(";", "Pano"), ("#", "Dosyalar"), ("=", "Hesap"), ("?", "Web")];
    self.chips.clear();
    if !self.edit.chars.is_empty() {
      return Ok(());
    }
    let style = TextStyle { size: 12.5, weight: 450.0 };
    let key = TextStyle { size: 13.0, weight: 650.0 };
    let placeholder = p.measure(&tr("Ara, hesapla veya çalıştır"), TextStyle { size: 15.0, weight: 450.0 })?;
    let mut right = field.right();
    let mut placed = Vec::new();
    for (prefix, label) in CHIPS {
      let text = tr(label);
      let (kw, lw) = (p.measure(prefix, key)?, p.measure(&text, style)?);
      let w = 12.0 + kw + 6.0 + lw + 12.0;
      // only what fits after the placeholder, the first ones first
      if right - w < field.x + placeholder + 16.0 {
        break;
      }
      placed.push((prefix, text, kw, lw, w));
      right -= w + 6.0;
    }
    let mut x = right + 6.0;
    for (prefix, text, kw, lw, w) in placed.into_iter().rev() {
      let r = Rect::new(x, field.y + (field.h - 28.0) / 2.0, w, 28.0);
      let hot = self.hover_chip == Some(prefix);
      p.fill_round(r, 14.0, if hot { t.surface_container_high } else { t.surface_container_high.alpha(0.55) })?;
      p.text(prefix, Rect::new(r.x + 12.0, r.y, kw + 1.0, r.h), key, t.primary, Align::Left, false)?;
      p.text(&text, Rect::new(r.x + 12.0 + kw + 6.0, r.y, lw + 1.0, r.h), style, t.on_surface_variant, Align::Left, false)?;
      self.chips.push((Rect::new(left + r.x, top + r.y, r.w, r.h), prefix));
      x += w + 6.0;
    }
    Ok(())
  }

  fn paint_field(&mut self, p: &mut Painter, t: &Theme, r: Rect, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
    let style = TextStyle { size: 15.0, weight: 450.0 };
    // the IME's unfinished text sits at the caret until it is committed
    let at = self.edit.caret;
    let mut shown: Vec<char> = self.edit.chars[..at].to_vec();
    shown.extend(self.comp.chars());
    shown.extend(self.edit.chars[at..].iter().copied());
    let comp_len = self.comp.chars().count();
    let caret_at = at + self.comp_cursor.min(comp_len);
    self.set_caret_px(r.x, r);
    if shown.is_empty() {
      p.text(&tr("Ara, hesapla veya çalıştır"), r, style, t.on_surface_variant, Align::Left, false)?;
      caret(p, r.x, r, t)?;
      self.scroll_x = 0.0;
      self.xs.clear();
      return Ok(());
    }
    let text: String = shown.iter().collect();
    let rtl = is_rtl(&text);
    let layout = p.layout(&text, style, 100_000.0, r.h, false)?;
    let full_w = Painter::width_of(&layout);
    // right to left (Arabic, Hebrew): the line ends at the field's right edge
    // and grows leftwards, as a browser's dir=auto field
    let layout_w = if rtl { full_w.max(r.w) + 1.0 } else { full_w };
    let utf16_at = |i: usize| shown[..i].iter().map(|c| c.len_utf16()).sum::<usize>() as u32;
    unsafe {
      if rtl {
        layout.SetMaxWidth(layout_w)?;
        layout.SetReadingDirection(DWRITE_READING_DIRECTION_RIGHT_TO_LEFT)?;
      }
      if comp_len > 0 {
        let range = DWRITE_TEXT_RANGE { startPosition: utf16_at(at), length: utf16_at(at + comp_len) - utf16_at(at) };
        layout.SetUnderline(true, range)?;
      }
    }
    let x_at = |i: usize| -> f32 {
      let (mut x, mut y) = (0f32, 0f32);
      let mut m = DWRITE_HIT_TEST_METRICS::default();
      unsafe {
        let _ = layout.HitTestTextPosition(utf16_at(i), false, &mut x, &mut y, &mut m);
      }
      x
    };
    let caret_x = x_at(caret_at);
    // keep the caret in view
    if caret_x - self.scroll_x > r.w - 2.0 {
      self.scroll_x = caret_x - r.w + 2.0;
    } else if caret_x < self.scroll_x {
      self.scroll_x = caret_x;
    }
    self.scroll_x = self.scroll_x.clamp(0.0, (layout_w - r.w + 2.0).max(0.0));
    // caret positions for the mouse (the text without a composition)
    self.xs = if comp_len == 0 { (0..=shown.len()).map(x_at).collect() } else { Vec::new() };
    unsafe {
      p.dc.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
    }
    let (a, b) = self.edit.selection();
    if a != b && comp_len == 0 {
      // one rectangle per run: mixed directions split a selection
      let mut runs = [DWRITE_HIT_TEST_METRICS::default(); 16];
      let mut count = 0u32;
      let got = unsafe { layout.HitTestTextRange(utf16_at(a), utf16_at(b) - utf16_at(a), 0.0, 0.0, Some(&mut runs), &mut count) };
      if got.is_ok() {
        for m in runs.iter().take(count as usize) {
          let sel = Rect::new(r.x + m.left - self.scroll_x, r.y + 9.0, m.width, r.h - 18.0);
          p.fill(sel, Rgba(t.primary.0, t.primary.1, t.primary.2, 0.35))?;
        }
      }
    }
    let brush = p.brush(t.on_layer0)?;
    // text layouts put the first line at the top: centre the line in the field
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe {
      let _ = layout.GetMetrics(&mut m);
    }
    let line_h = m.height;
    unsafe {
      p.dc.DrawTextLayout(
        pt(r.x - self.scroll_x, r.y + (r.h - line_h) / 2.0),
        &layout,
        &brush,
        D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
      );
      p.dc.PopAxisAlignedClip();
    }
    let x = r.x + caret_x - self.scroll_x;
    caret(p, x, r, t)?;
    self.set_caret_px(x, r);
    Ok(())
  }

  /// The caret (`x` in the surface) in client pixels, for the IME.
  fn set_caret_px(&mut self, x: f32, r: Rect) {
    let dx = self.field.x - r.x;
    let dy = self.field.y - r.y;
    self.caret_px = POINT { x: ((x + dx) * self.scale).round() as i32, y: ((r.y + dy + 8.0) * self.scale).round() as i32 };
    self.line_px = ((r.h - 16.0) * self.scale).round() as i32;
  }

  /// The caret position nearest to `x` (DIPs from the window) in the field.
  fn index_at(&self, x: f32) -> usize {
    let lx = x - self.field.x + self.scroll_x;
    self
      .xs
      .iter()
      .enumerate()
      .min_by(|(_, a), (_, b)| (*a - lx).abs().total_cmp(&(*b - lx).abs()))
      .map_or(self.edit.chars.len(), |(i, _)| i)
  }

  fn in_field(&self, x: f32, y: f32) -> bool {
    self.field.contains(x, y)
  }

  fn paint_row(&mut self, gfx: &Gfx, p: &mut Painter, t: &Theme, r: Rect, i: usize, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
    let item = self.results[i].clone();
    let selected = i == self.sel;
    if selected {
      p.fill_round(r, 17.0, t.primary_container)?;
    }
    let fg = if selected { t.on_primary_container } else { t.on_layer0 };
    // icon: 35 x 35
    let ico = Rect::new(r.x + 10.0, r.y + 6.0, 35.0, 35.0);
    let bmp = match &item.glyph {
      Glyph::App(a) => p.icons.app(gfx, *a),
      Glyph::Image(url) => {
        let key = item.clip_id.clone().unwrap_or_default();
        self.clip_images.entry(key).or_insert_with(|| data_url_bytes(url).and_then(|b| gfx.bitmap(&b).ok())).clone()
      }
      _ => None,
    };
    match (&item.glyph, bmp) {
      (_, Some(b)) => p.image(&b, contain(&b, ico)),
      (Glyph::Big(s), None) => {
        p.text(s, ico, TextStyle { size: 22.0, weight: 500.0 }, fg, Align::Center, false)?;
      }
      (Glyph::Material(name), None) => p.icon(name, ico.x + 17.5, ico.y + 17.5, 26.0, false, fg)?,
      (_, None) => p.icon("apps", ico.x + 17.5, ico.y + 17.5, 26.0, false, fg)?,
    }
    // kind (· sub) over the name; the verb on the right while selected
    let tx = ico.right() + 10.0;
    let verb_w = if selected && !item.verb.is_empty() {
      p.measure(&tr(item.verb), TextStyle { size: 13.0, weight: 450.0 })? + 14.0
    } else {
      0.0
    };
    let tw = r.right() - tx - verb_w - 6.0;
    let kind = if item.sub.is_empty() { tr(item.kind) } else { format!("{} · {}", tr(item.kind), tr(&item.sub)) };
    let kind_c = if selected { Rgba(fg.0, fg.1, fg.2, 0.8) } else { t.on_surface_variant };
    p.text(&kind, Rect::new(tx, r.y + 6.0, tw, 16.0), TextStyle { size: 12.0, weight: 450.0 }, kind_c, Align::Left, false)?;
    let name_r = Rect::new(tx, r.y + 22.0, tw, 20.0);
    let name = if item.tr_name || (item.key == "sh" && item.act == Act::None) { tr(&item.name) } else { item.name.clone() };
    match &item.highlight {
      Some(q) => highlighted(p, &name, q, name_r, fg, if selected { Rgba::hex(0xffffff) } else { t.primary })?,
      // commands and code: the monospace face (the web menu's .mono)
      None if item.mono => {
        p.text_mono(&name, name_r, TextStyle { size: 14.0, weight: 400.0 }, fg)?;
      }
      None => {
        p.text(&name, name_r, TextStyle { size: 15.0, weight: 450.0 }, fg, Align::Left, false)?;
      }
    }
    if verb_w > 0.0 {
      let vr = Rect::new(r.right() - verb_w, r.y, verb_w - 4.0, r.h);
      let vw = p.measure(&tr(item.verb), TextStyle { size: 13.0, weight: 450.0 })?;
      p.text(&tr(item.verb), Rect::new(vr.right() - vw - 4.0, r.y + (r.h - 18.0) / 2.0, vw + 2.0, 18.0), TextStyle { size: 13.0, weight: 450.0 }, fg, Align::Left, false)?;
    }
    Ok(())
  }

  // ---- input

  pub fn hit_row(&self, x: f32, y: f32) -> Option<usize> {
    self.rows.iter().find(|(r, _)| r.contains(x, y)).map(|(_, i)| *i)
  }

  pub fn hit_tool(&self, x: f32, y: f32) -> Option<Tool> {
    self.tools.iter().find(|(r, _)| r.contains(x, y)).map(|(_, t)| *t)
  }

  fn hit_chip(&self, x: f32, y: f32) -> Option<&'static str> {
    self.chips.iter().find(|(r, _)| r.contains(x, y)).map(|(_, prefix)| *prefix)
  }

  pub fn in_box(&self, x: f32, y: f32) -> bool {
    self.box_rect.contains(x, y)
  }

  fn hit_workspace(&self, x: f32, y: f32) -> Option<String> {
    self.workspaces.iter().find(|(r, _)| r.contains(x, y)).map(|(_, name)| name.clone())
  }

  fn hit_window(&self, x: f32, y: f32) -> Option<(String, String)> {
    self.windows.iter().rev().find(|(r, _, _)| r.contains(x, y)).map(|(_, ws, id)| (ws.clone(), id.clone()))
  }
}

/// What a key, a character or a click asks the owner to do.
#[derive(Debug, PartialEq)]
pub enum Do {
  Nothing,
  Redraw,
  /// the text changed: new results, then redraw
  Search,
  Hide,
  Run(Item),
  DeleteClip(String),
  Tool(Tool),
  Copy(String),
  Paste,
  Workspace(String),
  FocusWindow { workspace: String, id: String },
  MoveWindow { workspace: String, id: String },
  /// the Windows context menu of an app (its `shell:AppsFolder\...` path)
  Menu(String),
}

/// The Windows context menu belongs to app results only.
fn menu_path(item: &Item) -> Option<String> {
  match &item.act {
    Act::Launch(path) => Some(path.clone()),
    _ => None,
  }
}

impl Overview {
  /// WM_KEYDOWN
  pub fn key(&mut self, vk: u16) -> Do {
    // AltGr arrives as Ctrl+Alt: it types a character (@, €, ą), it is no
    // Ctrl shortcut
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 && GetKeyState(VK_MENU.0 as i32) >= 0 };
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    match vk {
      0x1B => {
        // Esc: first clears the text, then closes (as the web menu)
        if self.edit.chars.is_empty() {
          Do::Hide
        } else {
          self.edit.set("");
          Do::Search
        }
      }
      0x0D => self.selected().cloned().map(Do::Run).unwrap_or(Do::Nothing),
      0x26 => {
        let s = self.sel.saturating_sub(1);
        if self.select(s) {
          Do::Redraw
        } else {
          Do::Nothing
        }
      }
      0x28 => {
        let s = self.sel + 1;
        if self.select(s) {
          Do::Redraw
        } else {
          Do::Nothing
        }
      }
      0x09 => match self.selected() {
        Some(item) => {
          let name = item.name.clone();
          self.edit.set(&name);
          Do::Search
        }
        None => Do::Nothing,
      },
      0x2E => match self.selected().and_then(|i| i.clip_id.clone()) {
        Some(id) if !ctrl => Do::DeleteClip(id),
        _ => {
          self.edit.delete(ctrl);
          Do::Search
        }
      },
      0x08 => {
        self.edit.backspace(ctrl);
        Do::Search
      }
      // right to left text: the arrows move the way they point
      0x25 | 0x27 => {
        if (vk == 0x25) != self.edit.is_rtl() {
          self.edit.left(ctrl, shift);
        } else {
          self.edit.right(ctrl, shift);
        }
        Do::Redraw
      }
      0x24 => {
        self.edit.home(shift);
        Do::Redraw
      }
      0x23 => {
        self.edit.end(shift);
        Do::Redraw
      }
      0x41 if ctrl => {
        self.edit.select_all();
        Do::Redraw
      }
      0x43 if ctrl => {
        let s = self.edit.selected();
        if s.is_empty() {
          Do::Nothing
        } else {
          Do::Copy(s)
        }
      }
      0x58 if ctrl => {
        let s = self.edit.selected();
        if s.is_empty() {
          Do::Nothing
        } else {
          self.edit.backspace(false);
          Do::Copy(s)
        }
      }
      0x56 if ctrl => Do::Paste,
      0x5A if ctrl => {
        let changed = if shift { self.edit.redo() } else { self.edit.undo() };
        if changed { Do::Search } else { Do::Nothing }
      }
      0x59 if ctrl => {
        if self.edit.redo() { Do::Search } else { Do::Nothing }
      }
      // the menu key, Shift+F10: the selected app's context menu
      0x5D => self.selected().and_then(menu_path).map(Do::Menu).unwrap_or(Do::Nothing),
      0x79 if shift => self.selected().and_then(menu_path).map(Do::Menu).unwrap_or(Do::Nothing),
      _ => Do::Nothing,
    }
  }

  /// WM_CHAR (UTF-16 units; control characters come as keys)
  pub fn char(&mut self, unit: u16) -> Do {
    if (0xD800..0xDC00).contains(&unit) {
      self.surrogate = Some(unit);
      return Do::Nothing;
    }
    let s = if (0xDC00..0xE000).contains(&unit) {
      match self.surrogate.take() {
        Some(high) => String::from_utf16_lossy(&[high, unit]),
        None => return Do::Nothing,
      }
    } else {
      self.surrogate = None;
      if unit < 0x20 || unit == 0x7F {
        return Do::Nothing;
      }
      String::from_utf16_lossy(&[unit])
    };
    self.edit.insert(&s);
    Do::Search
  }

  /// WM_MOUSEMOVE (screen point, DIPs from the window): the row under a
  /// pointer that really moved becomes the selection (not when the list or
  /// the window moved under a resting pointer).
  pub fn mouse_move(&mut self, screen: POINT, x: f32, y: f32) -> Do {
    let moved = screen.x != self.last_mouse.x || screen.y != self.last_mouse.y;
    self.last_mouse = screen;
    if self.selecting {
      let i = self.index_at(x);
      if i == self.edit.caret {
        return Do::Nothing;
      }
      self.edit.place(i, true);
      return Do::Redraw;
    }
    let tool = self.hit_tool(x, y);
    let chip = self.hit_chip(x, y);
    let mut redraw = tool != self.hover_tool || chip != self.hover_chip;
    self.hover_tool = tool;
    self.hover_chip = chip;
    let hover_workspace = self.hit_workspace(x, y);
    let hover_window = self.hit_window(x, y).map(|(_, id)| id);
    redraw |= hover_workspace != self.hover_workspace || hover_window != self.hover_window;
    self.hover_workspace = hover_workspace.clone();
    self.hover_window = hover_window;
    if let Some((_, _, start)) = &self.pressed_window {
      if (screen.x - start.x).abs() + (screen.y - start.y).abs() > 6 {
        self.dragging = true;
      }
      if self.dragging {
        redraw |= self.drag_workspace != hover_workspace;
        self.drag_workspace = hover_workspace;
      }
    }
    if moved {
      if let Some(i) = self.hit_row(x, y) {
        redraw |= self.select(i);
      }
    }
    if redraw {
      Do::Redraw
    } else {
      Do::Nothing
    }
  }

  pub fn click(&mut self, x: f32, y: f32) -> Do {
    if let Some(tool) = self.hit_tool(x, y) {
      return Do::Tool(tool);
    }
    if let Some(prefix) = self.hit_chip(x, y) {
      self.edit.set(prefix);
      self.hover_chip = None;
      return Do::Search;
    }
    if self.in_field(x, y) {
      return self.field_press(x);
    }
    if let Some(i) = self.hit_row(x, y) {
      self.sel = i;
      return self.results.get(i).cloned().map(Do::Run).unwrap_or(Do::Nothing);
    }
    if let Some((workspace, id)) = self.hit_window(x, y) {
      let mut screen = POINT::default();
      unsafe {
        let _ = GetCursorPos(&mut screen);
        let _ = SetCapture(self.hwnd);
      }
      self.pressed_window = Some((workspace, id, screen));
      self.dragging = false;
      self.drag_workspace = None;
      return Do::Nothing;
    }
    if let Some(workspace) = self.hit_workspace(x, y) {
      return Do::Workspace(workspace);
    }
    // the backdrop closes (.backdrop onMouseDown)
    if !self.in_box(x, y) {
      return Do::Hide;
    }
    Do::Nothing
  }

  /// A press in the text field: places the caret (Shift: extends the
  /// selection) and starts a drag selection; the third click of a
  /// double click selects everything.
  fn field_press(&mut self, x: f32) -> Do {
    if !self.comp.is_empty() {
      return Do::Nothing;
    }
    let mut screen = POINT::default();
    unsafe {
      let _ = GetCursorPos(&mut screen);
    }
    let triple = self.last_dbl.take().is_some_and(|(at, p)| {
      let (tw, th) = unsafe { (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)) };
      at.elapsed().as_millis() <= unsafe { GetDoubleClickTime() } as u128 && (screen.x - p.x).abs() <= tw && (screen.y - p.y).abs() <= th
    });
    if triple {
      self.edit.select_all();
      return Do::Redraw;
    }
    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
    self.edit.place(self.index_at(x), shift);
    self.selecting = true;
    unsafe {
      let _ = SetCapture(self.hwnd);
    }
    Do::Redraw
  }

  /// WM_LBUTTONDBLCLK: a word in the field (elsewhere the first click did
  /// its work).
  pub fn double_click(&mut self, x: f32, y: f32) -> Do {
    if !self.in_field(x, y) || !self.comp.is_empty() || self.hit_chip(x, y).is_some() {
      return Do::Nothing;
    }
    let mut screen = POINT::default();
    unsafe {
      let _ = GetCursorPos(&mut screen);
    }
    self.last_dbl = Some((Instant::now(), screen));
    self.edit.select_word(self.index_at(x));
    Do::Redraw
  }

  /// The pointer's shape at (x, y): a text cursor over the field, a hand over
  /// what a click opens.
  pub fn cursor_at(&self, x: f32, y: f32) -> PCWSTR {
    if self.selecting {
      return IDC_IBEAM;
    }
    if self.hit_chip(x, y).is_some() || self.hit_tool(x, y).is_some() || self.hit_row(x, y).is_some() {
      return IDC_HAND;
    }
    if self.in_field(x, y) {
      return IDC_IBEAM;
    }
    if self.hit_window(x, y).is_some() || self.hit_workspace(x, y).is_some() {
      return IDC_HAND;
    }
    IDC_ARROW
  }

  pub fn release(&mut self) -> Do {
    if std::mem::take(&mut self.selecting) {
      unsafe { let _ = ReleaseCapture(); }
      return Do::Nothing;
    }
    let Some((workspace, id, _)) = self.pressed_window.take() else { return Do::Nothing };
    unsafe { let _ = ReleaseCapture(); }
    let target = self.drag_workspace.take();
    let dragged = std::mem::take(&mut self.dragging);
    if dragged {
      if let Some(to) = target.filter(|to| to != &workspace) {
        return Do::MoveWindow { workspace: to, id };
      }
      return Do::Redraw;
    }
    Do::FocusWindow { workspace, id }
  }

  /// Right click: selects the row; an app gets its Windows context menu.
  pub fn right_click(&mut self, x: f32, y: f32) -> Do {
    let Some(i) = self.hit_row(x, y) else { return Do::Nothing };
    self.sel = i;
    match self.results.get(i).and_then(menu_path) {
      Some(path) => Do::Menu(path),
      None => Do::Redraw,
    }
  }

  pub fn wheel(&mut self, delta: i32) -> Do {
    let n = self.visible_rows();
    if self.results.len() <= n {
      return Do::Nothing;
    }
    let max_first = self.results.len() - n;
    let step = if delta > 0 { -3i64 } else { 3 };
    let first = (self.first as i64 + step).clamp(0, max_first as i64) as usize;
    if first == self.first {
      return Do::Nothing;
    }
    self.first = first;
    // the selection stays in view
    self.sel = self.sel.clamp(first, first + n - 1);
    Do::Redraw
  }

  /// Opened again: an empty field (or the clipboard prefix for Super+V).
  pub fn reset(&mut self, text: &str) {
    self.edit.start(text);
    self.comp.clear();
    self.comp_cursor = 0;
    self.selecting = false;
    self.last_dbl = None;
    self.settle();
    self.clips_asked = false;
    self.results.clear();
    self.sel = 0;
    self.first = 0;
    self.scroll_x = 0.0;
    self.surrogate = None;
    self.hover_tool = None;
    self.hover_chip = None;
    self.hover_workspace = None;
    self.hover_window = None;
    self.pressed_window = None;
    self.dragging = false;
    self.drag_workspace = None;
    let mut p = POINT::default();
    unsafe {
      let _ = GetCursorPos(&mut p);
    }
    self.last_mouse = p;
  }

  pub fn monitor(&self) -> RECT {
    self.monitor
  }
}

// ------------------------------------------------------------ drawing helpers

fn caret(p: &mut Painter, x: f32, field: Rect, t: &Theme) -> anyhow::Result<()> {
  p.fill(Rect::new(x.round(), field.y + 10.0, 1.5, field.h - 20.0), t.primary)?;
  Ok(())
}

/// A bitmap fitted inside `r` (`object-fit: contain`).
fn contain(b: &ID2D1Bitmap1, r: Rect) -> Rect {
  let s = unsafe { b.GetSize() };
  let (w, h) = (s.width.max(1.0), s.height.max(1.0));
  let k = (r.w / w).min(r.h / h);
  let (dw, dh) = (w * k, h * k);
  Rect::new(r.x + (r.w - dw) / 2.0, r.y + (r.h - dh) / 2.0, dw, dh)
}

/// The name with the query's letters coloured and underlined (web `<u>`).
fn highlighted(p: &mut Painter, name: &str, query: &str, r: Rect, fg: Rgba, mark: Rgba) -> anyhow::Result<()> {
  let style = TextStyle { size: 15.0, weight: 450.0 };
  let layout = p.layout(name, style, r.w, r.h, false)?;
  let marks = search::highlight(name, query);
  let mark_brush = p.brush(mark)?;
  let mut pos = 0u32;
  for (c, on) in name.chars().zip(marks) {
    let len = c.len_utf16() as u32;
    if on {
      let range = DWRITE_TEXT_RANGE { startPosition: pos, length: len };
      unsafe {
        let _ = layout.SetDrawingEffect(&mark_brush, range);
        let _ = layout.SetUnderline(true, range);
      }
    }
    pos += len;
  }
  let brush = p.brush(fg)?;
  unsafe {
    p.dc.DrawTextLayout(pt(r.x, r.y), &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
  }
  Ok(())
}

/// ii MaterialShape by prefix (Cookie7Sided, Clover4Leaf, PixelCircle ...):
/// its outline as points around the centre (a 40 x 40 box), unturned, and
/// its turn. The action prefix is a pill.
fn outline(prefix: Prefix) -> ([(f32, f32); OUTLINE], f32) {
  let mut pts = [(0.0, 0.0); OUTLINE];
  let pill = prefix == Prefix::Action;
  let (n, depth) = match prefix {
    Prefix::App => (4.0, 0.22),
    Prefix::Math => (4.0, 0.2),
    Prefix::Shell => (12.0, 0.04),
    Prefix::Web => (10.0, 0.1),
    Prefix::Clip => (6.0, 0.16),
    _ => (7.0, 0.12),
  };
  for (i, pt) in pts.iter_mut().enumerate() {
    let a = i as f32 * std::f32::consts::TAU / OUTLINE as f32;
    let r = if pill { pill_radius(a) } else { 20.0 * (1.0 - depth + depth * (n * a).cos()) };
    *pt = (r * a.cos(), r * a.sin());
  }
  let turn = if prefix == Prefix::Math { std::f32::consts::FRAC_PI_4 } else { 0.0 };
  (pts, turn)
}

const OUTLINE: usize = 120;

/// Distance from the centre to a 40 x 20 pill's edge at angle `a`.
fn pill_radius(a: f32) -> f32 {
  let inside = |r: f32| {
    let (x, y) = (r * a.cos(), r * a.sin());
    let dx = (x.abs() - 10.0).max(0.0);
    dx * dx + y * y <= 100.0
  };
  let (mut lo, mut hi) = (0.0f32, 20.0f32);
  for _ in 0..20 {
    let mid = (lo + hi) / 2.0;
    if inside(mid) {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  lo
}

/// The shape around (cx, cy), `k` of the way from `from`'s outline to
/// `to`'s and `turn` of the way between their turns (the web menu's
/// path and transform transitions).
#[allow(clippy::too_many_arguments)]
fn shape(p: &mut Painter, from: Prefix, to: Prefix, k: f32, turn: f32, cx: f32, cy: f32, c: Rgba) -> anyhow::Result<()> {
  let (a, ta) = outline(from);
  let (b, tb) = outline(to);
  let rot = ta + (tb - ta) * turn;
  let (sin, cos) = rot.sin_cos();
  let brush = p.brush(c)?;
  unsafe {
    let factory: ID2D1Factory = p.dc.GetFactory()?;
    let geo = factory.CreatePathGeometry()?;
    let sink = geo.Open()?;
    for i in 0..OUTLINE {
      let x = a[i].0 + (b[i].0 - a[i].0) * k;
      let y = a[i].1 + (b[i].1 - a[i].1) * k;
      let at = pt(cx + x * cos - y * sin, cy + x * sin + y * cos);
      if i == 0 {
        sink.BeginFigure(at, D2D1_FIGURE_BEGIN_FILLED);
      } else {
        sink.AddLine(at);
      }
    }
    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    sink.Close()?;
    p.dc.FillGeometry(&geo, &brush, None);
  }
  Ok(())
}

// ------------------------------------------------------------ system helpers

/// Match the web menu's focused monitor, falling back to the primary one
/// before the window manager sends its first state.
fn overview_monitor(wm: &WmState) -> (RECT, f32) {
  let point = wm.monitors.iter().find(|m| m.has_focus).map(|m| POINT {
    x: m.x + m.width / 2,
    y: m.y + m.height / 2,
  }).unwrap_or(POINT { x: 0, y: 0 });
  unsafe {
    let mon = MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(mon, &mut mi);
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    (mi.rcMonitor, dx as f32 / 96.0)
  }
}

fn set_clipboard(hwnd: HWND, text: &str) -> bool {
  let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
  unsafe {
    if OpenClipboard(hwnd).is_err() {
      return false;
    }
    let ok = (|| -> windows::core::Result<()> {
      EmptyClipboard()?;
      let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
      let dst = GlobalLock(mem) as *mut u16;
      if dst.is_null() {
        return Err(windows::core::Error::from_win32());
      }
      std::ptr::copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
      let _ = GlobalUnlock(mem);
      SetClipboardData(CF_UNICODETEXT, HANDLE(mem.0))?;
      Ok(())
    })()
    .is_ok();
    let _ = CloseClipboard();
    ok
  }
}

fn clipboard_text(hwnd: HWND) -> Option<String> {
  unsafe {
    OpenClipboard(hwnd).ok()?;
    let text = (|| {
      let h = GetClipboardData(CF_UNICODETEXT).ok()?;
      let mem = HGLOBAL(h.0);
      let src = GlobalLock(mem) as *const u16;
      if src.is_null() {
        return None;
      }
      let mut n = 0;
      while *src.add(n) != 0 {
        n += 1;
      }
      let s = String::from_utf16_lossy(std::slice::from_raw_parts(src, n));
      let _ = GlobalUnlock(mem);
      Some(s)
    })();
    let _ = CloseClipboard();
    text
  }
}

fn spawn(program: &str, args: &[&str]) {
  let _ = std::process::Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).spawn();
}

/// The song recognition child of `run`, or (None) whichever there is.
fn take_songrec(run: Option<u64>) -> Option<std::process::Child> {
  let mut slot = super::SONGREC_CHILD.lock().ok()?;
  if let (Some((owner, _)), Some(want)) = (slot.as_ref(), run) {
    if *owner != want {
      return None;
    }
  }
  slot.take().map(|(_, child)| child)
}

/// `lunge.exe --songrec ...` kept where a second press finds and kills it
/// (the recognizer goes with it: the core holds both in one job object).
fn songrec_listen(run: u64) -> Option<Value> {
  use std::io::Read;
  let exe = core_api::core_exe()?;
  let mut child = std::process::Command::new(exe)
    .args(["--songrec", "-i", "2", "-t", "30", "-s", "monitor"])
    .stdout(std::process::Stdio::piped())
    .creation_flags(CREATE_NO_WINDOW)
    .spawn()
    .ok()?;
  let mut out = child.stdout.take()?;
  if let Ok(mut slot) = super::SONGREC_CHILD.lock() {
    *slot = Some((run, child));
  }
  let mut text = String::new();
  let _ = out.read_to_string(&mut text);
  if let Some(mut child) = take_songrec(Some(run)) {
    let _ = child.wait();
  }
  serde_json::from_str(text.trim().lines().last()?).ok()
}

/// `lunge.exe <args>`, waited for, its stdout (last line) as JSON.
fn core_json(args: &[&str]) -> Option<Value> {
  let exe = core_api::core_exe()?;
  let out = std::process::Command::new(exe).args(args).creation_flags(CREATE_NO_WINDOW).output().ok()?;
  let text = String::from_utf8_lossy(&out.stdout);
  serde_json::from_str(text.trim().lines().last()?).ok()
}

// ------------------------------------------------------------ the owner (bar UI thread)

impl Ui {
  /// Create the hidden native menu before startup widgets are selected, so
  /// the core's Super shortcut can find it by its existing window title.
  pub(super) fn overview_prepare_window(&mut self) -> bool {
    if self.overview.is_none() {
      match Overview::new(&self.gfx, self.demo, &self.model.wm) {
        Ok(o) => {
          super::OVERVIEW_HWND.store(o.hwnd.0 as isize, std::sync::atomic::Ordering::Release);
          self.overview = Some(o);
        }
        Err(err) => {
          tracing::error!("Super menu: {:?}", err);
          return false;
        }
      }
    }
    true
  }

  /// Display geometry and DirectComposition surfaces both belong to the
  /// window. Recreate them together after a monitor or graphics reset.
  pub(super) fn overview_recreate_window(&mut self) {
    if !self.native_overview { return; }
    self.overview_hide();
    super::OVERVIEW_HWND.store(0, std::sync::atomic::Ordering::Release);
    self.overview = None;
    self.overview_prepare_window();
  }

  /// Focus can move to another display while the menu is hidden. Its window
  /// and surface must follow that display's size and DPI before the next Super.
  pub(super) fn overview_sync_monitor(&mut self) {
    if !self.native_overview { return; }
    let Some(o) = self.overview.as_ref() else { return };
    if o.shown { return; }
    let (rect, scale) = overview_monitor(&self.model.wm);
    if o.monitor() != rect || (o.scale - scale).abs() > 0.001 {
      self.overview_recreate_window();
    }
  }

  pub(super) fn overview_open(&mut self, mode: &str) {
    if !self.overview_prepare(mode) { return; }
    // started just before the window shows: its first frame is the
    // transparent one
    if let Err(err) = self.overview_enter() {
      tracing::debug!("Super menu: entrance: {:?}", err);
    }
    let Some(o) = self.overview.as_mut() else { return };
    unsafe {
      let _ = ShowWindow(o.hwnd, SW_SHOW);
      let _ = SetForegroundWindow(o.hwnd);
    }
  }

  /// The web menu's entrance (`ovIn`, 260 ms, --emphDecel): from
  /// transparent, 14 px higher and 98 % of its size to its place, played by
  /// the compositor. With animations off it is simply there.
  fn overview_enter(&self) -> windows::core::Result<()> {
    let Some(o) = self.overview.as_ref() else { return Ok(()) };
    let dcomp = &self.gfx.dcomp;
    let visual: IDCompositionVisual3 = o.panel.visual.cast()?;
    unsafe {
      if !self.model.animations {
        visual.SetOpacity2(1.0)?;
        o.panel.visual.SetTransform2(&Matrix3x2::identity())?;
        return dcomp.Commit();
      }
      const MS: f32 = 260.0;
      visual.SetOpacity(&anim::build(dcomp, 0.0, 1.0, MS, POP_IN)?)?;
      // grows from the top of the box (its shadow margin above)
      let grow = dcomp.CreateScaleTransform()?;
      grow.SetCenterX2(o.surface_w * o.scale / 2.0)?;
      grow.SetCenterY2(SHADOW * o.scale)?;
      let size = anim::build(dcomp, 0.98, 1.0, MS, POP_IN)?;
      grow.SetScaleX(&size)?;
      grow.SetScaleY(&size)?;
      let lower = dcomp.CreateTranslateTransform()?;
      lower.SetOffsetY(&anim::build(dcomp, -14.0 * o.scale, 0.0, MS, POP_IN)?)?;
      let moves = dcomp.CreateTransformGroup(&[Some(grow.cast()?), Some(lower.cast()?)])?;
      o.panel.visual.SetTransform(&moves)?;
      dcomp.Commit()
    }
  }

  fn overview_prepare(&mut self, mode: &str) -> bool {
    if !self.overview_prepare_window() { return false; }
    // ";" clipboard (Super+V), "#" file search (Super+S): the box opens with that prefix
    let text = if mode == ";" || mode == "#" { mode } else { "" };
    let Some(o) = self.overview.as_mut() else { return false };
    o.reset(text);
    let apps = self.icons.apps().to_vec();
    let clip_mode = o.refresh(&apps, self.model.hour12);
    self.overview_want_clips(clip_mode);
    self.overview_request_files();
    // drawn before it is shown: no empty frame
    self.overview_render();
    let Some(o) = self.overview.as_mut() else { return false };
    o.shown = true;
    true
  }

  pub(super) fn overview_hide(&mut self) {
    FILE_SEARCH_GENERATION.fetch_add(1, Ordering::AcqRel);
    if let Some(o) = self.overview.as_mut() {
      if o.shown {
        if o.pressed_window.take().is_some() {
          unsafe { let _ = ReleaseCapture(); }
        }
        o.dragging = false;
        o.drag_workspace = None;
        o.shown = false;
        unsafe {
          let _ = ShowWindow(o.hwnd, SW_HIDE);
        }
      }
    }
  }

  pub(super) fn overview_render(&mut self) {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, overview, model, .. } = self;
    let Some(o) = overview.as_mut() else { return };
    o.animations = model.animations;
    let tr = |s: &str| model.tr(s);
    let mut requests = Vec::new();
    let surface = o.panel.surface.clone();
    let scale = o.scale;
    let drawn = gfx::draw_surface(&surface, scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = o.paint(gfx, &mut p, &theme, &model.wm, &tr) {
        tracing::warn!("Super menu: paint: {:?}", err);
      }
      Ok(())
    });
    if let Err(err) = drawn {
      tracing::warn!("Super menu: draw: {:?}", err);
    }
    unsafe {
      let _ = gfx.dcomp.Commit();
    }
    for h in requests {
      std::thread::spawn(move || {
        let png = match core_api::post(&format!("/winicon?h={h}")) {
          Some((200, body)) => data_url_bytes(&String::from_utf8_lossy(&body)),
          _ => None,
        };
        if png.is_none() { std::thread::sleep(Duration::from_secs(30)); }
        super::send(Msg::WinIcon(h, png));
      });
    }
  }

  /// Test: the menu with `text` drawn offscreen into a PNG (no window shown,
  /// no focus taken). `LL_NATIVE_OVERVIEW_SHOT=<png>` + `LL_NATIVE_OVERVIEW_TEXT`.
  pub(super) fn overview_snapshot(&mut self, text: &str, path: &std::path::Path) -> anyhow::Result<()> {
    if self.overview.is_none() {
      self.overview = Some(Overview::new(&self.gfx, true, &self.model.wm)?);
    }
    let apps = self.icons.apps().to_vec();
    let hour12 = self.model.hour12;
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, overview, model, .. } = self;
    let Some(o) = overview.as_mut() else { return Ok(()) };
    o.reset(text);
    if o.refresh(&apps, hour12) {
      let clips: Vec<Clip> = core_json(&["--clip-list"]).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
      o.set_clips(clips, &apps, hour12);
    }
    let tr = |s: &str| model.tr(s);
    let mut requests = Vec::new();
    let scale = o.scale;
    gfx.snapshot((o.surface_w * scale).ceil() as u32, (o.surface_h * scale).ceil() as u32, scale, path, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = o.paint(gfx, &mut p, &theme, &model.wm, &tr) {
        tracing::warn!("Super menu: paint: {:?}", err);
      }
      Ok(())
    })
  }

  /// Messages of the Super menu's window.
  pub(super) fn overview_msg(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if msg == WM_SHOWWINDOW {
      if wp.0 != 0 && !self.overview.as_ref().is_some_and(|o| o.shown) {
        let flag = super::state_dir().join("overview-mode.txt");
        let mode = std::fs::read_to_string(&flag).unwrap_or_default();
        let _ = std::fs::remove_file(flag);
        self.overview_prepare(mode.trim());
      } else if wp.0 == 0 {
        if let Some(o) = self.overview.as_mut() {
          if o.pressed_window.take().is_some() || std::mem::take(&mut o.selecting) {
            unsafe { let _ = ReleaseCapture(); }
          }
          if !o.comp.is_empty() {
            ime::cancel(o.hwnd);
            o.comp.clear();
          }
          o.dragging = false;
          o.drag_workspace = None;
          o.shown = false;
        }
      }
      return Some(LRESULT(0));
    }
    let o = self.overview.as_mut()?;
    let dip = |lp: LPARAM, scale: f32| {
      let x = (lp.0 & 0xFFFF) as i16 as f32 / scale;
      let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / scale;
      (x, y)
    };
    let d = match msg {
      WM_KEYDOWN | WM_SYSKEYDOWN => o.key(wp.0 as u16),
      WM_CHAR => o.char(wp.0 as u16),
      WM_MOUSEMOVE => {
        let (x, y) = dip(lp, o.scale);
        let mut p = POINT::default();
        unsafe {
          let _ = GetCursorPos(&mut p);
        }
        o.mouse_move(p, x, y)
      }
      WM_LBUTTONDOWN => {
        let (x, y) = dip(lp, o.scale);
        o.click(x, y)
      }
      WM_LBUTTONUP => o.release(),
      WM_LBUTTONDBLCLK => {
        let (x, y) = dip(lp, o.scale);
        o.double_click(x, y)
      }
      WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
        let mut p = POINT::default();
        unsafe {
          let _ = GetCursorPos(&mut p);
          let _ = ScreenToClient(o.hwnd, &mut p);
          if let Ok(cursor) = LoadCursorW(None, o.cursor_at(p.x as f32 / o.scale, p.y as f32 / o.scale)) {
            SetCursor(cursor);
          }
        }
        return Some(LRESULT(1));
      }
      WM_TIMER if wp.0 == TIMER_MORPH => {
        // the frame after the last one draws the end values
        if !o.animating() {
          unsafe {
            let _ = KillTimer(o.hwnd, TIMER_MORPH);
          }
        }
        Do::Redraw
      }
      // The IME: its composition is drawn in the field (not in the IME's
      // own window), the candidate list opens at the caret.
      ime::WM_IME_SETCONTEXT => {
        let lp = if wp.0 != 0 { LPARAM(lp.0 & !ime::ISC_SHOWUICOMPOSITIONWINDOW) } else { lp };
        return Some(unsafe { DefWindowProcW(o.hwnd, msg, wp, lp) });
      }
      ime::WM_IME_STARTCOMPOSITION => {
        // the composition replaces the selection, as typing does
        if o.edit.selection().0 != o.edit.selection().1 {
          o.edit.backspace(false);
        }
        o.comp.clear();
        o.comp_cursor = 0;
        self.overview_ime_render();
        return Some(LRESULT(0));
      }
      ime::WM_IME_COMPOSITION => {
        let flags = lp.0 as u32;
        let hwnd = o.hwnd;
        let mut d = Do::Nothing;
        if flags & ime::GCS_RESULTSTR != 0 {
          let text = ime::string(hwnd, ime::GCS_RESULTSTR);
          o.comp.clear();
          o.comp_cursor = 0;
          o.edit.insert(&text);
          d = Do::Search;
        }
        if flags & ime::GCS_COMPSTR != 0 {
          o.comp = ime::string(hwnd, ime::GCS_COMPSTR);
          o.comp_cursor = ime::char_index(&o.comp, ime::cursor(hwnd));
        }
        // handled here: DefWindowProc would type the result again as WM_CHAR
        if matches!(d, Do::Search) {
          self.overview_do(d);
          self.overview_place_ime();
        } else {
          self.overview_ime_render();
        }
        return Some(LRESULT(0));
      }
      ime::WM_IME_ENDCOMPOSITION => {
        o.comp.clear();
        o.comp_cursor = 0;
        Do::Redraw
      }
      WM_CAPTURECHANGED => {
        o.selecting = false;
        o.pressed_window = None;
        o.dragging = false;
        o.drag_workspace = None;
        Do::Redraw
      }
      WM_RBUTTONUP => {
        let (x, y) = dip(lp, o.scale);
        o.right_click(x, y)
      }
      WM_MOUSEWHEEL => o.wheel(((wp.0 >> 16) & 0xFFFF) as i16 as i32),
      // closing the menu hides it: the window lives as long as the bar
      // (another program's WM_CLOSE, e.g. taskkill without /f, would
      // otherwise destroy it)
      WM_CLOSE => {
        if o.shown {
          Do::Hide
        } else {
          Do::Nothing
        }
      }
      WM_ACTIVATE => {
        // focus went elsewhere: close (the web menu's blur); not to an app's
        // context menu, which gives it back or runs a command
        if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE && o.shown && !o.menu_open {
          Do::Hide
        } else {
          Do::Nothing
        }
      }
      WM_PAINT => {
        unsafe {
          let _ = windows::Win32::Graphics::Gdi::ValidateRect(o.hwnd, None);
        }
        return Some(LRESULT(0));
      }
      _ => return None,
    };
    self.overview_do(d);
    Some(LRESULT(0))
  }

  /// Repaints for the IME and moves its candidate list to the caret.
  fn overview_ime_render(&mut self) {
    self.overview_render();
    self.overview_place_ime();
  }

  fn overview_place_ime(&self) {
    if let Some(o) = self.overview.as_ref() {
      ime::place(o.hwnd, o.caret_px, o.line_px);
    }
  }

  fn overview_do(&mut self, d: Do) {
    match d {
      Do::Nothing => {}
      Do::Redraw => self.overview_render(),
      Do::Search => {
        let apps = self.icons.apps().to_vec();
        let hour12 = self.model.hour12;
        let clip_mode = self.overview.as_mut().is_some_and(|o| o.refresh(&apps, hour12));
        self.overview_want_clips(clip_mode);
        self.overview_request_files();
        self.overview_render();
      }
      Do::Hide => self.overview_hide(),
      Do::Workspace(workspace) => {
        self.overview_hide();
        self.slide(workspace);
      }
      Do::FocusWindow { workspace, id } => {
        self.overview_hide();
        let _ = self.wm_cmd.send(wm::Command::FocusWindow { workspace, id });
      }
      Do::MoveWindow { workspace, id } => {
        let _ = self.wm_cmd.send(wm::Command::MoveWindow { workspace, id });
        self.overview_render();
      }
      Do::Run(item) => self.overview_run(item),
      Do::DeleteClip(id) => {
        std::thread::spawn(move || {
          let _ = core_json(&["--clip-del", &id]);
          if let Some(list) = core_json(&["--clip-list"]) {
            let clips: Vec<Clip> = serde_json::from_value(list).unwrap_or_default();
            super::send(Msg::Clips(clips));
          }
        });
      }
      Do::Tool(Tool::Lens) => {
        self.overview_hide();
        std::thread::spawn(|| {
          std::thread::sleep(Duration::from_millis(280));
          core_api::run_core(&["--lens"]);
        });
      }
      Do::Tool(Tool::SongRec) => self.songrec(),
      Do::Copy(text) => {
        if let Some(o) = &self.overview {
          set_clipboard(o.hwnd, &text);
        }
        self.overview_do(Do::Search);
      }
      Do::Paste => {
        let text = self.overview.as_ref().and_then(|o| clipboard_text(o.hwnd));
        if let (Some(t), Some(o)) = (text, self.overview.as_mut()) {
          o.edit.insert(&t);
          self.overview_do(Do::Search);
        }
      }
      Do::Menu(path) => self.overview_menu(path),
    }
  }

  fn overview_request_files(&self) {
    let token = FILE_SEARCH_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let Some(query) = self.overview.as_ref().map(|o| o.edit.text()) else { return };
    let Some(term) = search::file_term(&query).filter(|term| !term.is_empty()).map(str::to_owned) else { return };
    std::thread::spawn(move || {
      std::thread::sleep(Duration::from_millis(110));
      if FILE_SEARCH_GENERATION.load(Ordering::Acquire) != token { return; }
      let result = crate::everything::query(&term, 8);
      if FILE_SEARCH_GENERATION.load(Ordering::Acquire) == token {
        super::send(Msg::Files(query, result));
      }
    });
  }

  pub(super) fn overview_files(&mut self, query: String, result: Result<Vec<crate::everything::FileHit>, String>) {
    if self.overview.as_mut().is_some_and(|o| o.set_files(&query, result)) {
      self.overview_render();
    }
  }

  /// An app's Windows context menu (`lunge.exe --shell-menu`, run from this
  /// unelevated process so what it opens is unelevated too). The helper
  /// answers `{"invoked":true|false}` as soon as the menu closes; it may live
  /// on for a window it opened (Properties). A command closes the Super menu,
  /// a cancel leaves it open (the helper gives the focus back). An app with
  /// an exe name gets "Keep in Dock" on top (the Dock matches its running
  /// windows by that name).
  fn overview_menu(&mut self, path: String) {
    let dock = self
      .icons
      .apps()
      .iter()
      .find(|a| a.path.eq_ignore_ascii_case(&path))
      .and_then(|a| a.exe.clone());
    let Some(o) = self.overview.as_mut() else { return };
    if o.menu_open {
      return;
    }
    o.menu_open = true;
    std::thread::spawn(move || {
      use std::io::BufRead;
      let mut invoked = None;
      if let Some(exe) = core_api::core_exe() {
        let mut args = vec!["--shell-menu".to_string(), path];
        if let Some(dock) = dock {
          args.extend(["--dock".to_string(), dock]);
        }
        let child = std::process::Command::new(exe)
          .args(&args)
          .stdout(std::process::Stdio::piped())
          .creation_flags(CREATE_NO_WINDOW)
          .spawn();
        match child {
          Ok(mut child) => {
            if let Some(out) = child.stdout.take() {
              for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if line.contains("\"invoked\"") {
                  let yes = line.contains("true");
                  invoked = Some(yes);
                  super::send(Msg::ShellMenu(yes));
                  break;
                }
              }
            }
            let _ = child.wait();
          }
          Err(err) => tracing::warn!("Super menu: context menu: {:?}", err),
        }
      }
      if invoked.is_none() {
        super::send(Msg::ShellMenu(false));
      }
    });
  }

  pub(super) fn overview_menu_done(&mut self, invoked: bool) {
    let Some(o) = self.overview.as_mut() else { return };
    o.menu_open = false;
    if invoked {
      self.overview_hide();
    }
  }

  /// The clipboard history, once per opening of the menu (deleting an entry
  /// reloads it on its own).
  fn overview_want_clips(&mut self, clip_mode: bool) {
    let Some(o) = self.overview.as_mut() else { return };
    if clip_mode && !o.clips_asked {
      o.clips_asked = true;
      self.overview_load_clips();
    }
  }

  fn overview_load_clips(&mut self) {
    std::thread::spawn(|| {
      let clips: Vec<Clip> = core_json(&["--clip-list"]).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
      super::send(Msg::Clips(clips));
    });
  }

  pub(super) fn overview_clips(&mut self, clips: Vec<Clip>) {
    let apps = self.icons.apps().to_vec();
    let hour12 = self.model.hour12;
    let Some(o) = self.overview.as_mut() else { return };
    o.set_clips(clips, &apps, hour12);
    if o.shown {
      self.overview_render();
    }
  }

  /// Runs a result (the web menu's overview.html `exec`): most close the menu first.
  fn overview_run(&mut self, item: Item) {
    if !item.stay {
      self.overview_hide();
    }
    let emit = self.emit.clone();
    let toast = move |v: Value| (emit)("ll:toast", v);
    match item.act {
      Act::None => {}
      Act::Query(s) => {
        if let Some(o) = self.overview.as_mut() {
          o.edit.set(&s);
        }
        self.overview_do(Do::Search);
      }
      Act::Launch(path) => {
        // ii / Hyprland dwindle: the new window splits the one under the pointer
        std::thread::spawn(move || {
          std::thread::sleep(Duration::from_millis(60));
          if let Some(exe) = core_api::core_exe() {
            let _ = std::process::Command::new(exe).arg("--focus-under-cursor").creation_flags(CREATE_NO_WINDOW).status();
          }
          spawn("explorer", &[&path]);
        });
      }
      Act::OpenPath(path) => {
        std::thread::spawn(move || {
          if let Some(exe) = core_api::core_exe() {
            let _ = std::process::Command::new(exe).args(["--launch", &path]).creation_flags(CREATE_NO_WINDOW).status();
          }
        });
      }
      Act::Script(mode, text) => {
        std::thread::spawn(move || {
          match core_json(&["--run", mode, &text]) {
            Some(mut res) => {
              let mono = res["kind"] == "ok" && res["icon"] == "terminal";
              res["mono"] = json!(mono);
              toast(res);
            }
            None => toast(json!({ "kind": "error", "title": "Çalıştırılamadı", "body": text, "icon": "error" })),
          }
        });
      }
      Act::Copy(text) => {
        if let Some(o) = &self.overview {
          set_clipboard(o.hwnd, &text);
        }
      }
      Act::ClipSet(id) => {
        let name = item.name.clone();
        std::thread::spawn(move || match core_json(&["--clip-set", &id]) {
          Some(res) if res["ok"].as_bool() == Some(true) => {
            toast(json!({ "kind": "ok", "title": "Panoya kopyalandı", "body": name.chars().take(80).collect::<String>(), "icon": "content_paste", "timeout": 1500 }))
          }
          res => {
            let why = res.as_ref().and_then(|r| r["error"].as_str()).unwrap_or("Pano meşgul").to_string();
            toast(json!({ "kind": "error", "title": "Kopyalanamadı", "body": why, "icon": "error" }))
          }
        });
      }
      Act::Action(name) => self.overview_action(name),
    }
  }

  /// ii /actions (search::ACTIONS)
  fn overview_action(&mut self, name: &str) {
    match name {
      "dark" => {
        let light = !self.model.light;
        self.set_light(light);
        core_api::set_pref("theme", if light { "light" } else { "dark" });
      }
      "lock" => spawn("rundll32", &["user32.dll,LockWorkStation"]),
      "sleep" => spawn("rundll32", &["powrprof.dll,SetSuspendState", "0,1,0"]),
      "logout" => spawn("shutdown", &["/l"]),
      "restart" => spawn("shutdown", &["/r", "/t", "0"]),
      "shutdown" => spawn("shutdown", &["/s", "/t", "0"]),
      "reload" => self.wm_command("command wm-reload-config".into()),
      "apps" => {
        let emit = self.emit.clone();
        std::thread::spawn(move || {
          let toast = |v: Value| (emit)("ll:toast", v);
          let _ = core_json(&["--build-apps"]);
          let apps = match core_api::post("/apps.json") {
            Some((200, body)) => serde_json::from_slice::<Vec<App>>(&body).ok(),
            _ => None,
          };
          // as the web menu: how many apps, or why there are none
          match apps {
            Some(apps) if !apps.is_empty() => {
              toast(json!({ "kind": "ok", "title": "Uygulama listesi yenilendi", "body": format!("{} uygulama", apps.len()), "icon": "apps" }));
              super::send(Msg::Apps(apps));
            }
            _ => toast(json!({ "kind": "error", "title": "Uygulamalar alınamadı", "body": "Uygulama listesi boş.", "icon": "error" })),
          }
        });
      }
      _ => {}
    }
  }

  /// ii SongRec: listen to what plays (`lunge.exe --songrec`), a toast with
  /// the result; pressing again stops it.
  fn songrec(&mut self) {
    let Some(o) = self.overview.as_mut() else { return };
    if o.songrec {
      // stop: no result from this run, and its helper ends now
      o.songrec = false;
      super::SONGREC_RUN.fetch_add(1, Ordering::AcqRel);
      if let Some(mut child) = take_songrec(None) {
        let _ = child.kill();
      }
      self.overview_render();
      return;
    }
    o.songrec = true;
    let run = super::SONGREC_RUN.fetch_add(1, Ordering::AcqRel) + 1;
    self.overview_render();
    let emit = self.emit.clone();
    std::thread::spawn(move || {
      let res = songrec_listen(run);
      // a stopped run, or one a new press replaced, stays quiet
      if super::SONGREC_RUN.load(Ordering::Acquire) != run {
        return;
      }
      super::send(Msg::SongRecDone);
      let toast = |v: Value| (emit)("ll:toast", v);
      match res {
        Some(r) if r["title"].is_string() => {
          let (title, sub) = (r["title"].as_str().unwrap_or(""), r["subtitle"].as_str().unwrap_or(""));
          let q = search::encode_uri_component(&format!("{} - {}", title, sub));
          toast(json!({
            "kind": "ok", "title": "Müzik tanındı", "body": format!("{} - {}", title, sub), "icon": "music_note",
            "image": r["cover"], "timeout": 12000,
            "actions": [
              { "label": "Spotify", "url": format!("spotify:search:{}", q) },
              { "label": "YouTube", "url": format!("https://www.youtube.com/results?search_query={}", q) }
            ]
          }))
        }
        Some(r) if r["error"] == "audio" => toast(json!({ "kind": "error", "title": "Müzik tanınamadı", "body": "Ses çıkışı dinlenemedi", "icon": "music_off" })),
        _ => toast(json!({ "kind": "error", "title": "Müzik tanınamadı", "body": "Dinlediğin şey fazla niş olabilir", "icon": "music_off" })),
      }
    });
  }

  pub(super) fn songrec_done(&mut self) {
    if let Some(o) = self.overview.as_mut() {
      o.songrec = false;
      if o.shown {
        self.overview_render();
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn edit(s: &str) -> Edit {
    let mut e = Edit::default();
    e.set(s);
    e
  }

  #[test]
  fn typing_and_selection() {
    let mut e = edit("hello world");
    e.left(true, false);
    assert_eq!(e.caret, 6);
    e.left(false, true);
    e.left(false, true);
    assert_eq!(e.selected(), "o ");
    e.insert("X");
    assert_eq!(e.text(), "hellXworld");
    e.select_all();
    e.insert("a\nb");
    assert_eq!(e.text(), "a b");
  }

  #[test]
  fn word_deletion() {
    let mut e = edit("open visual studio");
    e.backspace(true);
    assert_eq!(e.text(), "open visual ");
    e.backspace(true);
    assert_eq!(e.text(), "open ");
    e.home(false);
    e.delete(true);
    assert_eq!(e.text(), " ");
  }

  #[test]
  fn arrows_collapse_a_selection_to_its_side() {
    let mut e = edit("abcdef");
    e.left(false, true);
    e.left(false, true);
    assert_eq!(e.selection(), (4, 6));
    e.left(false, false);
    assert_eq!((e.caret, e.anchor), (4, 4));
    e.select_all();
    e.right(false, false);
    assert_eq!((e.caret, e.anchor), (6, 6));
  }

  fn typed(s: &str) -> Edit {
    let mut e = Edit::default();
    for c in s.chars() {
      e.insert(&c.to_string());
    }
    e
  }

  #[test]
  fn undo_takes_back_a_word_at_a_time() {
    let mut e = typed("open visual");
    assert!(e.undo());
    assert_eq!(e.text(), "open ");
    assert!(e.undo());
    assert_eq!(e.text(), "open");
    assert!(e.undo());
    assert_eq!(e.text(), "");
    assert!(!e.undo());
    assert!(e.redo());
    assert!(e.redo());
    assert_eq!(e.text(), "open ");
  }

  #[test]
  fn a_new_change_drops_the_redo_steps() {
    let mut e = typed("abc");
    e.backspace(false);
    e.backspace(false);
    assert_eq!(e.text(), "a");
    assert!(e.undo());
    assert_eq!(e.text(), "abc");
    e.insert("d");
    assert!(!e.redo());
    assert_eq!(e.text(), "abcd");
  }

  #[test]
  fn moving_the_caret_starts_a_new_step() {
    let mut e = typed("ab");
    e.left(false, false);
    e.insert("x");
    assert_eq!(e.text(), "axb");
    assert!(e.undo());
    assert_eq!(e.text(), "ab");
    assert_eq!(e.caret, 1);
  }

  #[test]
  fn opening_the_menu_forgets_the_history() {
    let mut e = typed("abc");
    e.start(";");
    assert!(!e.undo());
    assert_eq!((e.text(), e.caret), (";".to_string(), 1));
  }

  #[test]
  fn double_click_selects_the_run_under_it() {
    let mut e = edit("open visual  studio");
    e.select_word(7);
    assert_eq!(e.selected(), "visual");
    e.select_word(11);
    assert_eq!(e.selected(), "visual");
    e.select_word(12);
    assert_eq!(e.selected(), "  ");
    e.select_word(19);
    assert_eq!(e.selected(), "studio");
    let mut empty = Edit::default();
    empty.select_word(0);
    assert_eq!(empty.selection(), (0, 0));
  }

  #[test]
  fn shift_click_extends_the_selection() {
    let mut e = edit("hello world");
    e.place(2, false);
    e.place(8, true);
    assert_eq!(e.selected(), "llo wo");
    e.place(20, false);
    assert_eq!((e.caret, e.anchor), (11, 11));
  }

  #[test]
  fn paragraph_direction_follows_the_first_letter() {
    assert!(is_rtl("مرحبا world"));
    assert!(is_rtl("123 שלום"));
    assert!(!is_rtl("hello مرحبا"));
    assert!(!is_rtl("42"));
  }

  #[test]
  fn shapes_have_full_outlines() {
    let (pill, _) = outline(Prefix::Action);
    assert!((pill[0].0 - 20.0).abs() < 0.01);
    assert!((pill[OUTLINE / 4].1 - 10.0).abs() < 0.01);
    let (cookie, turn) = outline(Prefix::Math);
    assert!(cookie.iter().all(|(x, y)| (x * x + y * y).sqrt() <= 20.01));
    assert!(turn > 0.0);
  }

  #[test]
  fn turkish_and_emoji_characters() {
    let mut e = edit("görev");
    e.backspace(false);
    assert_eq!(e.text(), "göre");
    e.insert("ş😀");
    assert_eq!(e.text(), "göreş😀");
    e.backspace(false);
    assert_eq!(e.text(), "göreş");
  }
}
