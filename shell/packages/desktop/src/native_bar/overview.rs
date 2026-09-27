//! The Super menu (ii overview: the search box on top, the workspace grid
//! below), drawn like the bar on its UI thread. Sizes and colours from
//! ui/overview.css, results from `search.rs`, the responsibility map in
//! docs/native-overview.md.

use std::{collections::HashMap, os::windows::process::CommandExt, path::PathBuf, time::Duration};

use serde_json::{json, Value};
use windows::{
  core::HSTRING,
  Win32::{
    Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      Direct2D::{
        Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED},
        ID2D1Bitmap1, ID2D1Factory, D2D1_ANTIALIAS_MODE_ALIASED, D2D1_DRAW_TEXT_OPTIONS_NONE,
      },
      DirectComposition::{IDCompositionTarget, IDCompositionVisual2},
      DirectWrite::{DWRITE_HIT_TEST_METRICS, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE},
      Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY},
    },
    System::{
      DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData},
      LibraryLoader::GetModuleHandleW,
      Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    },
    UI::{
      HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
      Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT},
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  core_api,
  fonts::TextStyle,
  gfx::{self, pt, Gfx, Rect, Rgba},
  icons::{data_url_bytes, App},
  model,
  popup,
  search::{self, Act, Clip, Glyph, Item, Prefix},
  view::{Align, Painter, Theme},
  Layer, Msg, Ui, CLASS,
};

/// ii searchWidthCollapsed / searchWidth with the margins and the two buttons
const W_COLLAPSED: f32 = 356.0;
const W_EXPANDED: f32 = 560.0;
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

const CF_UNICODETEXT: u32 = 13;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The core finds the Super menu by this title (Super, Super+V, focus guard).
pub const TITLE: &str = "lunge-overview";
/// test run next to the running menu (the core does not know it)
pub const TITLE_DEMO: &str = "lunge-overview (native demo)";

// ------------------------------------------------------------ text editing

/// A one-line text field: characters, the caret, and the other end of the
/// selection (`anchor == caret`: no selection).
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Edit {
  pub chars: Vec<char>,
  pub caret: usize,
  pub anchor: usize,
}

fn is_word(c: char) -> bool {
  c.is_alphanumeric() || c == '_'
}

impl Edit {
  pub fn text(&self) -> String {
    self.chars.iter().collect()
  }

  pub fn set(&mut self, s: &str) {
    self.chars = s.chars().collect();
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

  /// Types or pastes `s` over the selection. Line breaks become spaces.
  pub fn insert(&mut self, s: &str) {
    let (a, b) = self.selection();
    let add: Vec<char> = s.chars().map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect();
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
      self.remove(a, b);
    } else if a > 0 {
      let from = if word { self.word_left(a) } else { a - 1 };
      self.remove(from, a);
    }
  }

  pub fn delete(&mut self, word: bool) {
    let (a, b) = self.selection();
    if a != b {
      self.remove(a, b);
    } else if a < self.chars.len() {
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
  }

  pub fn right(&mut self, word: bool, extend: bool) {
    let (a, b) = self.selection();
    let n = self.chars.len();
    self.caret = if !extend && a != b && !word { b } else if word { self.word_right(self.caret) } else { (self.caret + 1).min(n) };
    if !extend {
      self.anchor = self.caret;
    }
  }

  pub fn home(&mut self, extend: bool) {
    self.caret = 0;
    if !extend {
      self.anchor = 0;
    }
  }

  pub fn end(&mut self, extend: bool) {
    self.caret = self.chars.len();
    if !extend {
      self.anchor = self.caret;
    }
  }

  pub fn select_all(&mut self) {
    self.anchor = 0;
    self.caret = self.chars.len();
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
  pub demo: bool,
  /// monitor rectangle (screen pixels); the window covers it
  monitor: RECT,
  _target: IDCompositionTarget,
  root: IDCompositionVisual2,
  panel: Layer,
  pub shown: bool,
  /// the Windows context menu of an app is open (its helper has the focus)
  pub menu_open: bool,
  pub edit: Edit,
  results: Vec<Item>,
  sel: usize,
  /// first row shown (the list scrolls past 11 rows)
  first: usize,
  clips: Vec<Clip>,
  clip_images: HashMap<String, Option<ID2D1Bitmap1>>,
  pub songrec: bool,
  /// horizontal scroll of the text field (DIPs)
  scroll_x: f32,
  /// a high surrogate waiting for its pair (WM_CHAR)
  surrogate: Option<u16>,
  /// hover moves the selection only when the pointer really moved
  last_mouse: POINT,
  hover_tool: Option<Tool>,
  // layout of the last paint, in DIPs from the window's top-left
  box_rect: Rect,
  tools: [(Rect, Tool); 2],
  rows: Vec<(Rect, usize)>,
}

impl Drop for Overview {
  fn drop(&mut self) {
    unsafe {
      let _ = DestroyWindow(self.hwnd);
    }
  }
}

impl Overview {
  pub fn new(gfx: &Gfx, demo: bool) -> anyhow::Result<Self> {
    let (monitor, scale) = primary_monitor();
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
        let panel = Layer::new(gfx, (SURF_W * scale).ceil() as u32, (SURF_H * scale).ceil() as u32)?;
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
        demo,
        monitor,
        _target: target,
        root,
        panel,
        shown: false,
        menu_open: false,
        edit: Edit::default(),
        results: Vec::new(),
        sel: 0,
        first: 0,
        clips: Vec::new(),
        clip_images: HashMap::new(),
        songrec: false,
        scroll_x: 0.0,
        surrogate: None,
        last_mouse: POINT::default(),
        hover_tool: None,
        box_rect: Rect::new(0.0, 0.0, 0.0, 0.0),
        tools: [(Rect::new(0.0, 0.0, 0.0, 0.0), Tool::Lens), (Rect::new(0.0, 0.0, 0.0, 0.0), Tool::SongRec)],
        rows: Vec::new(),
      })
    }
  }

  fn width_dip(&self) -> f32 {
    (self.monitor.right - self.monitor.left) as f32 / self.scale
  }

  fn box_width(&self) -> f32 {
    if self.edit.chars.is_empty() {
      W_COLLAPSED
    } else {
      W_EXPANDED
    }
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
    self.results = search::results(&text, apps, &self.clips, &|t| model::clock_at(t, hour12));
    self.sel = 0;
    self.first = 0;
    clip_mode
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
  pub fn paint(&mut self, gfx: &Gfx, p: &mut Painter, t: &Theme, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
    let w = self.box_width();
    let h = BAR + self.list_height();
    // the box is centred on the monitor; the surface is placed around it
    let left = (self.width_dip() - SURF_W) / 2.0;
    let top = TOP - SHADOW;
    unsafe {
      self.panel.visual.SetOffsetX2((left * self.scale).round())?;
      self.panel.visual.SetOffsetY2((top * self.scale).round())?;
    }
    let bx = Rect::new(SHADOW + (W_EXPANDED - w) / 2.0, SHADOW, w, h);
    self.box_rect = Rect::new(left + bx.x, top + bx.y, w, h);

    popup::frame_shadow(p, bx, RADIUS)?;
    p.fill_round(bx, RADIUS, t.surface_container)?;

    // search bar: shape, field, Lens, song recognition
    let prefix = Prefix::of(&self.edit.text());
    let shape_c = (bx.x + 10.0 + 20.0, bx.y + BAR / 2.0);
    shape(p, prefix, shape_c.0, shape_c.1, t.primary_container)?;
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
    self.paint_field(p, t, field, tr)?;

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
    Ok(())
  }

  fn paint_field(&mut self, p: &mut Painter, t: &Theme, r: Rect, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
    let style = TextStyle { size: 15.0, weight: 450.0 };
    if self.edit.chars.is_empty() {
      p.text(&tr("Ara, hesapla veya çalıştır"), r, style, t.on_surface_variant, Align::Left, false)?;
      caret(p, r.x, r, t)?;
      self.scroll_x = 0.0;
      return Ok(());
    }
    let text = self.edit.text();
    let layout = p.layout(&text, style, 100_000.0, r.h, false)?;
    // caret and selection ends, in UTF-16 positions
    let utf16_at = |i: usize| self.edit.chars[..i].iter().map(|c| c.len_utf16()).sum::<usize>() as u32;
    let x_at = |i: usize| -> f32 {
      let (mut x, mut y) = (0f32, 0f32);
      let mut m = DWRITE_HIT_TEST_METRICS::default();
      unsafe {
        let _ = layout.HitTestTextPosition(utf16_at(i), false, &mut x, &mut y, &mut m);
      }
      x
    };
    let caret_x = x_at(self.edit.caret);
    // keep the caret in view
    if caret_x - self.scroll_x > r.w - 2.0 {
      self.scroll_x = caret_x - r.w + 2.0;
    } else if caret_x < self.scroll_x {
      self.scroll_x = caret_x;
    }
    let full_w = Painter::width_of(&layout);
    self.scroll_x = self.scroll_x.clamp(0.0, (full_w - r.w + 2.0).max(0.0));
    unsafe {
      p.dc.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED);
    }
    let (a, b) = self.edit.selection();
    if a != b {
      let (xa, xb) = (x_at(a), x_at(b));
      p.fill(Rect::new(r.x + xa - self.scroll_x, r.y + 9.0, xb - xa, r.h - 18.0), Rgba(t.primary.0, t.primary.1, t.primary.2, 0.35))?;
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
        D2D1_DRAW_TEXT_OPTIONS_NONE,
      );
      p.dc.PopAxisAlignedClip();
    }
    caret(p, r.x + caret_x - self.scroll_x, r, t)?;
    Ok(())
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
    let name = if item.key == "sh" && item.act == Act::None { tr(&item.name) } else { item.name.clone() };
    match &item.highlight {
      Some(q) => highlighted(p, &name, q, name_r, fg, if selected { Rgba::hex(0xffffff) } else { t.primary })?,
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

  pub fn in_box(&self, x: f32, y: f32) -> bool {
    self.box_rect.contains(x, y)
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
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
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
      0x25 => {
        self.edit.left(ctrl, shift);
        Do::Redraw
      }
      0x27 => {
        self.edit.right(ctrl, shift);
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
    let tool = self.hit_tool(x, y);
    let mut redraw = tool != self.hover_tool;
    self.hover_tool = tool;
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
    if let Some(i) = self.hit_row(x, y) {
      self.sel = i;
      return self.results.get(i).cloned().map(Do::Run).unwrap_or(Do::Nothing);
    }
    // the backdrop closes (.backdrop onMouseDown)
    if !self.in_box(x, y) {
      return Do::Hide;
    }
    Do::Nothing
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
    self.edit.set(text);
    self.results.clear();
    self.sel = 0;
    self.first = 0;
    self.scroll_x = 0.0;
    self.surrogate = None;
    self.hover_tool = None;
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
    p.dc.DrawTextLayout(pt(r.x, r.y), &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
  }
  Ok(())
}

/// ii MaterialShape by prefix (Cookie7Sided, Clover4Leaf, PixelCircle ...),
/// 40 x 40 around (cx, cy); the action prefix is a pill.
fn shape(p: &mut Painter, prefix: Prefix, cx: f32, cy: f32, c: Rgba) -> anyhow::Result<()> {
  if prefix == Prefix::Action {
    return Ok(p.fill_round(Rect::new(cx - 20.0, cy - 10.0, 40.0, 20.0), 10.0, c)?);
  }
  let (n, depth) = match prefix {
    Prefix::App => (4.0, 0.22),
    Prefix::Math => (4.0, 0.2),
    Prefix::Shell => (12.0, 0.04),
    Prefix::Web => (10.0, 0.1),
    Prefix::Clip => (6.0, 0.16),
    _ => (7.0, 0.12),
  };
  let rot: f32 = if prefix == Prefix::Math { std::f32::consts::FRAC_PI_4 } else { 0.0 };
  let brush = p.brush(c)?;
  unsafe {
    let factory: ID2D1Factory = p.dc.GetFactory()?;
    let geo = factory.CreatePathGeometry()?;
    let sink = geo.Open()?;
    for step in 0..=120 {
      let a = step as f32 * 3.0 * std::f32::consts::PI / 180.0;
      let r = 20.0 * (1.0 - depth + depth * (n * a).cos());
      let (x, y) = (cx + r * (a + rot).cos(), cy + r * (a + rot).sin());
      if step == 0 {
        sink.BeginFigure(pt(x, y), D2D1_FIGURE_BEGIN_FILLED);
      } else {
        sink.AddLine(pt(x, y));
      }
    }
    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
    sink.Close()?;
    p.dc.FillGeometry(&geo, &brush, None);
  }
  Ok(())
}

// ------------------------------------------------------------ system helpers

/// The primary monitor (the web menu's preset) and its scale.
fn primary_monitor() -> (RECT, f32) {
  unsafe {
    let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
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

/// The install folder (next to lunge-shell.exe): lunge.exe, scripts\.
fn install_dir() -> PathBuf {
  std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default()
}

fn spawn(program: &str, args: &[&str]) {
  let _ = std::process::Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).spawn();
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
  /// Super / the bar's search button: open (or close if open and in front).
  pub(super) fn overview_toggle(&mut self, mode: &str) {
    let open = self.overview.as_ref().is_some_and(|o| o.shown);
    if open {
      self.overview_hide();
    } else {
      self.overview_open(mode);
    }
  }

  pub(super) fn overview_open(&mut self, mode: &str) {
    if self.overview.is_none() {
      match Overview::new(&self.gfx, self.demo) {
        Ok(o) => {
          super::OVERVIEW_HWND.store(o.hwnd.0 as isize, std::sync::atomic::Ordering::Release);
          self.overview = Some(o);
        }
        Err(err) => {
          tracing::error!("Super menu: {:?}", err);
          return;
        }
      }
    }
    let text = if mode == ";" { ";" } else { "" };
    let Some(o) = self.overview.as_mut() else { return };
    o.reset(text);
    let apps = self.icons.apps().to_vec();
    let clip_mode = o.refresh(&apps, self.model.hour12);
    if clip_mode {
      self.overview_load_clips();
    }
    // drawn before it is shown: no empty frame
    self.overview_render();
    let Some(o) = self.overview.as_mut() else { return };
    o.shown = true;
    unsafe {
      let _ = ShowWindow(o.hwnd, SW_SHOW);
      let _ = SetForegroundWindow(o.hwnd);
    }
  }

  pub(super) fn overview_hide(&mut self) {
    if let Some(o) = self.overview.as_mut() {
      if o.shown {
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
    let tr = |s: &str| model.tr(s);
    let mut requests = Vec::new();
    let surface = o.panel.surface.clone();
    let scale = o.scale;
    let drawn = gfx::draw_surface(&surface, scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = o.paint(gfx, &mut p, &theme, &tr) {
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
  }

  /// Test: the menu with `text` drawn offscreen into a PNG (no window shown,
  /// no focus taken). `LL_NATIVE_OVERVIEW_SHOT=<png>` + `LL_NATIVE_OVERVIEW_TEXT`.
  pub(super) fn overview_snapshot(&mut self, text: &str, path: &std::path::Path) -> anyhow::Result<()> {
    if self.overview.is_none() {
      self.overview = Some(Overview::new(&self.gfx, true)?);
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
    gfx.snapshot((SURF_W * scale).ceil() as u32, (SURF_H * scale).ceil() as u32, scale, path, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = o.paint(gfx, &mut p, &theme, &tr) {
        tracing::warn!("Super menu: paint: {:?}", err);
      }
      Ok(())
    })
  }

  /// Messages of the Super menu's window.
  pub(super) fn overview_msg(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
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
      WM_RBUTTONUP => {
        let (x, y) = dip(lp, o.scale);
        o.right_click(x, y)
      }
      WM_MOUSEWHEEL => o.wheel(((wp.0 >> 16) & 0xFFFF) as i16 as i32),
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

  fn overview_do(&mut self, d: Do) {
    match d {
      Do::Nothing => {}
      Do::Redraw => self.overview_render(),
      Do::Search => {
        let apps = self.icons.apps().to_vec();
        let hour12 = self.model.hour12;
        let clip_mode = self.overview.as_mut().is_some_and(|o| o.refresh(&apps, hour12));
        if clip_mode {
          self.overview_load_clips();
        }
        self.overview_render();
      }
      Do::Hide => self.overview_hide(),
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

  /// An app's Windows context menu (`lunge.exe --shell-menu`, run from this
  /// unelevated process so what it opens is unelevated too). The helper
  /// answers `{"invoked":true|false}` as soon as the menu closes; it may live
  /// on for a window it opened (Properties). A command closes the Super menu,
  /// a cancel leaves it open (the helper gives the focus back).
  fn overview_menu(&mut self, path: String) {
    let Some(o) = self.overview.as_mut() else { return };
    if o.menu_open {
      return;
    }
    o.menu_open = true;
    std::thread::spawn(move || {
      use std::io::BufRead;
      let mut invoked = None;
      if let Some(exe) = core_api::core_exe() {
        let child = std::process::Command::new(exe)
          .args(["--shell-menu", &path])
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

  /// Runs a result (ui/overview.html `exec`): most close the menu first.
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
      Act::Script(mode, text) => {
        let script = install_dir().join("scripts").join("run.ps1");
        std::thread::spawn(move || {
          let script = script.to_string_lossy().to_string();
          match core_json(&["--ps", &script, mode, &text]) {
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
        let script = install_dir().join("scripts").join("build-apps.ps1");
        std::thread::spawn(move || {
          let script = script.to_string_lossy().to_string();
          let _ = core_json(&["--ps", &script]);
          if let Some((200, body)) = core_api::post("/apps.json") {
            if let Ok(apps) = serde_json::from_slice::<Vec<App>>(&body) {
              super::send(Msg::Apps(apps));
            }
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
      // stop: the helper ends on its own timeout; the result is dropped
      o.songrec = false;
      super::SONGREC_STOPPED.store(true, std::sync::atomic::Ordering::Release);
      self.overview_render();
      return;
    }
    o.songrec = true;
    super::SONGREC_STOPPED.store(false, std::sync::atomic::Ordering::Release);
    self.overview_render();
    let emit = self.emit.clone();
    std::thread::spawn(move || {
      let res = core_json(&["--songrec", "-i", "2", "-t", "30", "-s", "monitor"]);
      super::send(Msg::SongRecDone);
      if super::SONGREC_STOPPED.load(std::sync::atomic::Ordering::Acquire) {
        return;
      }
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
