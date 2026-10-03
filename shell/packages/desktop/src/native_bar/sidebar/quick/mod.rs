//! Quick settings (tiles and the cards under a tile): five columns of 56 DIP tiles,
//! an edit mode (drag to move, click to add / remove, right click for the
//! size, wheel to swap), and the cards a wide tile opens: Wi-Fi networks,
//! Ethernet, Bluetooth devices, the output / input device with its volume,
//! the night light.
//!
//! Hardware state comes from the core (`/qs/*`, `--nightlight`, `--mic`,
//! Wi-Fi through `/qs/wifi*`) on worker threads; audio and the Wi-Fi name from the
//! bar's providers.

mod actions;
mod cards;
mod defs;
pub(super) mod drag;

use std::{
  collections::HashMap,
  time::{Duration, Instant},
};

use serde_json::{json, Value};
use windows::Win32::Graphics::Direct2D::{ID2D1StrokeStyle};

use super::{
  super::{
    anim::{Curve, POP_IN, POP_OUT},
    core_api,
    gfx::{Rect, Rgba},
    model::Model,
    send, Msg, Ui,
  },
  kit::{blend, lerp_rect, st, stw, Cx},
  store::{Tile, Toggle, AVAILABLE},
  text::{TextField, Typed},
  Ev, FieldId, Hit, ScrollId,
};
use crate::providers::{AudioFunction, ProviderFunction, SetMuteArgs, SetVolumeArgs};

pub(super) const COLUMNS: u8 = 5;
pub(super) const CELL_H: f32 = 56.0;
pub(super) const SPACING: f32 = 6.0;
pub(super) const PADDING: f32 = 6.0;
/// `--clickBounce`-free tile moves (edit mode): 200 ms, cubic-bezier(.2, .8, .2, 1)
const MOVE: Curve = Curve(0.2, 0.8, 0.2, 1.0);
const CARD_IN_MS: f32 = 260.0;
const CARD_OUT_MS: f32 = 180.0;
/// the hardware state is read at most this often (each read costs the core
/// four requests and two processes)
const REFRESH_EVERY: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq)]
pub(super) enum QHit {
  Tile(Tile),
  /// the round icon of a wide tile with a card: switches it
  TileIcon(Tile),
  Edit,
  /// the card's background (clicks there keep it open)
  Card,
  Net(String),
  NetSubmit,
  Scan,
  More(&'static str),
  AudioMute,
  AudioSlider,
  AudioDev(String),
  NightToggle,
  NightMode(&'static str),
  NightSlider,
  NightPreset(u32),
}

/// What a worker read.
pub(in crate::native_bar) enum QEv {
  Radios(Value),
  Eth(Value),
  Bt(Value),
  Night(Value),
  Status(Value),
  Mic(Option<bool>),
  Wifi(Option<Value>),
  WifiConnected(String, Value),
  /// a switch's source, read after it was asked to switch (`verdict`):
  /// None when the core gave no answer; `hint` says what would repair it
  Check { tile: Tile, want: bool, until: Instant, hint: Option<String>, read: Option<Box<QEv>> },
  AudioDefault(String),
  ScanDone,
}

/// How long a switch may take to show in its source (an adapter comes up in
/// seconds), and how often it is read meanwhile.
const VERIFY_FOR: Duration = Duration::from_secs(10);
const VERIFY_EVERY: Duration = Duration::from_millis(500);

/// Where the state a tile's switch flips is read: one source per state the
/// hardware or the core owns. Tiles without one keep a state of ours (a
/// preference) or get theirs back from a provider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Source {
  Radios,
  Eth,
  Mic,
  Night,
  Status,
}

impl Source {
  const ALL: [Source; 5] = [Source::Radios, Source::Eth, Source::Mic, Source::Night, Source::Status];

  pub(super) fn of(tile: Tile) -> Option<Source> {
    match tile {
      Tile::Wifi | Tile::Bluetooth => Some(Source::Radios),
      Tile::Ethernet => Some(Source::Eth),
      Tile::Mic => Some(Source::Mic),
      Tile::NightLight => Some(Source::Night),
      Tile::IdleInhibitor => Some(Source::Status),
      Tile::DarkMode | Tile::ScreenSnip | Tile::OnScreenKeyboard | Tile::Audio | Tile::Notifications => None,
    }
  }

  /// Reads it (blocking: a worker thread); None when the core gave no answer.
  fn read(self) -> Option<QEv> {
    match self {
      Source::Radios => qs("radios").map(QEv::Radios),
      Source::Eth => qs("eth").map(QEv::Eth),
      Source::Night => core_json(&["--nightlight", "status"]).map(QEv::Night),
      Source::Status => qs("status").map(QEv::Status),
      Source::Mic => core_json(&["--mic", "status"]).and_then(|v| v["muted"].as_bool()).map(|muted| QEv::Mic(Some(!muted))),
    }
  }
}

/// What a switch's source showed after the switch was asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Seen {
  /// the core gave no answer
  NoAnswer,
  /// on / off, or neither (a wired adapter without a cable)
  Read(Option<bool>),
}

#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
  Done,
  Again,
  Failed,
}

/// A switch is done once its source shows it as asked. Until the time is up
/// it is read again; then it failed if the source shows the opposite or
/// never answered (neither on nor off is no failure: nothing contradicts it).
pub(super) fn verdict(want: bool, seen: Seen, left: Duration) -> Verdict {
  match seen {
    Seen::Read(Some(on)) if on == want => Verdict::Done,
    _ if !left.is_zero() => Verdict::Again,
    Seen::Read(None) => Verdict::Done,
    _ => Verdict::Failed,
  }
}

/// A switch's request on a worker, then its check: the tile's source is read
/// until it shows the switch as asked (`verdict`; the panel shows each read).
/// `request` returns what would repair a failure, if it knows.
fn switch(tile: Tile, want: bool, request: impl FnOnce() -> Option<String> + Send + 'static) {
  spawn(move || {
    let hint = request();
    if Source::of(tile).is_some() {
      check(tile, want, Instant::now() + VERIFY_FOR, hint);
    }
  });
}

/// One read of the tile's source for its check (on a worker).
fn check(tile: Tile, want: bool, until: Instant, hint: Option<String>) {
  let read = Source::of(tile).and_then(Source::read).map(Box::new);
  ev(QEv::Check { tile, want, until, hint, read });
}

/// The hardware as last read (also kept in the store's cache).
#[derive(Clone, Default)]
pub(super) struct Hw {
  /// {"wifi": "On" | "Off" | null, "bluetooth": ...}
  pub radios: Value,
  /// {"state", "desc", "speed", "ip"}
  pub eth: Value,
  /// {"adapter", "on", "devices": [{"name", "connected", "kind"}]}
  pub bt: Value,
  pub awake: bool,
  /// the microphone is on (None: not read yet)
  pub mic: Option<bool>,
  /// {"on", "active", "level", "mode", "from", "to"}
  pub night: Value,
}

struct Press {
  tile: Toggle,
  x0: f32,
  y0: f32,
  /// the pointer now (a long press lifts the tile where it is)
  x: f32,
  y: f32,
  dx: f32,
  dy: f32,
  w: f32,
  h: f32,
  at: Instant,
  /// where it was: (in the used rows, index)
  orig: (bool, usize),
}

/// A tile lifted and following the pointer. The pointer keeps its grab
/// offset (`dx`, `dy`) so the tile does not jump on the first move.
pub(super) struct Drag {
  pub tile: Toggle,
  pub x: f32,
  pub y: f32,
  pub dx: f32,
  pub dy: f32,
  pub w: f32,
  pub h: f32,
  to_used: bool,
  index: usize,
  orig: (bool, usize),
}

/// What a pointer move did to a press on a tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Moved {
  Nothing,
  /// it lifted now (a long press or a move past the threshold)
  Lifted,
  /// it follows the pointer, same drop place
  Moved,
  /// the drop place changed: the other tiles move
  Retargeted,
}

/// a press held this long lifts the tile without moving
pub(super) const LIFT_MS: u128 = 350;
/// the other tiles slide to their new places, the dropped one lands
pub(super) const SLIDE_MS: f32 = 180.0;

struct CardAnim {
  tile: Tile,
  top: f32,
  at: Instant,
  closing: bool,
}

#[derive(Default)]
pub(super) struct Quick {
  pub toggles: Vec<Toggle>,
  pub edit: bool,
  pub menu: Option<Tile>,
  card: Option<CardAnim>,
  pub hw: Hw,
  last_refresh: Option<Instant>,
  awake_restarted: bool,
  press: Option<Press>,
  pub drag: Option<Drag>,
  /// a drag just ended: the click that follows does not add / remove
  dragged: bool,
  /// the dropped (or given back) tile while its ghost lands: not drawn yet
  pub landing: Option<(Tile, Instant)>,
  /// where each tile was drawn (cards open under it), and in the edit mode
  /// the moves under way
  placed: HashMap<Tile, Rect>,
  shown: HashMap<Tile, Rect>,
  moving: HashMap<Tile, (Rect, Rect, Instant)>,
  /// the tiles' areas in the last paint (drop targets)
  used_area: Rect,
  unused_area: Rect,
  /// the panel's rectangle and the open card's (outside clicks close it)
  pub area: Rect,
  pub card_rect: Option<Rect>,
  // cards
  wifi: Option<Value>,
  wifi_scanning: bool,
  wifi_busy: Option<String>,
  wifi_ask: Option<String>,
  wifi_err: String,
  wifi_scanned: Option<Instant>,
  scanning: bool,
  audio_busy: Option<String>,
  /// volume being dragged: (device id, value, when last sent)
  audio_drag: Option<(String, u32, Instant)>,
  audio_track: Rect,
  night_level: Option<u32>,
  night_drag: bool,
  night_track: Rect,
  night_sent: Option<Instant>,
}

pub(super) fn rows_for(list: &[Toggle]) -> Vec<Vec<Toggle>> {
  let mut rows = Vec::new();
  let mut row: Vec<Toggle> = Vec::new();
  let mut total = 0u8;
  for t in list {
    if total + t.size > COLUMNS {
      rows.push(std::mem::take(&mut row));
      total = 0;
    }
    row.push(*t);
    total += t.size;
  }
  if !row.is_empty() {
    rows.push(row);
  }
  rows
}

/// Each tile's place in a `width` wide area (rows of five units, tiles
/// stretched by their size), as sidebar.html's rectsFor.
pub(super) fn rects_for(list: &[Toggle], width: f32) -> Vec<(Tile, Rect)> {
  let mut out = Vec::new();
  for (ri, row) in rows_for(list).iter().enumerate() {
    let units: f32 = row.iter().map(|t| t.size as f32).sum();
    let free = width - SPACING * (row.len() as f32 - 1.0);
    let mut x = 0.0;
    for t in row {
      let w = free * t.size as f32 / units;
      out.push((t.tile, Rect::new(x, ri as f32 * (CELL_H + SPACING), w, CELL_H)));
      x += w + SPACING;
    }
  }
  out
}

/// Where a dragged tile would drop: the place whose centre is nearest the
/// pointer (`px`, `py` from the area's top left).
pub(super) fn drop_index(list: &[Toggle], item: Toggle, px: f32, py: f32, width: f32) -> usize {
  let rest: Vec<Toggle> = list.iter().filter(|t| t.tile != item.tile).copied().collect();
  let (mut best, mut best_d) = (0, f32::MAX);
  for i in 0..=rest.len() {
    let mut l = rest.clone();
    l.insert(i, item);
    if let Some((_, r)) = rects_for(&l, width).into_iter().find(|(t, _)| *t == item.tile) {
      let d = (r.x + r.w / 2.0 - px).powi(2) + (r.y + r.h / 2.0 - py).powi(2);
      if d < best_d {
        best_d = d;
        best = i;
      }
    }
  }
  best
}

fn rows_h(n: usize) -> f32 {
  if n == 0 {
    0.0
  } else {
    n as f32 * CELL_H + (n as f32 - 1.0) * SPACING
  }
}

/// What a tile shows and does (sidebar.html `defs`).
struct Def {
  name: &'static str,
  icon: &'static str,
  toggled: bool,
  status: String,
  /// a wide tile with a card keeps its background, only its icon is coloured
  alt: bool,
  hidden: bool,
  menu: bool,
}

fn s(v: &Value) -> &str {
  v.as_str().unwrap_or("")
}

impl Quick {
  pub fn new(toggles: Vec<Toggle>, cache: &Value) -> Self {
    let hw = Hw {
      radios: cache["radios"].clone(),
      eth: cache["eth"].clone(),
      bt: cache["bt"].clone(),
      awake: cache["awake"].as_bool().unwrap_or(false),
      mic: cache["mic"].as_bool(),
      night: cache["night"].clone(),
    };
    Quick { toggles, hw, ..Default::default() }
  }

  pub fn cache(&self) -> Value {
    json!({
      "radios": self.hw.radios, "eth": self.hw.eth, "bt": self.hw.bt,
      "awake": self.hw.awake, "mic": self.hw.mic, "night": self.hw.night,
    })
  }

  fn wifi_on(&self) -> bool {
    s(&self.hw.radios["wifi"]) == "On"
  }

  fn bt_on(&self) -> bool {
    s(&self.hw.radios["bluetooth"]) == "On"
  }

  fn bt_no_adapter(&self) -> bool {
    if self.hw.bt.is_object() {
      self.hw.bt["adapter"].as_bool() != Some(true)
    } else {
      self.hw.radios.is_object() && self.hw.radios["bluetooth"].is_null()
    }
  }

  fn visible(&self, m: &Model, tr: &dyn Fn(&str) -> String) -> Vec<Toggle> {
    self.toggles.iter().filter(|t| self.edit || !self.def(t.tile, m, tr).hidden).copied().collect()
  }

  fn unused(&self) -> Vec<Toggle> {
    AVAILABLE.iter().filter(|a| !self.toggles.iter().any(|t| t.tile == **a)).map(|a| Toggle { tile: *a, size: 1 }).collect()
  }

  /// What the rows show now: while dragging, a gap where the tile would land.
  fn shown_lists(&self, m: &Model, tr: &dyn Fn(&str) -> String) -> (Vec<(Toggle, bool)>, Vec<(Toggle, bool)>) {
    let visible = self.visible(m, tr);
    let unused = self.unused();
    match &self.drag {
      None => (visible.into_iter().map(|t| (t, false)).collect(), unused.into_iter().map(|t| (t, false)).collect()),
      Some(d) => {
        let mut used: Vec<(Toggle, bool)> = visible.into_iter().filter(|t| t.tile != d.tile.tile).map(|t| (t, false)).collect();
        let mut rest: Vec<(Toggle, bool)> = unused.into_iter().filter(|t| t.tile != d.tile.tile).map(|t| (t, false)).collect();
        if d.to_used {
          let i = d.index.min(used.len());
          used.insert(i, (d.tile, true));
        } else {
          rest.push((Toggle { tile: d.tile.tile, size: 1 }, true));
        }
        (used, rest)
      }
    }
  }

  /// The panel's height for its rows (sidebar.css `.quickpanel`).
  pub fn height(&self, m: &Model, tr: &dyn Fn(&str) -> String) -> f32 {
    let (used, unused) = self.shown_lists(m, tr);
    let ur: Vec<Toggle> = used.iter().map(|t| t.0).collect();
    let nr: Vec<Toggle> = unused.iter().map(|t| t.0).collect();
    let mut h = PADDING + rows_h(rows_for(&ur).len()) + SPACING;
    if self.edit {
      h += 13.0 + SPACING + rows_h(rows_for(&nr).len()) + SPACING;
    }
    h + 38.0 + PADDING
  }

  // ------------------------------------------------------------------ paint

  pub fn paint(&mut self, cx: &mut Cx, m: &Model, area: Rect, animations: bool, dash: Option<&ID2D1StrokeStyle>) -> anyhow::Result<()> {
    self.area = area;
    cx.round(area, 17.0, cx.t.layer1)?;
    let tr = cx.tr;
    let (used, unused) = self.shown_lists(m, &|s| tr(s));
    let inner_w = area.w - 2.0 * PADDING;
    let used_at = Rect::new(area.x + PADDING, area.y + PADDING, inner_w, rows_h(rows_for(&used.iter().map(|t| t.0).collect::<Vec<_>>()).len()));
    self.used_area = used_at;
    let mut targets: Vec<(Toggle, bool, Rect, bool)> = Vec::new();
    for ((tile, r), (t, ph)) in rects_for(&used.iter().map(|t| t.0).collect::<Vec<_>>(), inner_w).into_iter().zip(used.iter()) {
      debug_assert_eq!(tile, t.tile);
      targets.push((*t, *ph, Rect::new(used_at.x + r.x, used_at.y + r.y, r.w, r.h), true));
    }
    let mut y = used_at.bottom() + SPACING;
    if self.edit {
      cx.round(Rect::new(area.x + 28.0, y + 6.0, area.w - 56.0, 1.0), 0.5, cx.t.outline_variant)?;
      y += 13.0 + SPACING;
      let ul: Vec<Toggle> = unused.iter().map(|t| t.0).collect();
      let at = Rect::new(area.x + PADDING, y, inner_w, rows_h(rows_for(&ul).len()));
      self.unused_area = at;
      for ((_, r), (t, ph)) in rects_for(&ul, inner_w).into_iter().zip(unused.iter()) {
        targets.push((*t, *ph, Rect::new(at.x + r.x, at.y + r.y, r.w, r.h), false));
      }
      y = at.bottom() + SPACING;
    } else {
      self.unused_area = Rect::default();
    }
    // edit mode: a tile whose place or size changed slides there (FLIP)
    let now = cx.now;
    let mut shown = HashMap::new();
    for (t, ph, target, in_used) in &targets {
      let mut r = *target;
      if self.edit && animations {
        let moving = self.moving.get(&t.tile).copied();
        let last = self.shown.get(&t.tile).copied();
        match moving {
          Some((from, to, at)) if to == *target => {
            let k = (now.duration_since(at).as_secs_f32() * 1000.0 / SLIDE_MS).min(1.0);
            r = lerp_rect(from, to, MOVE.at(k));
            if k < 1.0 {
              cx.busy = true;
            }
          }
          _ => {
            if let Some(prev) = last.filter(|l| (l.x - target.x).abs() > 0.5 || (l.y - target.y).abs() > 0.5 || (l.w - target.w).abs() > 0.5) {
              self.moving.insert(t.tile, (prev, *target, now));
              cx.busy = true;
              r = prev;
            }
          }
        }
      }
      shown.insert(t.tile, r);
      if self.landing.is_some_and(|(l, _)| l == t.tile) {
        continue; // its ghost is landing there
      }
      self.paint_tile(cx, m, *t, r, *ph, *in_used, dash)?;
    }
    self.placed = shown.clone();
    if self.edit {
      self.shown = shown;
    } else {
      self.shown.clear();
      self.moving.clear();
    }
    // the edit button, bottom right
    let eb = Rect::new(area.right() - PADDING - 40.0, y - 2.0, 40.0, 40.0);
    let on = self.edit;
    let (bg, fg) = if on { (Some(cx.t.sec_container), cx.t.on_sec_container) } else { (None, cx.t.on_layer1) };
    cx.round_btn(eb, if on { "check" } else { "edit" }, 22.0, false, bg, fg, Hit::Quick(QHit::Edit))?;
    Ok(())
  }

  #[allow(clippy::too_many_arguments)]
  fn paint_tile(&self, cx: &mut Cx, m: &Model, t: Toggle, r: Rect, placeholder: bool, in_used: bool, dash: Option<&ID2D1StrokeStyle>) -> anyhow::Result<()> {
    let tr = cx.tr;
    let def = self.def(t.tile, m, &|s| tr(s));
    let hit = Hit::Quick(QHit::Tile(t.tile));
    if placeholder {
      // the dashed gap the dragged tile would fill
      dashed(cx, r.inset(1.0, 1.0), 26.0, cx.t.primary.alpha(0.6), 2.0, dash)?;
      return Ok(());
    }
    let expanded = r.w >= CELL_H * 2.0 + SPACING;
    let alt_look = def.alt && expanded;
    let hot = cx.hot(&hit) || cx.hot(&Hit::Quick(QHit::TileIcon(t.tile)));
    let pressed = cx.down(&hit) || cx.down(&Hit::Quick(QHit::TileIcon(t.tile)));
    let menu_open = self.menu == Some(t.tile);
    let radius = if pressed || menu_open { 17.0 } else if def.toggled { 23.0 } else { r.h / 2.0 };
    let a = if def.hidden { 0.45 } else { 1.0 };
    let filled = def.toggled && !alt_look;
    let bg = if filled {
      if hot { blend(cx.t.primary, Rgba(255, 255, 255, 1.0), 0.06) } else { cx.t.primary }
    } else if hot {
      cx.c.layer2_hover
    } else {
      cx.c.layer2
    };
    let fg = if filled { cx.t.on_primary } else { cx.t.on_layer1 };
    cx.round(r, radius, bg.alpha(bg.3 * a))?;
    if self.edit {
      dashed(cx, r.inset(3.0, 3.0), (radius - 3.0).max(4.0), cx.c.outline.alpha(0.8 * a), 1.0, dash)?;
    }
    if !expanded {
      cx.icon(def.icon, r.x + r.w / 2.0, r.y + r.h / 2.0, 24.0, def.toggled, fg.alpha(a))?;
    } else {
      let ic = Rect::new(r.x + 6.0, r.y + 6.0, 44.0, 44.0);
      let (ibg, ifg) = if def.toggled && alt_look {
        (Some(cx.t.primary), cx.t.on_primary)
      } else if filled {
        (None, fg)
      } else {
        (Some(cx.c.layer3), fg)
      };
      if let Some(c) = ibg {
        cx.round(ic, if def.toggled { 17.0 } else { 22.0 }, c.alpha(c.3 * a))?;
      }
      cx.icon(def.icon, ic.x + 22.0, ic.y + 22.0, 22.0, def.toggled, ifg.alpha(a))?;
      let tx = ic.right() + 4.0;
      let arrow = def.menu;
      let tw = r.right() - tx - if arrow { 30.0 } else { 8.0 };
      let name_h = if def.status.is_empty() { 0.0 } else { 16.5 };
      let top = r.y + (r.h - 17.0 - name_h) / 2.0;
      cx.text(&cx.tr(def.name), Rect::new(tx, top, tw, 17.0), stw(13.0, 600.0), fg.alpha(a))?;
      if !def.status.is_empty() {
        cx.text(&def.status, Rect::new(tx, top + 17.0, tw, 16.0), stw(12.0, 300.0), fg.alpha(a))?;
      }
      if arrow {
        cx.icon(if menu_open { "expand_less" } else { "expand_more" }, r.right() - 8.0 - 9.0, r.y + r.h / 2.0, 18.0, false, fg.alpha(0.7 * a))?;
      }
    }
    cx.hit(r, hit);
    if expanded && def.menu && !self.edit && in_used {
      cx.hit(Rect::new(r.x + 6.0, r.y + 6.0, 44.0, 44.0), Hit::Quick(QHit::TileIcon(t.tile)));
    }
    Ok(())
  }

  /// The lifted tile at `r` (its own surface): a little bigger, with a shadow.
  pub fn paint_ghost(&self, cx: &mut Cx, m: &Model, r: Rect) -> anyhow::Result<()> {
    let Some(d) = &self.drag else { return Ok(()) };
    cx.shadow(r, 23.0, 0.8)?;
    // drawn as it looks outside the edit mode
    let me = Quick { edit: false, toggles: Vec::new(), hw: self.hw.clone(), ..Default::default() };
    me.paint_tile(cx, m, d.tile, r, false, false, None)?;
    Ok(())
  }

  // ------------------------------------------------------------------ cards

  /// Starts / ends the card under a tile (`top` in panel DIPs).
  pub fn set_menu(&mut self, tile: Option<Tile>, top: f32) {
    if tile == self.menu {
      return;
    }
    self.menu = tile;
    match tile {
      Some(t) => {
        self.card = Some(CardAnim { tile: t, top, at: Instant::now(), closing: false });
        self.wifi_err.clear();
        self.wifi_ask = None;
        self.night_level = None;
      }
      None => {
        if let Some(c) = self.card.as_mut() {
          if !c.closing {
            c.closing = true;
            c.at = Instant::now();
          }
        }
      }
    }
  }

  pub fn tile_rect(&self, tile: Tile) -> Option<Rect> {
    self.placed.get(&tile).copied()
  }

  pub fn tile_expanded(&self, tile: Tile) -> bool {
    self.tile_rect(tile).is_some_and(|r| r.w >= CELL_H * 2.0 + SPACING)
  }

  // ------------------------------------------------------------------ input

  /// A press on a tile in the edit mode may become a drag.
  pub fn press(&mut self, m: &Model, tr: &dyn Fn(&str) -> String, tile: Tile, x: f32, y: f32) {
    if !self.edit {
      return;
    }
    let size = self.toggles.iter().find(|t| t.tile == tile).map_or(1, |t| t.size);
    let r = self.shown.get(&tile).copied().unwrap_or(Rect::new(x - 20.0, y - 20.0, 40.0, CELL_H));
    let visible = self.visible(m, tr);
    let orig = match visible.iter().position(|t| t.tile == tile) {
      Some(i) => (true, i),
      None => (false, 0),
    };
    self.press = Some(Press { tile: Toggle { tile, size }, x0: x, y0: y, x, y, dx: x - r.x, dy: y - r.y, w: r.w, h: r.h, at: Instant::now(), orig });
  }

  /// A press held long enough without moving: where to lift the tile.
  pub fn lift_due(&self) -> Option<(f32, f32)> {
    let p = self.press.as_ref()?;
    (self.drag.is_none() && p.at.elapsed().as_millis() >= LIFT_MS).then_some((p.x, p.y))
  }

  pub fn pressing(&self) -> bool {
    self.press.is_some() && self.drag.is_none()
  }

  /// Where the tile would drop with the pointer at (x, y): outside the
  /// tiles it goes back where it was.
  fn target(&self, m: &Model, tr: &dyn Fn(&str) -> String, tile: Toggle, x: f32, y: f32, orig: (bool, usize)) -> (bool, usize) {
    let near = Rect::new(self.area.x - 24.0, self.area.y - 24.0, self.area.w + 48.0, self.area.h + 48.0);
    if !near.contains(x, y) {
      return orig;
    }
    if self.unused_area.h > 0.0 && y > self.unused_area.y - SPACING {
      return (false, 0);
    }
    let visible = self.visible(m, tr);
    (true, drop_index(&visible, tile, x - self.used_area.x, y - self.used_area.y, self.used_area.w))
  }

  /// The pointer moved with the button down (or a long press is due).
  pub fn drag_move(&mut self, m: &Model, tr: &dyn Fn(&str) -> String, x: f32, y: f32, lift: bool) -> Moved {
    let Some(p) = self.press.as_mut() else { return Moved::Nothing };
    p.x = x;
    p.y = y;
    if self.drag.is_none() {
      if !lift && (x - p.x0).hypot(y - p.y0) < 5.0 {
        return Moved::Nothing;
      }
      let (tile, dx, dy, w, h, orig) = (p.tile, p.dx, p.dy, p.w, p.h, p.orig);
      let (to_used, index) = self.target(m, tr, tile, x, y, orig);
      self.drag = Some(Drag { tile, x, y, dx, dy, w, h, to_used, index, orig });
      self.dragged = true;
      return Moved::Lifted;
    }
    let (tile, orig) = {
      let d = self.drag.as_ref().unwrap();
      (d.tile, d.orig)
    };
    let (to_used, index) = self.target(m, tr, tile, x, y, orig);
    let d = self.drag.as_mut().unwrap();
    d.x = x;
    d.y = y;
    if (d.to_used, d.index) != (to_used, index) {
      d.to_used = to_used;
      d.index = index;
      Moved::Retargeted
    } else {
      Moved::Moved
    }
  }

  /// The button went up (`drop`) or the drag was cancelled: the tile that
  /// lands, and whether the layout changed.
  pub fn release(&mut self, m: &Model, tr: &dyn Fn(&str) -> String, drop: bool) -> Option<(Tile, bool)> {
    self.press = None;
    let d = self.drag.take()?;
    self.landing = Some((d.tile.tile, Instant::now()));
    if !drop || (d.to_used, d.index) == d.orig {
      return Some((d.tile.tile, false));
    }
    let rest: Vec<Toggle> = self.toggles.iter().filter(|t| t.tile != d.tile.tile).copied().collect();
    if !d.to_used {
      self.toggles = rest;
    } else {
      let visible: Vec<Toggle> = self.visible(m, tr).into_iter().filter(|t| t.tile != d.tile.tile).collect();
      let at = visible.get(d.index).and_then(|b| rest.iter().position(|t| t.tile == b.tile)).unwrap_or(rest.len());
      let mut n = rest;
      n.insert(at, d.tile);
      self.toggles = n;
    }
    Some((d.tile.tile, true))
  }

  /// The click after a press: false when a drag just ended (no add / remove).
  pub fn take_click(&mut self) -> bool {
    !std::mem::take(&mut self.dragged)
  }

  /// Edit mode: add (size 1, at the end) or remove a tile.
  pub fn edit_click(&mut self, tile: Tile) {
    if self.toggles.iter().any(|t| t.tile == tile) {
      self.toggles.retain(|t| t.tile != tile);
    } else {
      self.toggles.push(Toggle { tile, size: 1 });
    }
  }

  pub fn edit_size(&mut self, tile: Tile) {
    for t in self.toggles.iter_mut().filter(|t| t.tile == tile) {
      t.size = 3 - t.size;
    }
  }

  pub fn edit_move(&mut self, tile: Tile, off: i32) {
    let Some(i) = self.toggles.iter().position(|t| t.tile == tile) else { return };
    let j = i as i32 + off;
    if j < 0 || j as usize >= self.toggles.len() {
      return;
    }
    self.toggles.swap(i, j as usize);
  }

  pub fn has_card(&self) -> bool {
    self.card.as_ref().is_some_and(|c| !c.closing)
  }
}

fn audio_devices(m: &Model, out: bool) -> Vec<crate::providers::audio::AudioDevice> {
  m.audio
    .as_ref()
    .map(|a| if out { a.playback_devices.clone() } else { a.recording_devices.clone() })
    .unwrap_or_default()
}

/// A dashed rounded outline (`outline: 1px dashed`).
fn dashed(cx: &mut Cx, r: Rect, radius: f32, c: Rgba, width: f32, dash: Option<&ID2D1StrokeStyle>) -> anyhow::Result<()> {
  let b = cx.p.brush(c)?;
  unsafe {
    cx.p.dc.DrawRoundedRectangle(&r.rounded(radius.min(r.h / 2.0)), &b, width, dash);
  }
  Ok(())
}

// ---------------------------------------------------------------- workers

fn qs(path: &str) -> Option<Value> {
  match core_api::post(&format!("/qs/{path}")) {
    Some((200, body)) => serde_json::from_slice(&body).ok(),
    Some((204, _)) => Some(Value::Null),
    _ => None,
  }
}

fn core_json(args: &[&str]) -> Option<Value> {
  core_api::run_core_output(args).and_then(|s| serde_json::from_str(&s).ok())
}

/// Wi-Fi from the core (the Native Wifi API): `list` scans and lists,
/// `connect` (with a password for a new network) and `disconnect`.
pub(super) fn wifi(action: &str, ssid: &str, password: Option<&str>) -> Option<Value> {
  match action {
    "connect" => {
      let mut path = format!("wifi-connect?ssid={}", query(ssid));
      if let Some(p) = password {
        path.push_str(&format!("&pw={}", query(p)));
      }
      qs(&path)
    }
    "disconnect" => qs("wifi-disconnect"),
    _ => qs("wifi"),
  }
}

/// Percent-encodes a query value (UTF-8, unreserved characters kept).
fn query(s: &str) -> String {
  s.bytes()
    .map(|b| match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
      _ => format!("%{b:02X}"),
    })
    .collect()
}

fn ev(e: QEv) {
  send(Msg::Sidebar(Ev::Quick(e)));
}

fn spawn(f: impl FnOnce() + Send + 'static) {
  std::thread::spawn(f);
}


/// "7:5" -> "07:05"; None when it is no time of day.
pub(super) fn valid_time(s: &str) -> Option<String> {
  let (h, m) = s.trim().split_once(':')?;
  let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
  (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn t(tile: Tile, size: u8) -> Toggle {
    Toggle { tile, size }
  }

  #[test]
  fn every_switch_is_read_back_and_only_switches_are() {
    // every hardware state on: a tile has a switch exactly when it has a source
    let all_on = json!({
      "radios": { "wifi": "On", "bluetooth": "On" }, "eth": { "state": "up" },
      "awake": true, "mic": true, "night": { "on": true },
    });
    let q = Quick::new(Vec::new(), &all_on);
    for tile in AVAILABLE {
      assert_eq!(Source::of(tile).is_some(), q.switched(tile).is_some(), "{:?}", tile);
    }
  }

  #[test]
  fn a_switch_is_judged_by_what_its_source_shows() {
    let seen = [Seen::NoAnswer, Seen::Read(None), Seen::Read(Some(false)), Seen::Read(Some(true))];
    for want in [false, true] {
      for s in seen {
        for left in [Duration::ZERO, VERIFY_EVERY] {
          let v = verdict(want, s, left);
          let shown = s == Seen::Read(Some(want));
          let contradicted = matches!(s, Seen::NoAnswer | Seen::Read(Some(_))) && !shown;
          let expected = if shown {
            Verdict::Done
          } else if !left.is_zero() {
            Verdict::Again
          } else if contradicted {
            Verdict::Failed
          } else {
            Verdict::Done
          };
          assert_eq!(v, expected, "want {want}, seen {s:?}, left {left:?}");
        }
      }
    }
  }

  #[test]
  fn rows_hold_five_units() {
    let list = vec![t(Tile::Wifi, 2), t(Tile::Ethernet, 2), t(Tile::Mic, 1), t(Tile::Audio, 2), t(Tile::DarkMode, 1)];
    let rows = rows_for(&list);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].len(), 3);
    assert_eq!(rows[1].len(), 2);
  }

  #[test]
  fn tiles_stretch_with_their_size() {
    let list = vec![t(Tile::Wifi, 2), t(Tile::Mic, 1)];
    let r = rects_for(&list, 306.0);
    // 300 free DIP for 3 units
    assert_eq!(r[0].1.w, 200.0);
    assert_eq!(r[1].1.x, 206.0);
    assert_eq!(r[1].1.w, 100.0);
  }

  #[test]
  fn a_compact_tile_stretched_across_a_row_keeps_its_label_and_menu() {
    let saved: super::super::store::Store = serde_json::from_str(r#"{"quickToggles":[{"type":"nightLight","size":1}]}"#).unwrap();
    let mut q = Quick::new(saved.toggles(), &Value::Null);
    q.placed = rects_for(&q.toggles, 418.0).into_iter().collect();
    assert!(q.tile_expanded(Tile::NightLight));
    let r = q.tile_rect(Tile::NightLight).unwrap();
    assert!(r.contains(r.x + 28.0, r.y + 28.0));
    assert!(r.right() - (r.x + 54.0) - 30.0 > 0.0);
    q.placed.insert(Tile::NightLight, Rect::new(0.0, 0.0, 81.0, CELL_H));
    assert!(!q.tile_expanded(Tile::NightLight));
    q.edit_size(Tile::NightLight);
    assert_eq!(q.toggles[0].size, 2);
    q.placed = rects_for(&q.toggles, 418.0).into_iter().collect();
    assert!(q.tile_expanded(Tile::NightLight));
  }

  #[test]
  fn a_tile_drops_where_its_centre_is_nearest() {
    let list = vec![t(Tile::Wifi, 1), t(Tile::Mic, 1), t(Tile::Audio, 1)];
    let item = t(Tile::Audio, 1);
    // far left of the first row: first
    assert_eq!(drop_index(&list, item, 5.0, 20.0, 300.0), 0);
    // second row: last
    assert_eq!(drop_index(&list, item, 280.0, 20.0, 300.0), 2);
  }

  #[test]
  fn times_are_checked_and_padded() {
    assert_eq!(valid_time("7:5"), Some("07:05".into()));
    assert_eq!(valid_time("24:00"), None);
    assert_eq!(valid_time("ab"), None);
  }
}
