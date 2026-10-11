//! The shortcut list (ii's cheatsheet, Super+/): every shortcut that works
//! right now, grouped (workspaces, windows, layout, apps and the launcher,
//! screenshots and tools, media, system), with a search field.
//!
//! The list is the shortcut editor's model (`lunge.exe --keybinds-model`):
//! the core's defaults with the user's changes (keybinds.json), the apps the
//! user added and the window manager's shortcuts (config.yaml), so a change
//! shows here at once. It is read on a worker thread each time the list
//! opens; the last one read is shown meanwhile.
//!
//! One window on the monitor under the pointer, made when it opens and
//! destroyed when it closes. Two visuals: the scrim (the session screen's,
//! stretched over the monitor) and the card. Typing filters (label or
//! keys), Up / Down / PageUp / PageDown / Home / End and the wheel scroll,
//! Esc clears the search and then closes; a click outside the card, the
//! focus going elsewhere or Super+/ again (`ll:cheatsheet-toggle`) close it.

use std::{collections::HashMap, sync::atomic::Ordering, time::Instant};

use serde_json::Value;
use windows::{
  core::{w, Interface},
  Foundation::Numerics::Matrix3x2,
  Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
      Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
      DirectComposition::{
        IDCompositionTarget, IDCompositionVisual2, IDCompositionVisual3,
        DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR, DCOMPOSITION_BORDER_MODE_HARD,
      },
      Gdi::ValidateRect,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
      Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU},
      WindowsAndMessaging::*,
    },
  },
};

use super::{
  anim::{self, POP_IN, POP_OUT},
  core_api,
  fonts::TextStyle,
  gfx::{self, Gfx, Rect, Rgba},
  overview::Edit,
  session::{paint_scrim, SCRIM_PX},
  sidebar::keys::{glyph, ll_label, tiling_label},
  view::{Align, Painter, Theme},
  Layer, Msg, Ui, CHEATSHEET_HWND, CLASS, TIMER_CHEATSHEET_CLOSE,
};

/// the card at most (DIP), and its share of the monitor's height
const CARD_MAX_W: f32 = 1180.0;
const CARD_MAX_H: f32 = 980.0;
const CARD_H_SHARE: f32 = 0.86;
/// the least room left around the card on a small monitor
const MARGIN: f32 = 16.0;
const RADIUS: f32 = 28.0;
/// room around the card for its shadow
const SHADOW: f32 = 40.0;
const PAD: f32 = 28.0;
const HEAD_H: f32 = 76.0;
const FOOT_H: f32 = 40.0;
const FIELD_W: f32 = 360.0;
const FIELD_H: f32 = 40.0;
const COL_GAP: f32 = 16.0;
const GROUP_HEAD: f32 = 40.0;
const ROW_H: f32 = 32.0;
const GROUP_PAD: f32 = 10.0;
const GROUP_GAP: f32 = 16.0;
const CAP_H: f32 = 22.0;
/// a scroll step (arrows, one wheel notch)
const STEP: f32 = 56.0;
/// focus that leaves this soon after the list got it is Windows settling
const BLUR_GRACE_MS: u128 = 300;

/// The groups, in the order they are shown.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Cat {
  Workspaces,
  Windows,
  Layout,
  Apps,
  Tools,
  Media,
  System,
}

const CATS: [Cat; 7] = [Cat::Workspaces, Cat::Windows, Cat::Layout, Cat::Apps, Cat::Tools, Cat::Media, Cat::System];

impl Cat {
  fn name(self) -> &'static str {
    match self {
      Cat::Workspaces => "Workspace’ler",
      Cat::Windows => "Pencereler",
      Cat::Layout => "Yerleşim",
      Cat::Apps => "Uygulamalar ve başlatıcı",
      Cat::Tools => "Ekran görüntüsü ve araçlar",
      Cat::Media => "Medya",
      Cat::System => "Sistem",
    }
  }

  fn icon(self) -> &'static str {
    match self {
      Cat::Workspaces => "view_carousel",
      Cat::Windows => "select_window",
      Cat::Layout => "dashboard",
      Cat::Apps => "apps",
      Cat::Tools => "screenshot_region",
      Cat::Media => "music_note",
      Cat::System => "settings",
    }
  }
}

/// A core shortcut's group (`app`: it opens an app).
pub(super) fn core_category(id: &str, app: bool) -> Cat {
  match id {
    _ if app || id.starts_with("app:") => Cat::Apps,
    "screenshot" | "screenshot-alt" | "screenshot-screen" | "ocr" | "color-picker" | "cheatsheet" => Cat::Tools,
    "overview" | "overview-alt" | "search" | "run" | "file-search" | "clipboard" | "terminal" | "terminal-alt" => Cat::Apps,
    "workspaces" => Cat::Workspaces,
    "close" | "focus-urgent-or-last" => Cat::Windows,
    _ if id.starts_with("focus-") || id.starts_with("move-") => Cat::Windows,
    _ if id.starts_with("ws-") => Cat::Workspaces,
    _ if id.contains("media") || id.contains("volume") => Cat::Media,
    _ => Cat::System,
  }
}

/// A window manager shortcut's group, from its commands (config.yaml).
pub(super) fn tiling_category(cmds: &[String]) -> Cat {
  let c = cmds.join(" ; ").to_lowercase();
  let has = |s: &str| c.contains(s);
  if has("sndvol") || has("media") || has("volume") || has("audio") {
    Cat::Media
  } else if has("ms-settings") || ["wm-reload", "wm-redraw", "wm-toggle-pause", "wm-exit"].iter().any(|p| c.starts_with(p)) {
    Cat::System
  } else if has("shell-exec") {
    Cat::Apps
  } else if has("workspace-in-direction") || c.starts_with("move-workspace") {
    Cat::Layout
  } else if has("workspace") {
    Cat::Workspaces
  } else if ["toggle-floating", "toggle-fullscreen", "toggle-tiling-direction", "resize", "split-ratio", "toggle-pin", "binding-mode"]
    .iter()
    .any(|p| has(p))
  {
    Cat::Layout
  } else {
    Cat::Windows
  }
}

/// "Super+Ctrl+Left" -> ["Super", "Ctrl", "←"]: one key cap each.
pub(super) fn keycaps(combo: &str) -> Vec<String> {
  combo.split('+').map(str::trim).filter(|k| !k.is_empty()).map(|k| glyph(k).to_string()).collect()
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
  pub cat: Cat,
  pub label: String,
  /// each combo as its key caps (the first two are drawn)
  pub combos: Vec<Vec<String>>,
  /// what the search looks in (lower case)
  hay: String,
}

impl Entry {
  fn new(cat: Cat, label: String, combos: Vec<String>) -> Option<Self> {
    let combos: Vec<String> = combos.into_iter().filter(|c| !c.trim().is_empty()).collect();
    if combos.is_empty() || label.trim().is_empty() {
      return None;
    }
    let hay = format!("{} {} {}", label, combos.join(" "), combos.iter().flat_map(|c| keycaps(c)).collect::<Vec<_>>().join(" "));
    let hay = fold(&hay);
    Some(Entry { cat, label, combos: combos.iter().map(|c| keycaps(c)).collect(), hay })
  }

  /// Every word of the search is in the label or the keys.
  pub(super) fn matches(&self, query: &str) -> bool {
    fold(query).split_whitespace().all(|w| self.hay.contains(w))
  }
}

/// Lower case for the search, the Turkish I's folded together: "İ"
/// lowercases to "i" and a combining dot, and "ı" / "i" both match.
fn fold(s: &str) -> String {
  s.to_lowercase().chars().filter(|c| *c != '\u{307}').map(|c| if c == 'ı' { 'i' } else { c }).collect()
}

fn strs(v: &Value) -> Vec<String> {
  v.as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The list from the editor's model, built as the editor builds its rows:
/// the fixed ones (Super, the Dock, mouse), the core's (without removed
/// apps), the apps the user added, the window manager's. Unbound ones are
/// left out.
pub(super) fn entries(model: &Value, tr: &dyn Fn(&str) -> String) -> Vec<Entry> {
  let fixed: [(Cat, &str, &str); 6] = [
    (Cat::Apps, "Arama / overview", "Super"),
    (Cat::Apps, "Uygulama Dock’u", "Super+Alt"),
    (Cat::Windows, "Pencereyi taşı (sürükle)", "Super+Mouse L"),
    (Cat::Windows, "Pencereyi boyutlandır (sürükle)", "Super+Mouse R"),
    (Cat::Workspaces, "Workspace değiştir (tekerlek)", "Super+Wheel"),
    (Cat::Workspaces, "Gizli workspace’i aç / kapat", "Super+Mouse 4"),
  ];
  let mut out: Vec<Entry> = fixed.iter().filter_map(|(cat, label, combo)| Entry::new(*cat, tr(label), vec![combo.to_string()])).collect();
  for b in model["core"].as_array().into_iter().flatten() {
    if b["removed"].as_bool() == Some(true) {
      continue;
    }
    let id = b["id"].as_str().unwrap_or("");
    let app = b["app"].as_bool() == Some(true);
    let label = if b["custom"].as_bool() == Some(true) { b["name"].as_str().unwrap_or(id).to_string() } else { tr(&ll_label(id)) };
    out.extend(Entry::new(core_category(id, app), label, vec![b["combo"].as_str().unwrap_or("").to_string()]));
  }
  for g in model["tiling"].as_array().into_iter().flatten() {
    let cmds = strs(&g["commands"]);
    out.extend(Entry::new(tiling_category(&cmds), tr(&tiling_label(&cmds)), strs(&g["bindings"])));
  }
  out
}

/// Where each group goes: the shortest column each time (masonry). Returns
/// (column, top) per group and the tallest column's height.
pub(super) fn masonry(heights: &[f32], cols: usize) -> (Vec<(usize, f32)>, f32) {
  let mut tops = vec![0.0f32; cols.max(1)];
  let mut at = Vec::with_capacity(heights.len());
  for h in heights {
    let (col, top) = tops.iter().copied().enumerate().fold((0, f32::MAX), |best, (i, t)| if t < best.1 { (i, t) } else { best });
    at.push((col, top));
    tops[col] = top + h + GROUP_GAP;
  }
  let tallest = tops.iter().copied().fold(0.0, f32::max);
  (at, (tallest - GROUP_GAP).max(0.0))
}

fn columns(card_w: f32) -> usize {
  if card_w >= 980.0 {
    3
  } else if card_w >= 620.0 {
    2
  } else {
    1
  }
}

fn group_h(rows: usize) -> f32 {
  GROUP_HEAD + rows as f32 * ROW_H + GROUP_PAD
}

/// The card's size for a monitor of `mon_w` x `mon_h` DIP.
fn card_size(mon_w: f32, mon_h: f32) -> (f32, f32) {
  let w = CARD_MAX_W.min(mon_w - 2.0 * MARGIN).max(280.0);
  let h = (mon_h * CARD_H_SHARE).min(CARD_MAX_H).min(mon_h - 2.0 * MARGIN).max(240.0);
  (w.floor(), h.floor())
}

/// The window while the list is open.
pub(super) struct Sheet {
  hwnd: HWND,
  scale: f32,
  /// the card (DIP, without its shadow)
  card_w: f32,
  card_h: f32,
  /// the card surface's top left in the window (pixels)
  card_at: (f32, f32),
  _target: IDCompositionTarget,
  _root: IDCompositionVisual2,
  scrim: Layer,
  card: Layer,
  edit: Edit,
  surrogate: Option<u16>,
  scroll: f32,
  /// the list's height at the last paint (scroll limit)
  content_h: f32,
  entries: Vec<Entry>,
  /// None: still reading; Some(false): could not be read
  loaded: Option<bool>,
  active_at: Option<Instant>,
  closing: bool,
  cap_w: HashMap<String, f32>,
}

impl Sheet {
  fn view(&self) -> Rect {
    Rect::new(PAD, HEAD_H, self.card_w - 2.0 * PAD, self.card_h - HEAD_H - FOOT_H)
  }

  fn max_scroll(&self) -> f32 {
    (self.content_h - self.view().h).max(0.0)
  }

  fn scroll_by(&mut self, d: f32) -> bool {
    let s = (self.scroll + d).clamp(0.0, self.max_scroll());
    let changed = (s - self.scroll).abs() > 0.01;
    self.scroll = s;
    changed
  }

  /// Card coordinates (DIP) of a client point.
  fn card_point(&self, lp: LPARAM) -> (f32, f32) {
    let x = (lp.0 & 0xFFFF) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
    ((x - self.card_at.0) / self.scale - SHADOW, (y - self.card_at.1) / self.scale - SHADOW)
  }
}

/// The open list (if any) and the last model read (shown at once next time).
#[derive(Default)]
pub(super) struct CheatState {
  win: Option<Sheet>,
  model: Option<Value>,
}

fn make_win(gfx: &Gfx, rect: RECT, dpi: u32) -> anyhow::Result<Sheet> {
  let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
  let scale = super::scale::of_dpi(dpi);
  let (card_w, card_h) = card_size(w as f32 / scale, h as f32 / scale);
  let ex = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
  unsafe {
    let hwnd = CreateWindowExW(ex, CLASS, w!("lunge-cheatsheet"), WS_POPUP, rect.left, rect.top, w, h, None, None, GetModuleHandleW(None)?, None)?;
    let made = (|| -> windows::core::Result<Sheet> {
      let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
      let root = gfx.dcomp.CreateVisual()?;
      let scrim = Layer::new(gfx, SCRIM_PX, SCRIM_PX)?;
      scrim.visual.SetBitmapInterpolationMode(DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR)?;
      scrim.visual.SetBorderMode(DCOMPOSITION_BORDER_MODE_HARD)?;
      scrim.visual.SetTransform2(&Matrix3x2 {
        M11: w as f32 / SCRIM_PX as f32,
        M12: 0.0,
        M21: 0.0,
        M22: h as f32 / SCRIM_PX as f32,
        M31: 0.0,
        M32: 0.0,
      })?;
      let (cw, ch) = (((card_w + 2.0 * SHADOW) * scale).ceil(), ((card_h + 2.0 * SHADOW) * scale).ceil());
      let card = Layer::new(gfx, cw as u32, ch as u32)?;
      let card_at = (((w as f32 - cw) / 2.0).round(), ((h as f32 - ch) / 2.0).round());
      card.visual.SetOffsetX2(card_at.0)?;
      card.visual.SetOffsetY2(card_at.1)?;
      root.AddVisual(&scrim.visual, false, None)?;
      root.AddVisual(&card.visual, false, None)?;
      target.SetRoot(&root)?;
      Ok(Sheet {
        hwnd,
        scale,
        card_w,
        card_h,
        card_at,
        _target: target,
        _root: root,
        scrim,
        card,
        edit: Edit::default(),
        surrogate: None,
        scroll: 0.0,
        content_h: 0.0,
        entries: Vec::new(),
        loaded: None,
        active_at: None,
        closing: false,
        cap_w: HashMap::new(),
      })
    })();
    made.map_err(|err| {
      let _ = DestroyWindow(hwnd);
      err.into()
    })
  }
}

/// The monitor under the pointer (rectangle and DPI); the primary one, or
/// the first, when that cannot be told.
fn pointer_monitor() -> Option<(RECT, u32)> {
  let mons = super::monitor_layout();
  let mut pt = POINT::default();
  let _ = unsafe { GetCursorPos(&mut pt) };
  let at = |&&(l, t, r, b, _): &&(i32, i32, i32, i32, u32)| pt.x >= l && pt.x < r && pt.y >= t && pt.y < b;
  let m = mons.iter().find(at).or_else(|| mons.iter().find(|m| m.0 == 0 && m.1 == 0)).or(mons.first())?;
  Some((RECT { left: m.0, top: m.1, right: m.2, bottom: m.3 }, m.4))
}

/// Draws the key caps of `combos` (the first two) right-aligned to `right`,
/// centred on `cy`; returns their width.
fn paint_caps(p: &mut Painter, t: &Theme, cache: &mut HashMap<String, f32>, combos: &[Vec<String>], right: f32, cy: f32) -> anyhow::Result<f32> {
  let style = TextStyle { size: 12.0, weight: 600.0 };
  let mut widths = Vec::new();
  for (i, combo) in combos.iter().take(2).enumerate() {
    if i > 0 {
      widths.push(None);
    }
    for cap in combo {
      let w = match cache.get(cap) {
        Some(w) => *w,
        None => {
          let w = (p.measure(cap, style)? + 14.0).max(CAP_H + 2.0).ceil();
          cache.insert(cap.clone(), w);
          w
        }
      };
      widths.push(Some((cap, w)));
    }
  }
  const GAP: f32 = 4.0;
  const OR_W: f32 = 14.0;
  let total: f32 = widths.iter().map(|w| w.map_or(OR_W, |(_, w)| w)).sum::<f32>() + GAP * widths.len().saturating_sub(1) as f32;
  let mut x = right - total;
  for w in widths {
    match w {
      Some((cap, w)) => {
        let r = Rect::new(x, cy - CAP_H / 2.0, w, CAP_H);
        p.fill_round(r, 6.0, t.sec_container)?;
        p.fill(Rect::new(r.x + 3.0, r.bottom() - 1.0, r.w - 6.0, 1.0), Rgba(0, 0, 0, 0.18))?;
        p.text(cap, r, style, t.on_sec_container, Align::Center, false)?;
        x += w + GAP;
      }
      None => {
        p.text("/", Rect::new(x, cy - CAP_H / 2.0, OR_W, CAP_H), TextStyle { size: 12.0, weight: 450.0 }, t.subtext, Align::Center, false)?;
        x += OR_W + GAP;
      }
    }
  }
  Ok(total)
}

/// The card: head (title, search), the groups in columns, the hint.
fn paint_card(p: &mut Painter, t: &Theme, s: &mut Sheet, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
  let card = Rect::new(SHADOW, SHADOW, s.card_w, s.card_h);
  for k in 1..=12 {
    let g = k as f32 * 3.0;
    p.fill_round(Rect::new(card.x - g, card.y + 10.0 - g, card.w + 2.0 * g, card.h + 2.0 * g), RADIUS + g, Rgba(0, 0, 0, 0.035))?;
  }
  p.fill_round(card, RADIUS, t.layer0)?;
  p.stroke_round(card.inset(0.5, 0.5), RADIUS, t.border, 1.0)?;

  // head: icon and title, the search field on the right
  let (ox, oy) = (card.x, card.y);
  p.icon("keyboard", ox + PAD + 14.0, oy + HEAD_H / 2.0 + 2.0, 26.0, false, t.primary)?;
  let fw = FIELD_W.min((s.card_w - 2.0 * PAD) * 0.45);
  let field = Rect::new(ox + s.card_w - PAD - fw, oy + (HEAD_H - FIELD_H) / 2.0 + 4.0, fw, FIELD_H);
  let title_w = field.x - (ox + PAD + 36.0) - 12.0;
  p.text(&tr("Klavye kısayolları"), Rect::new(ox + PAD + 36.0, oy + 4.0, title_w.max(0.0), HEAD_H), TextStyle { size: 22.0, weight: 600.0 }, t.on_layer0, Align::Left, false)?;
  paint_field(p, t, &s.edit, field, tr)?;

  // the groups that have something for the search
  let query = s.edit.text();
  let groups: Vec<(Cat, Vec<&Entry>)> = CATS
    .iter()
    .map(|c| (*c, s.entries.iter().filter(|e| e.cat == *c && e.matches(&query)).collect::<Vec<_>>()))
    .filter(|(_, rows)| !rows.is_empty())
    .collect();
  let view = s.view();
  let view = Rect::new(ox + view.x, oy + view.y, view.w, view.h);
  let cols = columns(s.card_w);
  let col_w = (view.w - 8.0 - COL_GAP * (cols - 1) as f32) / cols as f32;
  let heights: Vec<f32> = groups.iter().map(|(_, rows)| group_h(rows.len())).collect();
  let (at, content_h) = masonry(&heights, cols);
  s.content_h = content_h;
  s.scroll = s.scroll.clamp(0.0, s.max_scroll());

  let empty = match s.loaded {
    None if s.entries.is_empty() => Some(tr("Yükleniyor…")),
    Some(false) if s.entries.is_empty() => Some(tr("Kısayollar okunamadı")),
    _ if groups.is_empty() => Some(tr("Eşleşen kısayol yok")),
    _ => None,
  };
  if let Some(msg) = empty {
    p.text(&msg, Rect::new(view.x, view.y, view.w, 80.0), TextStyle { size: 14.0, weight: 450.0 }, t.subtext, Align::Center, false)?;
  }

  unsafe { p.dc.PushAxisAlignedClip(&view.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
  let drawn = (|| -> anyhow::Result<()> {
    let label_style = TextStyle { size: 13.5, weight: 450.0 };
    for ((cat, rows), (col, top)) in groups.iter().zip(&at) {
      let h = group_h(rows.len());
      let r = Rect::new(view.x + *col as f32 * (col_w + COL_GAP), view.y + top - s.scroll, col_w, h);
      if r.bottom() < view.y || r.y > view.bottom() {
        continue;
      }
      p.fill_round(r, 18.0, t.layer1)?;
      p.icon(cat.icon(), r.x + 24.0, r.y + GROUP_HEAD / 2.0 + 2.0, 20.0, false, t.primary)?;
      p.text(&tr(cat.name()), Rect::new(r.x + 44.0, r.y + 2.0, r.w - 56.0, GROUP_HEAD), TextStyle { size: 14.0, weight: 600.0 }, t.primary, Align::Left, false)?;
      for (i, e) in rows.iter().enumerate() {
        let y = r.y + GROUP_HEAD + i as f32 * ROW_H;
        if y + ROW_H < view.y || y > view.bottom() {
          continue;
        }
        let cy = y + ROW_H / 2.0;
        let caps_w = paint_caps(p, t, &mut s.cap_w, &e.combos, r.right() - 12.0, cy)?;
        let lw = (r.w - 16.0 - 12.0 - caps_w - 10.0).max(0.0);
        p.text(&e.label, Rect::new(r.x + 16.0, y, lw, ROW_H), label_style, t.on_layer1, Align::Left, false)?;
      }
    }
    Ok(())
  })();
  unsafe { p.dc.PopAxisAlignedClip() };
  drawn?;
  p.scrollbar(Rect::new(view.x, view.y, view.w + 6.0, view.h), content_h, s.scroll, t.outline_variant)?;

  let hint = Rect::new(ox + PAD, oy + s.card_h - FOOT_H, s.card_w - 2.0 * PAD, FOOT_H - 6.0);
  p.text(&tr("Esc: kapat  ·  Super+/: aç / kapat"), hint, TextStyle { size: 12.5, weight: 450.0 }, t.subtext, Align::Center, false)?;
  Ok(())
}

/// The search field: placeholder or text, the selection and the caret.
fn paint_field(p: &mut Painter, t: &Theme, edit: &Edit, r: Rect, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
  p.fill_round(r, r.h / 2.0, t.surface_container_high)?;
  p.icon("search", r.x + 22.0, r.y + r.h / 2.0, 20.0, false, t.on_surface_variant)?;
  let style = TextStyle { size: 14.0, weight: 450.0 };
  let tx = r.x + 42.0;
  let tw = r.right() - 16.0 - tx;
  let text = edit.text();
  if text.is_empty() {
    p.text(&tr("Kısayollarda ara"), Rect::new(tx, r.y, tw, r.h), style, t.on_surface_variant, Align::Left, false)?;
  }
  fn prefix(p: &mut Painter, edit: &Edit, n: usize, style: TextStyle) -> anyhow::Result<f32> {
    let s: String = edit.chars[..n.min(edit.chars.len())].iter().collect();
    // trailing spaces count (DirectWrite leaves them out of the width)
    let with_bar = p.measure(&format!("{s}|"), style)?;
    Ok(with_bar - p.measure("|", style)?)
  }
  let (a, b) = edit.selection();
  let caret_x = prefix(p, edit, edit.caret, style)?;
  // a long search scrolls left so the caret stays in the field
  let shift = (caret_x - tw + 2.0).max(0.0);
  unsafe { p.dc.PushAxisAlignedClip(&Rect::new(tx - 1.0, r.y, tw + 3.0, r.h).d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
  let drawn = (|| -> anyhow::Result<()> {
    if a != b {
      let (xa, xb) = (prefix(p, edit, a, style)?, prefix(p, edit, b, style)?);
      p.fill(Rect::new(tx + xa - shift, r.y + 9.0, xb - xa, r.h - 18.0), Rgba(t.primary.0, t.primary.1, t.primary.2, 0.35))?;
    }
    if !text.is_empty() {
      p.text(&text, Rect::new(tx - shift, r.y, tw + shift, r.h), style, t.on_layer0, Align::Left, false)?;
    }
    p.fill(Rect::new(tx + caret_x - shift, r.y + 10.0, 1.5, r.h - 20.0), t.primary)?;
    Ok(())
  })();
  unsafe { p.dc.PopAxisAlignedClip() };
  drawn
}

impl Ui {
  /// `ll:cheatsheet-toggle` (Super+/) and the Super menu's `/keys`
  pub(super) fn cheatsheet_toggle(&mut self) {
    match &self.cheatsheet.win {
      Some(s) if !s.closing => self.cheatsheet_close(),
      _ => self.cheatsheet_open(),
    }
  }

  pub(super) fn cheatsheet_open(&mut self) {
    self.cheatsheet_destroy();
    self.overview_hide();
    self.settings_close();
    self.session_close();
    let Some((rect, dpi)) = pointer_monitor() else { return };
    let mut sheet = match make_win(&self.gfx, rect, dpi) {
      Ok(s) => s,
      Err(err) => {
        tracing::warn!("Cheatsheet: window: {:?}", err);
        return;
      }
    };
    if let Some(m) = &self.cheatsheet.model {
      let model = &self.model;
      sheet.entries = entries(m, &|s| model.tr(s));
    }
    CHEATSHEET_HWND.store(sheet.hwnd.0 as isize, Ordering::Release);
    self.cheatsheet.win = Some(sheet);
    // the shortcuts as they are now (the user may have changed them)
    std::thread::spawn(|| {
      let model = core_api::run_core_output(&["--keybinds-model"]).and_then(|s| serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object);
      super::send(Msg::Cheatsheet(model));
    });
    if let Err(err) = self.cheatsheet_paint(true) {
      tracing::warn!("Cheatsheet: paint: {:?}", err);
      self.cheatsheet_destroy();
      return;
    }
    if let Err(err) = self.cheatsheet_enter() {
      tracing::debug!("Cheatsheet: entrance: {:?}", err);
    }
    if let Some(s) = &self.cheatsheet.win {
      unsafe {
        let _ = ShowWindow(s.hwnd, SW_SHOW);
        let _ = SetForegroundWindow(s.hwnd);
      }
    }
  }

  /// The model came back from the core (None: it could not be read).
  pub(super) fn cheatsheet_loaded(&mut self, model: Option<Value>) {
    if let Some(m) = &model {
      self.cheatsheet.model = Some(m.clone());
    }
    let tr_model = &self.model;
    let Some(s) = self.cheatsheet.win.as_mut() else { return };
    match &model {
      Some(m) => {
        s.entries = entries(m, &|t| tr_model.tr(t));
        s.loaded = Some(true);
      }
      None => {
        tracing::warn!("Cheatsheet: the shortcut model could not be read");
        s.loaded = Some(s.loaded == Some(true) || !s.entries.is_empty());
      }
    }
    if let Err(err) = self.cheatsheet_paint(false) {
      tracing::warn!("Cheatsheet: paint: {:?}", err);
    }
  }

  fn cheatsheet_paint(&mut self, scrim: bool) -> anyhow::Result<()> {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, model, cheatsheet, .. } = self;
    let Some(s) = cheatsheet.win.as_mut() else { return Ok(()) };
    let tr = |t: &str| model.tr(t);
    let mut requests = Vec::new();
    let surface = s.card.surface.clone();
    let scale = s.scale;
    gfx::draw_surface(&surface, scale, |dc| {
      let mut p = Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      if let Err(err) = paint_card(&mut p, &theme, s, &tr) {
        tracing::warn!("Cheatsheet: card: {:?}", err);
      }
      Ok(())
    })?;
    if scrim {
      paint_scrim(&s.scrim.surface)?;
    }
    unsafe { gfx.dcomp.Commit()? };
    Ok(())
  }

  /// As the session screen: the scrim fades in, the card rises and grows.
  fn cheatsheet_enter(&self) -> windows::core::Result<()> {
    let Some(s) = &self.cheatsheet.win else { return Ok(()) };
    if !self.model.animations {
      return Ok(());
    }
    let dcomp = &self.gfx.dcomp;
    unsafe {
      for layer in [&s.scrim, &s.card] {
        let v: IDCompositionVisual3 = layer.visual.cast()?;
        v.SetOpacity(&anim::build(dcomp, 0.0, 1.0, 220.0, POP_IN)?)?;
      }
      let (cw, ch) = ((s.card_w + 2.0 * SHADOW) * s.scale, (s.card_h + 2.0 * SHADOW) * s.scale);
      let grow = dcomp.CreateScaleTransform()?;
      grow.SetCenterX2(cw / 2.0)?;
      grow.SetCenterY2(ch / 2.0)?;
      let size = anim::build(dcomp, 0.96, 1.0, 340.0, POP_IN)?;
      grow.SetScaleX(&size)?;
      grow.SetScaleY(&size)?;
      let rise = dcomp.CreateTranslateTransform()?;
      rise.SetOffsetY(&anim::build(dcomp, 18.0 * s.scale, 0.0, 340.0, POP_IN)?)?;
      let moves = dcomp.CreateTransformGroup(&[Some(grow.cast()?), Some(rise.cast()?)])?;
      s.card.visual.SetTransform(&moves)?;
      dcomp.Commit()
    }
  }

  /// Fades out (or, with animations off, goes at once).
  pub(super) fn cheatsheet_close(&mut self) {
    match self.cheatsheet.win.as_mut() {
      Some(s) if !s.closing => s.closing = true,
      _ => return,
    }
    if !self.model.animations || self.cheatsheet_fade_out().is_err() {
      self.cheatsheet_destroy();
      return;
    }
    unsafe { SetTimer(self.msg_hwnd, TIMER_CHEATSHEET_CLOSE, 150, None) };
  }

  fn cheatsheet_fade_out(&self) -> windows::core::Result<()> {
    let Some(s) = &self.cheatsheet.win else { return Ok(()) };
    let dcomp = &self.gfx.dcomp;
    unsafe {
      for layer in [&s.scrim, &s.card] {
        let v: IDCompositionVisual3 = layer.visual.cast()?;
        v.SetOpacity(&anim::build(dcomp, 1.0, 0.0, 140.0, POP_OUT)?)?;
      }
      dcomp.Commit()
    }
  }

  pub(super) fn cheatsheet_destroy(&mut self) {
    unsafe {
      let _ = KillTimer(self.msg_hwnd, TIMER_CHEATSHEET_CLOSE);
    }
    CHEATSHEET_HWND.store(0, Ordering::Release);
    if let Some(s) = self.cheatsheet.win.take() {
      unsafe {
        let _ = DestroyWindow(s.hwnd);
      }
    }
  }

  /// A message for the list's window (None: not its window).
  pub(super) fn cheatsheet_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    let s = self.cheatsheet.win.as_mut().filter(|s| s.hwnd == hwnd)?;
    if msg == WM_PAINT {
      unsafe {
        let _ = ValidateRect(hwnd, None);
      }
      return Some(Some(LRESULT(0)));
    }
    if s.closing {
      return Some(None);
    }
    let (result, then) = match msg {
      WM_KEYDOWN | WM_SYSKEYDOWN => key(s, wp.0 as u16, msg == WM_SYSKEYDOWN),
      WM_CHAR => (Some(LRESULT(0)), typed(s, wp.0 as u16)),
      WM_MOUSEWHEEL => {
        let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
        (Some(LRESULT(0)), if s.scroll_by(-delta / 120.0 * STEP) { Then::Paint } else { Then::Nothing })
      }
      WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
        let (x, y) = s.card_point(lp);
        let inside = (0.0..=s.card_w).contains(&x) && (0.0..=s.card_h).contains(&y);
        (Some(LRESULT(0)), if inside { Then::Nothing } else { Then::Close })
      }
      WM_ACTIVATE => {
        let mut then = Then::Nothing;
        if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
          if s.active_at.is_some_and(|t| t.elapsed().as_millis() > BLUR_GRACE_MS) {
            then = Then::Close;
          }
        } else {
          s.active_at = Some(Instant::now());
        }
        (None, then)
      }
      WM_CLOSE => (Some(LRESULT(0)), Then::Close),
      _ => (None, Then::Nothing),
    };
    match then {
      Then::Nothing => {}
      Then::Paint => {
        if let Err(err) = self.cheatsheet_paint(false) {
          tracing::warn!("Cheatsheet: paint: {:?}", err);
        }
      }
      Then::Close => self.cheatsheet_close(),
    }
    Some(result)
  }
}

/// What a message leads to once the list's state is updated.
enum Then {
  Nothing,
  Paint,
  Close,
}

/// WM_KEYDOWN: the search field's editing keys, scrolling, Esc.
fn key(s: &mut Sheet, vk: u16, sys: bool) -> (Option<LRESULT>, Then) {
  // AltGr arrives as Ctrl+Alt: it types a character, it is no Ctrl shortcut
  let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 && GetKeyState(VK_MENU.0 as i32) >= 0 };
  let page = s.view().h * 0.9;
  let edited = |s: &mut Sheet| {
    s.scroll = 0.0;
    Then::Paint
  };
  let then = match vk {
    0x1B if !s.edit.chars.is_empty() => {
      s.edit.set("");
      edited(s)
    }
    0x1B => Then::Close,
    0x08 => {
      s.edit.backspace(ctrl);
      edited(s)
    }
    0x2E => {
      s.edit.delete(ctrl);
      edited(s)
    }
    0x41 if ctrl => {
      s.edit.select_all();
      Then::Paint
    }
    // Ctrl+F: the field always has the keyboard; select its text to replace it
    0x46 if ctrl => {
      s.edit.select_all();
      Then::Paint
    }
    0x5A if ctrl => {
      if s.edit.undo() {
        edited(s)
      } else {
        Then::Nothing
      }
    }
    0x25 => {
      s.edit.left(ctrl, false);
      Then::Paint
    }
    0x27 => {
      s.edit.right(ctrl, false);
      Then::Paint
    }
    0x26 | 0x28 | 0x21 | 0x22 | 0x24 | 0x23 => {
      let d = match vk {
        0x26 => -STEP,
        0x28 => STEP,
        0x21 => -page,
        0x22 => page,
        0x24 => -f32::MAX / 4.0,
        _ => f32::MAX / 4.0,
      };
      if s.scroll_by(d) {
        Then::Paint
      } else {
        Then::Nothing
      }
    }
    // Alt+F4 comes back as WM_CLOSE
    _ if sys => return (None, Then::Nothing),
    _ => Then::Nothing,
  };
  (Some(LRESULT(0)), then)
}

/// WM_CHAR (UTF-16 units; control characters come as keys).
fn typed(s: &mut Sheet, unit: u16) -> Then {
  if (0xD800..0xDC00).contains(&unit) {
    s.surrogate = Some(unit);
    return Then::Nothing;
  }
  let text = if (0xDC00..0xE000).contains(&unit) {
    match s.surrogate.take() {
      Some(high) => String::from_utf16_lossy(&[high, unit]),
      None => return Then::Nothing,
    }
  } else {
    s.surrogate = None;
    if unit < 0x20 || unit == 0x7F {
      return Then::Nothing;
    }
    String::from_utf16_lossy(&[unit])
  };
  s.edit.insert(&text);
  s.scroll = 0.0;
  Then::Paint
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::*;

  #[test]
  fn core_shortcuts_land_in_their_groups() {
    assert_eq!(core_category("ws-3", false), Cat::Workspaces);
    assert_eq!(core_category("ws-move-next", false), Cat::Workspaces);
    assert_eq!(core_category("workspaces", false), Cat::Workspaces);
    assert_eq!(core_category("focus-left", false), Cat::Windows);
    assert_eq!(core_category("move-up", false), Cat::Windows);
    assert_eq!(core_category("close", false), Cat::Windows);
    assert_eq!(core_category("browser", true), Cat::Apps);
    assert_eq!(core_category("app:k2x", true), Cat::Apps);
    assert_eq!(core_category("overview-alt", false), Cat::Apps);
    assert_eq!(core_category("clipboard", false), Cat::Apps);
    assert_eq!(core_category("screenshot-alt", false), Cat::Tools);
    assert_eq!(core_category("ocr", false), Cat::Tools);
    assert_eq!(core_category("color-picker", false), Cat::Tools);
    assert_eq!(core_category("cheatsheet", false), Cat::Tools);
    assert_eq!(core_category("settings", false), Cat::System);
    assert_eq!(core_category("task-manager", false), Cat::System);
  }

  #[test]
  fn window_manager_shortcuts_land_in_their_groups() {
    let c = |s: &str| tiling_category(&s.split(" ; ").map(str::to_string).collect::<Vec<_>>());
    assert_eq!(c("focus --next-workspace"), Cat::Workspaces);
    assert_eq!(c("move --workspace 3"), Cat::Workspaces);
    assert_eq!(c("toggle-special-workspace"), Cat::Workspaces);
    assert_eq!(c("toggle-floating --centered"), Cat::Layout);
    assert_eq!(c("toggle-fullscreen"), Cat::Layout);
    assert_eq!(c("resize --width +10%"), Cat::Layout);
    assert_eq!(c("split-ratio 0.1"), Cat::Layout);
    assert_eq!(c("move --workspace-in-direction left ; focus --workspace-in-direction left"), Cat::Layout);
    assert_eq!(c("move-workspace --direction right"), Cat::Layout);
    assert_eq!(c("wm-enable-binding-mode --name resize"), Cat::Layout);
    assert_eq!(c("shell-exec sndvol"), Cat::Media);
    assert_eq!(c("shell-exec ms-settings:"), Cat::System);
    assert_eq!(c("shell-exec %LUNGE_HOME%\\tools\\wezterm\\wezterm-gui.exe"), Cat::Apps);
    assert_eq!(c("wm-reload-config"), Cat::System);
    assert_eq!(c("wm-cycle-focus"), Cat::Windows);
    assert_eq!(c("toggle-minimized"), Cat::Windows);
  }

  #[test]
  fn combos_become_key_caps() {
    assert_eq!(keycaps("Super+Ctrl+Left"), ["Super", "Ctrl", "←"]);
    assert_eq!(keycaps("Super+Shift+/"), ["Super", "Shift", "/"]);
    assert_eq!(keycaps("Super+Enter"), ["Super", "↵"]);
    assert_eq!(keycaps("Super+PageDown"), ["Super", "PgDn"]);
    assert!(keycaps("").is_empty());
  }

  #[test]
  fn the_search_looks_at_labels_and_keys() {
    let e = Entry::new(Cat::Tools, "Ekrandan metin (OCR)".into(), vec!["Super+Shift+T".into()]).unwrap();
    assert!(e.matches(""));
    assert!(e.matches("ocr"));
    assert!(e.matches("METİN"), "case does not matter");
    assert!(e.matches("METIN") && e.matches("metın"), "dotted and dotless i alike");
    assert!(e.matches("shift t"), "every word, anywhere");
    assert!(e.matches("super+shift+t"));
    assert!(!e.matches("renk"));
    assert!(!e.matches("ocr renk"));
    let arrows = Entry::new(Cat::Windows, "Sola odaklan".into(), vec!["Super+Left".into()]).unwrap();
    assert!(arrows.matches("left") && arrows.matches("←"));
    assert!(Entry::new(Cat::Apps, "Bağlı değil".into(), vec!["".into()]).is_none(), "unbound rows are left out");
  }

  #[test]
  fn the_list_is_built_from_the_editor_model() {
    let model = json!({
      "core": [
        { "id": "ws-1", "combo": "Super+1", "app": false, "custom": false, "removed": false },
        { "id": "cheatsheet", "combo": "Super+/", "app": false, "custom": false, "removed": false },
        { "id": "files", "combo": "Super+E", "app": true, "custom": false, "removed": true },
        { "id": "ws-prev5", "combo": "", "app": false, "custom": false, "removed": false },
        { "id": "app:x1", "combo": "Super+Alt+N", "app": true, "custom": true, "removed": false, "name": "Notepad" },
      ],
      "tiling": [
        { "index": 0, "commands": ["toggle-floating --centered"], "bindings": ["Super+Alt+Space", "Super+G"] },
        { "index": 1, "commands": ["shell-exec sndvol"], "bindings": [] },
      ],
    });
    let list = entries(&model, &|s| s.to_string());
    let find = |label: &str| list.iter().find(|e| e.label == label);
    assert_eq!(find("Workspace 1").map(|e| e.cat), Some(Cat::Workspaces));
    assert_eq!(find("Kısayol listesi").map(|e| e.combos.clone()), Some(vec![vec!["Super".to_string(), "/".to_string()]]));
    assert!(find("Dosya yöneticisi").is_none(), "a removed app");
    assert!(find("5 workspace geri").is_none(), "a shortcut without keys");
    assert_eq!(find("Notepad").map(|e| e.cat), Some(Cat::Apps));
    assert_eq!(find("Yüzen pencere").map(|e| (e.cat, e.combos.len())), Some((Cat::Layout, 2)));
    assert!(find("Ses karıştırıcı").is_none());
    assert_eq!(find("Arama / overview").map(|e| e.cat), Some(Cat::Apps));
  }

  #[test]
  fn groups_go_to_the_shortest_column() {
    let (at, h) = masonry(&[100.0, 50.0, 40.0, 30.0], 2);
    assert_eq!(at[0], (0, 0.0));
    assert_eq!(at[1], (1, 0.0));
    assert_eq!(at[2], (1, 50.0 + GROUP_GAP));
    assert_eq!(at[3], (0, 100.0 + GROUP_GAP), "column 0 (116) is now shorter than column 1 (122)");
    assert_eq!(h, 100.0 + GROUP_GAP + 30.0);
    assert_eq!(masonry(&[], 3), (vec![], 0.0));
  }

  #[test]
  fn the_card_fits_small_monitors() {
    assert_eq!(card_size(2560.0, 1440.0), (CARD_MAX_W, CARD_MAX_H));
    let (w, h) = card_size(800.0, 600.0);
    assert!(w <= 800.0 - 2.0 * MARGIN && h <= 600.0 * CARD_H_SHARE);
    assert_eq!(columns(CARD_MAX_W), 3);
    assert_eq!(columns(w), 2);
    assert_eq!(columns(400.0), 1);
  }
}
