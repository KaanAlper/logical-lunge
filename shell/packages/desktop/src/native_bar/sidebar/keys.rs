//! The shortcut editor page. One model from the core (`--keybinds-model`):
//! the core's shortcuts, the apps the user added, the window manager's
//! (config.yaml) and the combos Windows keeps. Edits are staged: every
//! change asks the core for the conflicts of the whole staged state
//! (`--keybinds-check`) and the rows in a conflict turn red; "Kaydet" writes
//! it all (`--keybinds-save`) or shakes and explains when something still
//! clashes. A row is changed by pressing it and then the new keys (the core
//! catches them: `--capture`); Esc cancels.

use std::{
  collections::{BTreeMap, BTreeSet},
  time::{Duration, Instant},
};

use serde_json::{json, Value};

use super::{
  super::{
    core_api,
    dialog::{Answer, Kind, Spec},
    gfx::{Rect, Rgba},
    send, Msg, Ui,
  },
  kit::{blend, st, stw, Cx},
  text::{TextField, Typed},
  Ev, FieldId, Hit, ScrollId,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum KHit {
  Lock,
  Reset,
  Row(String),
  Undo(String),
  Clear(String),
  /// an app row's remove button
  Remove(String),
  /// "+" under the apps
  Add,
  Save,
  Discard,
  /// the dim around the app picker
  Dismiss,
  /// an app in the picker (index in the filtered list)
  Pick(usize),
  Browse,
}

pub(in crate::native_bar) enum KEv {
  Loaded(Option<Value>),
  Captured(String, String),
  Checked(u64, Vec<Conflict>),
  Saved(bool, Vec<Conflict>, String),
  Reset(bool, String),
  Picked(Option<(String, String)>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::native_bar) struct Conflict {
  combo: String,
  keys: Vec<String>,
  /// Windows keeps this combo (its label)
  reserved: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Src {
  Fixed,
  Core,
  Tiling,
}

#[derive(Clone)]
struct Row {
  key: String,
  group: &'static str,
  label: String,
  combo: String,
  default: String,
  extra: Vec<String>,
  src: Src,
  /// an app shortcut: it can be removed
  app: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CustomApp {
  id: String,
  name: String,
  path: String,
  combo: String,
}

/// What the editor shows: the core's model with the unsaved changes on top.
#[derive(Default)]
struct Staged {
  core: BTreeMap<String, String>,
  apps: Vec<CustomApp>,
  removed: BTreeSet<String>,
  tiling: BTreeMap<i64, Vec<String>>,
}

#[derive(Default)]
pub(super) struct Keys {
  model: Value,
  staged: Staged,
  /// the saved apps and removals (the staged ones differ: unsaved)
  saved_apps: Vec<CustomApp>,
  saved_removed: BTreeSet<String>,
  conflicts: Vec<Conflict>,
  check_seq: u64,
  pub locked: bool,
  pub capturing: Option<String>,
  msg: Option<(bool, String)>,
  query: String,
  /// the app picker is open (apps of the Super menu: name, launch path)
  picker: bool,
  apps: Vec<(String, String)>,
  shake: Option<Instant>,
  animations: bool,
  busy: bool,
}

const GROUPS: [(&str, &str, &str); 5] = [("win", "Pencereler", "select_window"), ("ws", "Workspace", "view_carousel"), ("mon", "Monitörler", "monitor"), ("app", "Uygulamalar", "apps"), ("sys", "Sistem", "settings")];
const SHAKE: Duration = Duration::from_millis(420);
const FOOTER_H: f32 = 56.0;

fn ll_label(id: &str) -> String {
  let fixed = match id {
    "focus-left" => "Sola odaklan",
    "focus-right" => "Sağa odaklan",
    "focus-up" => "Yukarı odaklan",
    "focus-down" => "Aşağı odaklan",
    "move-left" => "Pencereyi sola taşı",
    "move-right" => "Pencereyi sağa taşı",
    "move-up" => "Pencereyi yukarı taşı",
    "move-down" => "Pencereyi aşağı taşı",
    "ws-prev" => "Önceki workspace",
    "ws-next" => "Sonraki workspace",
    "ws-move-prev" => "Pencereyi önceki workspace'e taşı",
    "ws-move-next" => "Pencereyi sonraki workspace'e taşı",
    "terminal" => "Terminal",
    "terminal-alt" => "Terminal (ikinci kısayol)",
    "browser" => "Tarayıcı",
    "files" => "Dosya yöneticisi",
    "code" => "Kod editörü",
    "editor" => "Metin editörü",
    "close" => "Pencereyi kapat",
    "screenshot" => "Ekran alıntısı",
    "screenshot-alt" => "Ekran alıntısı (ikinci kısayol)",
    "screenshot-screen" => "Monitörün tamamı (panoya)",
    "clipboard" => "Pano geçmişi",
    "file-search" => "Dosya araması",
    "overview-alt" => "Super menüsü",
    "run" => "Çalıştır",
    "search" => "Ara (Super menüsü)",
    "workspaces" => "Workspace'ler (Super menüsü)",
    "settings" => "Ayarlar",
    "sidebar" => "Sağ panel",
    "notifications" => "Bildirimler",
    "task-manager" => "Görev Yöneticisi",
    "focus-urgent-or-last" => "Dikkat isteyen pencereye git",
    _ => "",
  };
  if !fixed.is_empty() {
    fixed.to_string()
  } else if let Some(n) = id.strip_prefix("ws-") {
    format!("Workspace {n}")
  } else {
    id.to_string()
  }
}

fn ll_group(id: &str, app: bool) -> &'static str {
  if app {
    "app"
  } else if id.starts_with("focus-") || id.starts_with("move-") || id == "close" {
    "win"
  } else if id.starts_with("ws-") || id == "workspaces" {
    "ws"
  } else {
    "sys"
  }
}

fn tiling_label(cmds: &[String]) -> String {
  let c = cmds.join(" ; ");
  if let Some(n) = c.strip_prefix("move --workspace ").filter(|n| n.chars().all(|ch| ch.is_ascii_digit()) && !n.is_empty()) {
    return format!("Pencereyi workspace {n}'e gönder");
  }
  let known = match c.as_str() {
    "focus --prev-workspace" => "Önceki workspace (PageUp)",
    "focus --next-workspace" => "Sonraki workspace (PageDown)",
    "focus --prev-active-workspace" => "Önceki dolu workspace",
    "focus --next-active-workspace" => "Sonraki dolu workspace",
    "move --prev-workspace ; focus --prev-workspace" => "Pencereyi önceki workspace'e taşı (PageUp)",
    "move --next-workspace ; focus --next-workspace" => "Pencereyi sonraki workspace'e taşı (PageDown)",
    "toggle-fullscreen" => "Tam ekran",
    "toggle-fullscreen-spoof" => "Yuvasında tam ekran (spoof)",
    "toggle-special-workspace" => "Gizli workspace’i aç / kapat",
    "move-to-special-workspace" => "Pencereyi gizli workspace’e gönder",
    "toggle-pin" => "Pencereyi sabitle (her workspace’te)",
    "toggle-floating --centered" => "Yüzen pencere",
    "toggle-tiling-direction" => "Bölme yönünü değiştir",
    "toggle-minimized" => "Simge durumuna küçült",
    "wm-cycle-focus" => "Yüzen / döşeli pencereler arasında geç",
    "resize --width -10%" => "Pencereyi daralt",
    "resize --width +10%" => "Pencereyi genişlet",
    "split-ratio -0.1" => "Bölme oranını azalt",
    "split-ratio 0.1" => "Bölme oranını artır",
    "focus --prev-active-workspace-on-monitor" => "Bu monitörde önceki dolu workspace",
    "focus --next-active-workspace-on-monitor" => "Bu monitörde sonraki dolu workspace",
    "focus --workspace-in-direction left" => "Soldaki monitöre geç",
    "focus --workspace-in-direction right" => "Sağdaki monitöre geç",
    "move --workspace-in-direction left ; focus --workspace-in-direction left" => "Pencereyi soldaki monitöre gönder",
    "move --workspace-in-direction right ; focus --workspace-in-direction right" => "Pencereyi sağdaki monitöre gönder",
    "move-workspace --direction left" => "Workspace'i soldaki monitöre taşı",
    "move-workspace --direction right" => "Workspace'i sağdaki monitöre taşı",
    "wm-enable-binding-mode --name resize" => "Boyutlandırma modu",
    "shell-exec ms-settings:" => "Windows ayarları",
    "shell-exec sndvol" => "Ses karıştırıcı",
    "wm-reload-config" => "Pencere yöneticisi ayarlarını yenile",
    "wm-redraw" => "Pencereleri yeniden çiz",
    "wm-toggle-pause" => "Döşemeyi duraklat",
    "wm-exit" => "Logical Lunge'dan çık",
    _ => "",
  };
  if !known.is_empty() {
    known.to_string()
  } else if c.to_lowercase().contains("wezterm") {
    "Terminal (yedek)".into()
  } else {
    c
  }
}

fn tiling_group(cmds: &[String]) -> &'static str {
  let c = cmds.join(" ");
  if c.contains("workspace-in-direction") || c.starts_with("move-workspace") {
    "mon"
  } else if c.contains("workspace") {
    "ws"
  } else if c.contains("shell-exec") {
    "app"
  } else if ["wm-reload", "wm-redraw", "wm-toggle-pause", "wm-exit"].iter().any(|p| c.starts_with(p)) {
    "sys"
  } else {
    "win"
  }
}

fn glyph(k: &str) -> &str {
  match k {
    "Left" => "←",
    "Right" => "→",
    "Up" => "↑",
    "Down" => "↓",
    "Enter" => "↵",
    "PageUp" => "PgUp",
    "PageDown" => "PgDn",
    "Escape" => "Esc",
    other => other,
  }
}

fn strs(v: &Value) -> Vec<String> {
  v.as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn conflicts_of(v: &Value) -> Vec<Conflict> {
  v.as_array()
    .map(|a| {
      a.iter()
        .map(|c| Conflict {
          combo: c["combo"].as_str().unwrap_or("").to_string(),
          keys: strs(&c["keys"]),
          reserved: c["reserved"].as_str().map(str::to_string),
        })
        .collect()
    })
    .unwrap_or_default()
}

fn custom_apps(model: &Value) -> Vec<CustomApp> {
  model["core"]
    .as_array()
    .into_iter()
    .flatten()
    .filter(|b| b["custom"].as_bool() == Some(true))
    .map(|b| CustomApp {
      id: b["id"].as_str().unwrap_or("").to_string(),
      name: b["name"].as_str().unwrap_or("").to_string(),
      path: b["path"].as_str().unwrap_or("").to_string(),
      combo: b["combo"].as_str().unwrap_or("").to_string(),
    })
    .collect()
}

fn removed_apps(model: &Value) -> BTreeSet<String> {
  model["core"]
    .as_array()
    .into_iter()
    .flatten()
    .filter(|b| b["removed"].as_bool() == Some(true))
    .filter_map(|b| b["id"].as_str().map(str::to_string))
    .collect()
}

/// A new app shortcut's id: `app:` and the time in base 36 (unique per user).
fn new_app_id() -> String {
  let mut n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
  let mut s = String::new();
  while n > 0 {
    s.insert(0, std::char::from_digit((n % 36) as u32, 36).unwrap_or('0'));
    n /= 36;
  }
  format!("app:{s}")
}

impl Keys {
  fn rows(&self, tr: &dyn Fn(&str) -> String) -> Vec<Row> {
    let mut rows = vec![
      Row { key: "super".into(), group: "sys", label: tr("Arama / overview"), combo: "Super".into(), default: String::new(), extra: vec![], src: Src::Fixed, app: false },
      Row { key: "dock".into(), group: "sys", label: tr("Uygulama Dock’u"), combo: "Super+Alt".into(), default: String::new(), extra: vec![], src: Src::Fixed, app: false },
      // Hyprland's mouse binds (ii), in the input hook
      Row { key: "mouse-move".into(), group: "win", label: tr("Pencereyi taşı (sürükle)"), combo: "Super+Mouse L".into(), default: String::new(), extra: vec![], src: Src::Fixed, app: false },
      Row { key: "mouse-resize".into(), group: "win", label: tr("Pencereyi boyutlandır (sürükle)"), combo: "Super+Mouse R".into(), default: String::new(), extra: vec![], src: Src::Fixed, app: false },
      Row { key: "mouse-wheel".into(), group: "ws", label: tr("Workspace değiştir (tekerlek)"), combo: "Super+Wheel".into(), default: String::new(), extra: vec![], src: Src::Fixed, app: false },
      Row { key: "mouse-back".into(), group: "ws", label: tr("Gizli workspace’i aç / kapat"), combo: "Super+Mouse 4".into(), default: String::new(), extra: vec![], src: Src::Fixed, app: false },
    ];
    for b in self.model["core"].as_array().into_iter().flatten() {
      if b["custom"].as_bool() == Some(true) {
        continue;
      }
      let id = b["id"].as_str().unwrap_or("").to_string();
      if self.staged.removed.contains(&id) {
        continue;
      }
      let app = b["app"].as_bool() == Some(true);
      let combo = self.staged.core.get(&id).cloned().unwrap_or_else(|| b["combo"].as_str().unwrap_or("").to_string());
      rows.push(Row {
        key: format!("ll:{id}"),
        group: ll_group(&id, app),
        label: tr(&ll_label(&id)),
        combo,
        default: b["default"].as_str().unwrap_or("").to_string(),
        extra: vec![],
        src: Src::Core,
        app,
      });
    }
    for a in &self.staged.apps {
      rows.push(Row { key: format!("ll:{}", a.id), group: "app", label: a.name.clone(), combo: a.combo.clone(), default: String::new(), extra: vec![], src: Src::Core, app: true });
    }
    for g in self.model["tiling"].as_array().into_iter().flatten() {
      let cmds = strs(&g["commands"]);
      let index = g["index"].as_i64().unwrap_or(-1);
      let binds = self.staged.tiling.get(&index).cloned().unwrap_or_else(|| strs(&g["bindings"]));
      rows.push(Row {
        key: format!("tiling:{index}"),
        group: tiling_group(&cmds),
        label: tr(&tiling_label(&cmds)),
        combo: binds.first().cloned().unwrap_or_default(),
        default: String::new(),
        extra: binds.iter().skip(1).cloned().collect(),
        src: Src::Tiling,
        app: false,
      });
    }
    rows
  }

  /// Unsaved changes?
  fn dirty(&self) -> bool {
    !self.staged.core.is_empty() || !self.staged.tiling.is_empty() || self.staged.apps != self.saved_apps || self.staged.removed != self.saved_removed
  }

  /// The staged state as the core reads it.
  fn staged_json(&self) -> String {
    let tiling: serde_json::Map<String, Value> = self.staged.tiling.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    json!({
      "core": self.staged.core,
      "apps": self.staged.apps.iter().map(|a| json!({ "id": a.id, "name": a.name, "path": a.path, "combo": a.combo })).collect::<Vec<_>>(),
      "removed": self.staged.removed,
      "tiling": tiling,
    })
    .to_string()
  }

  fn conflict_for(&self, key: &str) -> Option<&Conflict> {
    self.conflicts.iter().find(|c| c.keys.iter().any(|k| k == key))
  }

  fn shaking(&self, now: Instant) -> Option<f32> {
    self.shake.map(|s| now.saturating_duration_since(s).as_secs_f32() / SHAKE.as_secs_f32()).filter(|t| *t < 1.0)
  }

  /// The footer (save, discard) takes this much of the page's bottom.
  pub(super) fn footer_h(&self) -> f32 {
    if self.locked {
      0.0
    } else {
      FOOTER_H
    }
  }
}

/// The lock chip of the page's head.
pub(super) fn paint_actions(cx: &mut Cx, k: &Keys, right: f32, y: f32) -> anyhow::Result<()> {
  let label = cx.tr(if k.locked { "Salt okunur" } else { "Düzenleme" });
  let icon = if k.locked { "lock" } else { "lock_open" };
  let w = cx.chip_w(&label, Some(icon))?;
  cx.chip(right - w, y, 32.0, &label, Some(icon), !k.locked, true, Hit::Keys(KHit::Lock))?;
  Ok(())
}

/// Keycaps right-aligned ending at `right`; returns their width.
fn keycaps(cx: &mut Cx, combo: &str, right: f32, cy: f32, paint: bool) -> anyhow::Result<f32> {
  if combo.is_empty() {
    let w = cx.measure("—", st(13.0))?;
    if paint {
      cx.text("—", Rect::new(right - w - 1.0, cy - 10.0, w + 2.0, 20.0), st(13.0), cx.c.outline)?;
    }
    return Ok(w);
  }
  let parts: Vec<&str> = combo.split('+').collect();
  let mut widths = Vec::new();
  for p in &parts {
    let w = if *p == "Super" { 11.0 + 3.0 + cx.measure("Super", stw(12.0, 550.0))? } else { cx.measure(glyph(p), stw(12.0, 550.0))? };
    widths.push((w + 14.0).max(24.0).ceil());
  }
  let total = widths.iter().sum::<f32>() + 3.0 * (widths.len() as f32 - 1.0);
  if paint {
    let mut x = right - total;
    for (p, w) in parts.iter().zip(widths) {
      let r = Rect::new(x, cy - 12.0, w, 24.0);
      let sup = *p == "Super";
      let (bg, fg) = if sup { (cx.t.sec_container, cx.t.on_sec_container) } else { (cx.c.layer3, cx.t.on_layer1) };
      // `box-shadow: inset 0 -2px 0 rgba(0 0 0 / 30%)`
      cx.round(r, 7.0, Rgba(0, 0, 0, 0.3))?;
      cx.round(Rect::new(r.x, r.y, r.w, r.h - 2.0), 7.0, bg)?;
      if sup {
        cx.windows_logo(r.x + 7.0, r.y + 6.0, 11.0, fg)?;
        cx.text("Super", Rect::new(r.x + 7.0 + 14.0, r.y, r.w - 21.0, 22.0), stw(12.0, 550.0), fg)?;
      } else {
        cx.text_center(glyph(p), Rect::new(r.x, r.y, r.w, 22.0), stw(12.0, 550.0), fg)?;
      }
      x += w + 3.0;
    }
  }
  Ok(total)
}

/// What a conflicting row says under its label.
fn conflict_text(cx: &Cx, rows: &[Row], row: &Row, c: &Conflict) -> String {
  if let Some(r) = &c.reserved {
    return format!("{}: {}", cx.tr("Windows'a ayrılmış"), cx.tr(r));
  }
  let others: Vec<String> = c.keys.iter().filter(|k| **k != row.key).filter_map(|k| rows.iter().find(|r| &r.key == k).map(|r| r.label.clone())).collect();
  format!("{} {}", cx.tr("Çakışıyor:"), others.join(", "))
}

/// Draws the body from `r.y`; returns its height.
pub(super) fn paint(cx: &mut Cx, k: &mut Keys, search: &mut TextField, r: Rect) -> anyhow::Result<f32> {
  let mut y = r.y;
  // tools: search, reset
  let reset = cx.tr("Sıfırla");
  let rw = cx.chip_w(&reset, Some("restart_alt"))?;
  let sr = Rect::new(r.x, y, r.w - rw - 8.0, 38.0);
  cx.round(sr, 19.0, cx.t.layer1)?;
  cx.icon("search", sr.x + 12.0 + 9.0, sr.y + 19.0, 18.0, false, cx.t.on_surface_variant)?;
  let focused = cx.focus == Some(FieldId::KeysSearch);
  let ph = cx.tr("Kısayol ara");
  let inner = Rect::new(sr.x + 12.0 + 18.0 + 6.0, sr.y, sr.w - 48.0, sr.h);
  let color = cx.t.on_layer1;
  search.paint(cx.p, &cx.t, inner, st(14.0), color, &ph, focused)?;
  cx.hit(sr, Hit::Field(FieldId::KeysSearch));
  cx.chip(r.right() - rw, y + 3.0, 32.0, &reset, Some("restart_alt"), false, !k.locked, Hit::Keys(KHit::Reset))?;
  y += 38.0 + 10.0;
  if !k.locked {
    y += cx.hint(r.x, y, r.w, &cx.tr("Değiştirmek için bir satıra tıkla ve yeni tuşlara bas. Esc: iptal."))? + 10.0;
  }
  if let Some((ok, text)) = &k.msg {
    y += cx.message(r.x, y, r.w, *ok, text)? + 10.0;
  }
  k.query = search.text().to_lowercase();
  let tr = cx.tr;
  let rows = k.rows(&|s| tr(s));
  let q = k.query.clone();
  let shown: Vec<Row> = rows
    .iter()
    .filter(|row| q.is_empty() || row.label.to_lowercase().contains(&q) || row.combo.to_lowercase().contains(&q))
    .cloned()
    .collect();
  for (g, title, icon) in GROUPS {
    let list: Vec<&Row> = shown.iter().filter(|row| row.group == g).collect();
    let add = g == "app" && !k.locked;
    if list.is_empty() && !add {
      continue;
    }
    // a conflict adds a line under its row
    let heights: Vec<f32> = list.iter().map(|row| if k.conflict_for(&row.key).is_some() { 56.0 } else { 38.0 }).collect();
    let gh = 6.0 + 28.0 + heights.iter().map(|h| h + 2.0).sum::<f32>() + if add { 40.0 } else { 0.0 } + 6.0;
    let gr = Rect::new(r.x, y, r.w, gh);
    cx.round(gr, 17.0, cx.t.layer1)?;
    cx.section_title(gr.x + 6.0, gr.y + 6.0, gr.w - 12.0, icon, &cx.tr(title))?;
    let mut ry = gr.y + 6.0 + 28.0;
    for (row, rh) in list.into_iter().zip(heights) {
      let rr = Rect::new(gr.x + 6.0, ry, gr.w - 12.0, rh);
      let line = Rect::new(rr.x, rr.y, rr.w, 38.0);
      let editable = !k.locked && row.src != Src::Fixed;
      let capturing = k.capturing.as_deref() == Some(row.key.as_str());
      let conflict = k.conflict_for(&row.key).cloned();
      let hit = Hit::Keys(KHit::Row(row.key.clone()));
      let fg = if capturing { cx.t.on_primary_container } else { cx.t.on_layer1 };
      if capturing {
        cx.round(rr, 12.0, cx.t.primary_container)?;
      } else if conflict.is_some() {
        cx.round(rr, 12.0, blend(cx.t.layer1, cx.t.error, 0.22))?;
        cx.p.stroke_round(rr.inset(0.5, 0.5), 12.0, cx.t.error, 1.0)?;
      } else if editable && cx.hot(&hit) {
        cx.round(rr, 12.0, cx.c.layer2_hover)?;
      }
      // the mini buttons, then the keys, right to left
      let mut right = line.right() - 8.0;
      let mut minis: Vec<(KHit, &str)> = Vec::new();
      if !k.locked && row.app && !capturing {
        minis.push((KHit::Remove(row.key.clone()), "delete"));
      }
      if !k.locked && row.src != Src::Fixed && !row.combo.is_empty() && !capturing {
        minis.push((KHit::Clear(row.key.clone()), "close"));
      }
      if !k.locked && row.src == Src::Core && !row.default.is_empty() && row.combo != row.default && !capturing {
        minis.push((KHit::Undo(row.key.clone()), "undo"));
      }
      let mut mini_rects = Vec::new();
      for (h, icon) in &minis {
        let b = Rect::new(right - 26.0, line.y + 6.0, 26.0, 26.0);
        mini_rects.push((b, h.clone(), *icon));
        right -= 26.0 + 8.0;
      }
      let kw = if capturing {
        let text = cx.tr("Tuşlara bas…");
        let w = cx.measure(&text, st(13.0))?.ceil() + 24.0;
        // `kbPulse`: breathing while it waits
        let t = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()) % 1000) as f32 / 1000.0;
        let pulse = 0.725 + 0.275 * (t * std::f32::consts::TAU).cos();
        cx.busy = true;
        cx.icon("keyboard", right - w + 9.0, line.y + 19.0, 18.0, false, fg.alpha(pulse))?;
        cx.text(&text, Rect::new(right - w + 24.0, line.y, w - 24.0, 38.0), st(13.0), fg.alpha(pulse))?;
        w
      } else {
        keycaps(cx, &row.combo, right, line.y + 19.0, true)?
      };
      cx.text(&row.label, Rect::new(line.x + 12.0, line.y, (right - kw - 8.0 - line.x - 12.0).max(20.0), 38.0), st(13.5), fg)?;
      if let (Some(c), false) = (&conflict, capturing) {
        let text = conflict_text(cx, &rows, row, c);
        cx.icon("error", line.x + 12.0 + 7.0, line.y + 38.0 + 8.0, 14.0, false, cx.t.error)?;
        cx.text(&text, Rect::new(line.x + 12.0 + 18.0, line.y + 36.0, line.w - 40.0, 18.0), st(12.0), cx.t.error)?;
      }
      cx.hit(rr, hit);
      for (b, h, icon) in mini_rects {
        let hh = Hit::Keys(h);
        if cx.hot(&hh) {
          cx.round(b, 13.0, cx.c.layer3)?;
        }
        cx.icon(icon, b.x + 13.0, b.y + 13.0, 16.0, false, if cx.hot(&hh) { cx.t.on_layer1 } else { cx.t.on_surface_variant })?;
        cx.hit(b, hh);
      }
      ry += rh + 2.0;
    }
    if add {
      // "+": a shortcut for any app
      let b = Rect::new(gr.x + 6.0, ry + 2.0, gr.w - 12.0, 36.0);
      let h = Hit::Keys(KHit::Add);
      cx.round(b, 12.0, if cx.hot(&h) { cx.c.layer2_hover } else { cx.c.layer2 })?;
      cx.icon("add", b.x + 12.0 + 9.0, b.y + 18.0, 18.0, false, cx.t.primary)?;
      cx.text(&cx.tr("Uygulama ekle"), Rect::new(b.x + 12.0 + 18.0 + 8.0, b.y, b.w - 50.0, 36.0), stw(13.0, 550.0), cx.t.primary)?;
      cx.hit(b, h);
    }
    y += gh + 10.0;
  }
  Ok(y - r.y)
}

/// The bar at the page's bottom: "Kaydet" (shakes red while something
/// clashes) and "Vazgeç" for the unsaved changes.
pub(super) fn paint_footer(cx: &mut Cx, k: &Keys, f: Rect) -> anyhow::Result<()> {
  if k.locked {
    return Ok(());
  }
  cx.p.fill(Rect::new(f.x + 10.0, f.y, f.w - 20.0, 1.0), cx.t.outline_variant)?;
  let dirty = k.dirty();
  let clash = !k.conflicts.is_empty();
  let save = cx.tr("Kaydet");
  let sw = cx.measure(&save, stw(14.0, 600.0))?.ceil() + 24.0 + 26.0;
  let shaking = k.shaking(cx.now);
  if shaking.is_some() {
    cx.busy = true;
  }
  let dx = match shaking {
    Some(t) if k.animations => (1.0 - t) * 9.0 * (t * std::f32::consts::PI * 7.0).sin(),
    _ => 0.0,
  };
  let sb = Rect::new(f.right() - 10.0 - sw + dx, f.y + 10.0, sw, 36.0);
  let hs = Hit::Keys(KHit::Save);
  let red = shaking.is_some() || (clash && dirty);
  let (bg, fg) = if red {
    (if cx.hot(&hs) { blend(cx.t.error, cx.t.on_primary, 0.12) } else { cx.t.error }, cx.t.on_primary)
  } else if dirty {
    (if cx.hot(&hs) { blend(cx.t.primary, cx.t.on_primary, 0.12) } else { cx.t.primary }, cx.t.on_primary)
  } else {
    (cx.c.layer2, cx.t.on_surface_variant)
  };
  cx.round(sb, 18.0, bg)?;
  cx.icon(if red { "error" } else { "save" }, sb.x + 14.0 + 9.0, sb.y + 18.0, 18.0, false, fg)?;
  cx.text(&save, Rect::new(sb.x + 14.0 + 18.0 + 6.0, sb.y, sw - 40.0, 36.0), stw(14.0, 600.0), fg)?;
  cx.hit(sb, hs);
  if dirty {
    let discard = cx.tr("Vazgeç");
    let dw = cx.measure(&discard, st(13.0))?.ceil() + 28.0;
    let db = Rect::new(f.right() - 10.0 - sw - 8.0 - dw, f.y + 10.0, dw, 36.0);
    let hd = Hit::Keys(KHit::Discard);
    cx.round(db, 18.0, if cx.hot(&hd) { cx.c.layer2_hover } else { cx.c.layer2 })?;
    cx.text_center(&discard, db, st(13.0), cx.t.on_layer1)?;
    cx.hit(db, hd);
    let note = if clash { cx.tr("Çakışan kısayollar var") } else { cx.tr("Kaydedilmemiş değişiklikler") };
    cx.text(&note, Rect::new(f.x + 14.0, f.y + 10.0, (db.x - 8.0 - f.x - 14.0).max(10.0), 36.0), st(12.5), if clash { cx.t.error } else { cx.t.on_surface_variant })?;
  }
  Ok(())
}

/// The app picker over the page; returns how far its list can scroll.
pub(super) fn paint_overlay(cx: &mut Cx, k: &mut Keys, field: &mut TextField, scroll: f32, panel: Rect) -> anyhow::Result<f32> {
  if !k.picker {
    return Ok(0.0);
  }
  cx.round(panel, 19.0, Rgba(4, 3, 7, 0.6))?;
  cx.hit(panel, Hit::Keys(KHit::Dismiss));
  let b = Rect::new(panel.x + 14.0, panel.y + 60.0, panel.w - 28.0, panel.h - 120.0);
  cx.shadow(b, 18.0, 1.0)?;
  cx.round(b, 18.0, cx.t.layer1)?;
  cx.p.stroke_round(b.inset(0.5, 0.5), 18.0, cx.t.outline_variant, 1.0)?;
  cx.hit(b, Hit::Panel);
  cx.icon("apps", b.x + 18.0 + 12.0, b.y + 18.0 + 13.0, 22.0, false, cx.t.primary)?;
  cx.text(&cx.tr("Uygulama ekle"), Rect::new(b.x + 18.0 + 32.0, b.y + 18.0, b.w - 60.0, 26.0), stw(15.0, 600.0), cx.t.on_layer1)?;
  let fr = Rect::new(b.x + 14.0, b.y + 56.0, b.w - 28.0, 38.0);
  let ph = cx.tr("Uygulama ara");
  let bg = cx.c.layer2;
  cx.field_box(fr, 19.0, field, FieldId::KeysApp, &ph, st(14.0), 14.0, Some(bg))?;
  // the list, scrolled; "Gözat…" under it
  let lr = Rect::new(b.x + 8.0, fr.bottom() + 8.0, b.w - 16.0, b.bottom() - 14.0 - 36.0 - 10.0 - (fr.bottom() + 8.0));
  let q = field.text().to_lowercase();
  let list: Vec<String> = k.apps.iter().filter(|(n, _)| q.is_empty() || n.to_lowercase().contains(&q)).map(|(n, _)| n.clone()).collect();
  cx.push_clip(lr);
  let mut y = lr.y - scroll;
  for (i, name) in list.iter().enumerate() {
    let rr = Rect::new(lr.x, y, lr.w, 36.0);
    if cx.visible(rr) {
      let h = Hit::Keys(KHit::Pick(i));
      if cx.hot(&h) {
        cx.round(rr, 12.0, cx.c.layer2_hover)?;
      }
      cx.icon("apps", rr.x + 12.0 + 9.0, rr.y + 18.0, 18.0, false, cx.t.on_surface_variant)?;
      cx.text(name, Rect::new(rr.x + 12.0 + 18.0 + 10.0, rr.y, rr.w - 50.0, 36.0), st(13.5), cx.t.on_layer1)?;
      cx.hit(rr, h);
    }
    y += 38.0;
  }
  if list.is_empty() {
    cx.text_center(&cx.tr("Uygulama bulunamadı"), Rect::new(lr.x, lr.y, lr.w, 40.0), st(13.0), cx.t.on_surface_variant)?;
  }
  cx.pop_clip();
  let content = list.len() as f32 * 38.0;
  cx.region(lr, ScrollId::KeysApps, content, false);
  let browse = cx.tr("Gözat…");
  let bw = cx.chip_w(&browse, Some("folder_open"))?;
  cx.chip(b.right() - 14.0 - bw, b.bottom() - 14.0 - 36.0 + 2.0, 32.0, &browse, Some("folder_open"), false, true, Hit::Keys(KHit::Browse))?;
  Ok((content - lr.h).max(0.0))
}

impl Ui {
  pub(super) fn sb_keys_open(&mut self) {
    let animations = self.model.animations;
    let apps: Vec<(String, String)> = self.icons.apps().iter().filter(|a| !a.path.is_empty()).map(|a| (a.name.clone(), a.path.clone())).collect();
    let k = &mut self.sidebar.keys;
    k.locked = true;
    k.capturing = None;
    k.msg = None;
    k.picker = false;
    k.shake = None;
    k.animations = animations;
    k.apps = apps;
    self.sidebar.field(FieldId::KeysSearch).set("");
    sb_keys_load();
  }

  /// Esc on the page: the app picker closes first.
  pub(super) fn sb_keys_escape(&mut self) -> bool {
    let k = &mut self.sidebar.keys;
    if k.picker {
      k.picker = false;
      return true;
    }
    false
  }

  pub(super) fn sb_keys_event(&mut self, e: KEv) {
    match e {
      KEv::Loaded(model) => {
        let k = &mut self.sidebar.keys;
        if let Some(m) = model.filter(|m| m.is_object()) {
          k.saved_apps = custom_apps(&m);
          k.saved_removed = removed_apps(&m);
          k.staged = Staged { apps: k.saved_apps.clone(), removed: k.saved_removed.clone(), ..Default::default() };
          k.conflicts = conflicts_of(&m["conflicts"]);
          k.model = m;
        }
        k.busy = false;
      }
      KEv::Captured(key, combo) => {
        self.sidebar.keys.capturing = None;
        if !combo.is_empty() {
          self.sb_keys_stage(&key, combo);
        }
      }
      KEv::Checked(seq, conflicts) => {
        let k = &mut self.sidebar.keys;
        if seq == k.check_seq {
          k.conflicts = conflicts;
        }
      }
      KEv::Saved(ok, conflicts, text) => {
        self.sidebar.keys.busy = false;
        if ok {
          self.sidebar.keys.msg = None;
          send(Msg::Toast(json!({ "kind": "ok", "icon": "keyboard", "title": "Kısayollar kaydedildi", "timeout": 2500 })));
          sb_keys_load();
        } else {
          if !conflicts.is_empty() {
            self.sidebar.keys.conflicts = conflicts;
          }
          self.sb_keys_refuse(&text);
        }
      }
      KEv::Reset(ok, text) => {
        self.sidebar.keys.busy = false;
        self.sidebar.keys.msg = Some((ok, text));
        sb_keys_load();
      }
      KEv::Picked(app) => {
        self.sb_modal(false);
        if let Some((path, name)) = app {
          self.sb_keys_add_app(name, path);
        }
      }
    }
    self.sb_render();
  }

  /// Save refused: the button shakes red and our card says what clashes.
  fn sb_keys_refuse(&mut self, why: &str) {
    self.sidebar.keys.shake = Some(Instant::now());
    let tr = |s: &str| self.model.tr(s);
    let rows = self.sidebar.keys.rows(&tr);
    let lines: Vec<String> = self
      .sidebar
      .keys
      .conflicts
      .iter()
      .take(3)
      .map(|c| {
        let names: Vec<String> = c.keys.iter().filter_map(|key| rows.iter().find(|r| &r.key == key).map(|r| r.label.clone())).collect();
        match &c.reserved {
          Some(r) => format!("{}: {} ({})", c.combo, names.join(", "), tr(r)),
          None => format!("{}: {}", c.combo, names.join(", ")),
        }
      })
      .collect();
    let body = if lines.is_empty() { why.to_string() } else { lines.join("\n") };
    let title = if self.sidebar.keys.conflicts.is_empty() { "Kısayollar kaydedilemedi" } else { "Kısayollar kaydedilmedi: çakışma var" };
    send(Msg::Toast(json!({ "kind": "error", "icon": "keyboard", "title": title, "body": body, "timeout": 6000 })));
    self.sb_frames();
  }

  /// A row's new keys go to the staged state, then the core checks it all.
  fn sb_keys_stage(&mut self, key: &str, combo: String) {
    let k = &mut self.sidebar.keys;
    k.msg = None;
    if let Some(id) = key.strip_prefix("ll:") {
      if let Some(a) = k.staged.apps.iter_mut().find(|a| a.id == id) {
        a.combo = combo;
      } else {
        let saved = k.model["core"].as_array().into_iter().flatten().find(|b| b["id"] == id).and_then(|b| b["combo"].as_str()).unwrap_or("").to_string();
        if saved == combo {
          k.staged.core.remove(id);
        } else {
          k.staged.core.insert(id.to_string(), combo);
        }
      }
    } else if let Some(index) = key.strip_prefix("tiling:").and_then(|i| i.parse::<i64>().ok()) {
      let saved = k.model["tiling"].as_array().into_iter().flatten().find(|g| g["index"].as_i64() == Some(index)).map(|g| strs(&g["bindings"])).unwrap_or_default();
      let mut b = k.staged.tiling.get(&index).cloned().unwrap_or_else(|| saved.clone());
      // the first binding changes, the others stay (cleared: the first goes)
      if combo.is_empty() {
        if !b.is_empty() {
          b.remove(0);
        }
      } else if b.is_empty() {
        b.push(combo);
      } else {
        b[0] = combo;
      }
      if b == saved {
        k.staged.tiling.remove(&index);
      } else {
        k.staged.tiling.insert(index, b);
      }
    }
    self.sb_keys_check();
  }

  fn sb_keys_check(&mut self) {
    let k = &mut self.sidebar.keys;
    k.check_seq += 1;
    let (seq, staged) = (k.check_seq, k.staged_json());
    std::thread::spawn(move || {
      let out = core_api::run_core_output(&["--keybinds-check", &staged]).and_then(|s| serde_json::from_str::<Value>(&s).ok());
      if let Some(v) = out.filter(|v| v["ok"].as_bool() == Some(true)) {
        send(Msg::Sidebar(Ev::Keys(KEv::Checked(seq, conflicts_of(&v["conflicts"])))));
      }
    });
  }

  fn sb_keys_add_app(&mut self, name: String, path: String) {
    let k = &mut self.sidebar.keys;
    k.picker = false;
    let id = new_app_id();
    k.staged.apps.push(CustomApp { id: id.clone(), name, path, combo: String::new() });
    self.sb_keys_check();
    // its keys straight away
    self.sb_keys_capture(format!("ll:{id}"));
  }

  fn sb_keys_capture(&mut self, key: String) {
    let k = &mut self.sidebar.keys;
    k.msg = None;
    k.capturing = Some(key.clone());
    self.sb_frames();
    std::thread::spawn(move || {
      let combo = core_api::run_core_output(&["--capture"]).unwrap_or_default();
      send(Msg::Sidebar(Ev::Keys(KEv::Captured(key, combo.trim().to_string()))));
    });
  }

  pub(super) fn sb_keys_click(&mut self, h: KHit, button: u8) {
    if button != 0 {
      return;
    }
    let k = &mut self.sidebar.keys;
    // the open picker takes every click until it closes
    if k.picker && !matches!(h, KHit::Dismiss | KHit::Pick(_) | KHit::Browse) {
      return self.sb_render();
    }
    match h {
      KHit::Lock => {
        k.locked = !k.locked;
        k.msg = None;
      }
      KHit::Reset if !k.locked => {
        // asked only when the user added apps: keep them or remove them too
        if k.saved_apps.is_empty() {
          self.sb_keys_reset(false);
        } else {
          let tr = |s: &str| self.model.tr(s);
          let spec = Spec::new(Kind::Question, tr("Kısayollar sıfırlansın mı?"), tr("Bütün kısayollar varsayılanlarına döner."), vec![tr("Sıfırla"), tr("Vazgeç")])
            .cancel(1)
            .default_button(0)
            .checkbox(tr("Kendi eklediğin uygulamalar da kaldırılsın"), false);
          self.dialog_open(spec, |ui: &mut Ui, answer: Answer| {
            if answer.button == Some(0) {
              ui.sb_keys_reset(answer.checked);
              ui.sb_render();
            }
          });
        }
      }
      KHit::Dismiss => {
        k.picker = false;
      }
      KHit::Row(key) => {
        let fixed = key == "super" || key == "dock";
        if k.locked || fixed || k.capturing.is_some() {
          return self.sb_render();
        }
        self.sb_keys_capture(key);
      }
      KHit::Undo(key) => {
        let tr = |s: &str| self.model.tr(s);
        if let Some(def) = self.sidebar.keys.rows(&tr).iter().find(|r| r.key == key).map(|r| r.default.clone()) {
          self.sb_keys_stage(&key, def);
        }
      }
      KHit::Clear(key) => self.sb_keys_stage(&key, String::new()),
      KHit::Remove(key) => {
        if let Some(id) = key.strip_prefix("ll:") {
          if id.starts_with("app:") {
            k.staged.apps.retain(|a| a.id != id);
          } else {
            k.staged.removed.insert(id.to_string());
            k.staged.core.remove(id);
          }
          self.sb_keys_check();
        }
      }
      KHit::Add if !k.locked => {
        k.picker = true;
        self.sidebar.field(FieldId::KeysApp).set("");
        self.sidebar.scroll.remove(&ScrollId::KeysApps);
        self.sidebar.focus = Some(FieldId::KeysApp);
      }
      KHit::Pick(i) => {
        let q = self.sidebar.field(FieldId::KeysApp).text().to_lowercase();
        let pick = self.sidebar.keys.apps.iter().filter(|(n, _)| q.is_empty() || n.to_lowercase().contains(&q)).nth(i).cloned();
        if let Some((name, path)) = pick {
          self.sb_keys_add_app(name, path);
        }
      }
      KHit::Browse => {
        self.sb_modal(true);
        std::thread::spawn(|| {
          let v = core_api::run_core_output(&["--keybinds-pick-app"]).and_then(|s| serde_json::from_str::<Value>(&s).ok());
          let app = v.and_then(|v| Some((v["path"].as_str()?.to_string(), v["name"].as_str()?.to_string())));
          send(Msg::Sidebar(Ev::Keys(KEv::Picked(app))));
        });
      }
      KHit::Discard => {
        k.staged = Staged { apps: k.saved_apps.clone(), removed: k.saved_removed.clone(), ..Default::default() };
        k.msg = None;
        self.sb_keys_check();
      }
      KHit::Save if !k.locked && k.dirty() && !k.busy => {
        if !k.conflicts.is_empty() {
          self.sb_keys_refuse("");
        } else {
          k.busy = true;
          let staged = k.staged_json();
          let failed = self.model.tr("Kısayollar kaydedilemedi");
          std::thread::spawn(move || {
            let v = core_api::run_core_output(&["--keybinds-save", &staged]).and_then(|s| serde_json::from_str::<Value>(&s).ok());
            let ok = v.as_ref().is_some_and(|v| v["ok"].as_bool() == Some(true));
            let conflicts = v.as_ref().map(|v| conflicts_of(&v["conflicts"])).unwrap_or_default();
            send(Msg::Sidebar(Ev::Keys(KEv::Saved(ok, conflicts, failed))));
          });
        }
      }
      _ => {}
    }
    self.sb_render();
  }

  fn sb_keys_reset(&mut self, apps: bool) {
    let k = &mut self.sidebar.keys;
    k.busy = true;
    k.capturing = None;
    let (ok_text, err_text) = (self.model.tr("Tüm kısayollar varsayılana döndü"), self.model.tr("Kısayollar sıfırlanamadı"));
    std::thread::spawn(move || {
      let args: &[&str] = if apps { &["--keybinds-reset", "--apps"] } else { &["--keybinds-reset"] };
      let ok = core_api::run_core_output(args).and_then(|s| serde_json::from_str::<Value>(&s).ok()).is_some_and(|v| v["ok"].as_bool() == Some(true));
      send(Msg::Sidebar(Ev::Keys(KEv::Reset(ok, if ok { ok_text } else { err_text }))));
    });
  }

  pub(super) fn sb_keys_typed(&mut self, t: Typed) {
    let _ = t;
    self.sidebar.scroll.remove(&ScrollId::Page);
  }

  pub(super) fn sb_keys_app_typed(&mut self, t: Typed) {
    let _ = t;
    self.sidebar.scroll.remove(&ScrollId::KeysApps);
  }
}

fn sb_keys_load() {
  std::thread::spawn(|| {
    let model = core_api::run_core_output(&["--keybinds-model"]).and_then(|s| serde_json::from_str::<Value>(&s).ok());
    send(Msg::Sidebar(Ev::Keys(KEv::Loaded(model))));
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn model() -> Value {
    json!({
      "core": [
        { "id": "browser", "combo": "Super+W", "default": "Super+W", "app": true, "custom": false, "removed": false },
        { "id": "ws-1", "combo": "Super+1", "default": "Super+1", "app": false, "custom": false, "removed": false },
        { "id": "files", "combo": "Super+E", "default": "Super+E", "app": true, "custom": false, "removed": true },
        { "id": "app:x1", "combo": "Super+K", "default": "", "app": true, "custom": true, "removed": false, "name": "Spotify", "path": "C:\\s.exe" }
      ],
      "tiling": [{ "index": 0, "commands": ["toggle-fullscreen"], "bindings": ["Super+F", "Super+Shift+F"] }],
      "conflicts": [{ "combo": "Super+W", "keys": ["ll:browser", "tiling:0"], "reserved": null }]
    })
  }

  fn loaded() -> Keys {
    let m = model();
    let mut k = Keys { saved_apps: custom_apps(&m), saved_removed: removed_apps(&m), conflicts: conflicts_of(&m["conflicts"]), ..Default::default() };
    k.staged = Staged { apps: k.saved_apps.clone(), removed: k.saved_removed.clone(), ..Default::default() };
    k.model = m;
    k
  }

  #[test]
  fn window_manager_commands_get_names_and_groups() {
    let c = |s: &str| s.split(" ; ").map(str::to_string).collect::<Vec<_>>();
    assert_eq!(tiling_label(&c("move --workspace 3")), "Pencereyi workspace 3'e gönder");
    assert_eq!(tiling_label(&c("move --next-workspace ; focus --next-workspace")), "Pencereyi sonraki workspace'e taşı (PageDown)");
    assert_eq!(tiling_group(&c("shell-exec wezterm")), "app");
    assert_eq!(tiling_group(&c("wm-exit")), "sys");
    assert_eq!(tiling_group(&c("toggle-fullscreen")), "win");
    assert_eq!(tiling_group(&c("move-workspace --direction left")), "mon");
    assert_eq!(tiling_group(&c("focus --workspace-in-direction right")), "mon");
    assert_eq!(tiling_group(&c("focus --next-active-workspace-on-monitor")), "ws");
  }

  #[test]
  fn core_shortcuts_get_names_and_groups() {
    assert_eq!(ll_label("ws-4"), "Workspace 4");
    assert_eq!(ll_group("ws-4", false), "ws");
    assert_eq!(ll_group("clipboard", false), "sys");
    assert_eq!(ll_group("browser", true), "app");
    assert_eq!(ll_label("task-manager"), "Görev Yöneticisi");
  }

  #[test]
  fn rows_hide_removed_apps_and_list_added_ones() {
    let k = loaded();
    let rows = k.rows(&|s| s.to_string());
    assert_eq!(rows[0].key, "super");
    assert!(rows.iter().all(|r| r.key != "ll:files"), "a removed app stays hidden");
    let added = rows.iter().find(|r| r.key == "ll:app:x1").expect("the added app");
    assert_eq!((added.label.as_str(), added.combo.as_str(), added.app), ("Spotify", "Super+K", true));
    let wm = rows.iter().find(|r| r.key == "tiling:0").expect("the window manager row");
    assert_eq!((wm.combo.as_str(), wm.extra.clone()), ("Super+F", vec!["Super+Shift+F".to_string()]));
    assert!(k.conflict_for("ll:browser").is_some() && k.conflict_for("ll:ws-1").is_none());
  }

  #[test]
  fn staged_state_is_what_the_core_reads() {
    let mut k = loaded();
    assert!(!k.dirty());
    k.staged.core.insert("ws-1".into(), "Super+F1".into());
    k.staged.removed.insert("browser".into());
    k.staged.tiling.insert(0, vec!["Super+G".into()]);
    assert!(k.dirty());
    let v: Value = serde_json::from_str(&k.staged_json()).unwrap();
    assert_eq!(v["core"]["ws-1"], "Super+F1");
    assert_eq!(v["tiling"]["0"][0], "Super+G");
    assert_eq!(v["apps"][0]["id"], "app:x1");
    assert!(strs(&v["removed"]).contains(&"browser".to_string()) && strs(&v["removed"]).contains(&"files".to_string()));
  }

  #[test]
  fn new_app_ids_are_valid_for_the_core() {
    let id = new_app_id();
    assert!(id.starts_with("app:") && id.len() > 4 && id[4..].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
  }
}
