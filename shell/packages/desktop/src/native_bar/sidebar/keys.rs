//! The shortcut editor page (sidebar.html ShortcutsPage): the core's own
//! shortcuts (`--keybinds`, `--bind`, `--bind-reset`) and the window
//! manager's (`scripts\keybinds-tiling.ps1`), grouped, searchable, read-only
//! until unlocked. A row is changed by pressing it and then the new keys
//! (the core catches them: `--capture`); Esc cancels.

use serde_json::Value;

use super::{
  super::{core_api, gfx::{Rect, Rgba}, send, Msg, Ui},
  kit::{st, stw, Cx},
  quick::ps,
  text::{TextField, Typed},
  Ev, FieldId, Hit,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum KHit {
  Lock,
  Reset,
  Row(String),
  Undo(String),
  Clear(String),
}

pub(in crate::native_bar) enum KEv {
  Loaded(Option<Value>, Option<Value>),
  Captured(String, String),
  Done(bool, String),
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
  /// the core's id or the window manager's index
  id: String,
}

#[derive(Default)]
pub(super) struct Keys {
  core: Vec<Value>,
  tiling: Vec<Value>,
  pub locked: bool,
  pub capturing: Option<String>,
  msg: Option<(bool, String)>,
  query: String,
}

const GROUPS: [(&str, &str, &str); 4] = [("win", "Pencereler", "select_window"), ("ws", "Workspace", "view_carousel"), ("app", "Uygulamalar", "apps"), ("sys", "Sistem", "settings")];

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
    "screenshot-screen" => "Monitörün tamamı (panoya)",
    "clipboard" => "Pano geçmişi",
    "file-search" => "Dosya araması",
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

fn ll_group(id: &str) -> &'static str {
  if id.starts_with("focus-") || id.starts_with("move-") || id == "close" {
    "win"
  } else if id.starts_with("ws-") {
    "ws"
  } else if id.starts_with("screenshot") || id == "clipboard" || id == "file-search" {
    "sys"
  } else {
    "app"
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
    "toggle-floating --centered" => "Yüzen pencere",
    "toggle-tiling-direction" => "Bölme yönünü değiştir",
    "toggle-minimized" => "Simge durumuna küçült",
    "wm-cycle-focus" => "Yüzen / döşeli pencereler arasında geç",
    "resize --width -10%" => "Pencereyi daralt",
    "resize --width +10%" => "Pencereyi genişlet",
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
  if c.contains("workspace") {
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
    other => other,
  }
}

impl Keys {
  fn rows(&self, tr: &dyn Fn(&str) -> String) -> Vec<Row> {
    let mut rows = vec![
      Row { key: "super".into(), group: "sys", label: tr("Arama / overview"), combo: "Super".into(), default: String::new(), extra: vec![], src: Src::Fixed, id: String::new() },
      Row { key: "dock".into(), group: "sys", label: tr("Uygulama Dock’u"), combo: "Super+Alt".into(), default: String::new(), extra: vec![], src: Src::Fixed, id: String::new() },
    ];
    for b in &self.core {
      let id = b["id"].as_str().unwrap_or("").to_string();
      rows.push(Row {
        key: format!("ll:{id}"),
        group: ll_group(&id),
        label: tr(&ll_label(&id)),
        combo: b["combo"].as_str().unwrap_or("").to_string(),
        default: b["default"].as_str().unwrap_or("").to_string(),
        extra: vec![],
        src: Src::Core,
        id,
      });
    }
    for g in &self.tiling {
      let cmds: Vec<String> = g["commands"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default();
      let binds: Vec<String> = g["bindings"].as_array().map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default();
      let index = g["index"].as_i64().unwrap_or(-1).to_string();
      rows.push(Row {
        key: format!("tiling:{index}"),
        group: tiling_group(&cmds),
        label: tr(&tiling_label(&cmds)),
        combo: binds.first().cloned().unwrap_or_default(),
        default: String::new(),
        extra: binds.iter().skip(1).cloned().collect(),
        src: Src::Tiling,
        id: index,
      });
    }
    rows
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
    .into_iter()
    .filter(|row| q.is_empty() || row.label.to_lowercase().contains(&q) || row.combo.to_lowercase().contains(&q))
    .collect();
  for (g, title, icon) in GROUPS {
    let list: Vec<&Row> = shown.iter().filter(|row| row.group == g).collect();
    if list.is_empty() {
      continue;
    }
    let gh = 6.0 + 28.0 + list.len() as f32 * 40.0 + 6.0;
    let gr = Rect::new(r.x, y, r.w, gh);
    cx.round(gr, 17.0, cx.t.layer1)?;
    cx.section_title(gr.x + 6.0, gr.y + 6.0, gr.w - 12.0, icon, &cx.tr(title))?;
    let mut ry = gr.y + 6.0 + 28.0;
    for row in list {
      let rr = Rect::new(gr.x + 6.0, ry, gr.w - 12.0, 38.0);
      let editable = !k.locked && row.src != Src::Fixed;
      let capturing = k.capturing.as_deref() == Some(row.key.as_str());
      let hit = Hit::Keys(KHit::Row(row.key.clone()));
      let fg = if capturing { cx.t.on_primary_container } else { cx.t.on_layer1 };
      if capturing {
        cx.round(rr, 12.0, cx.t.primary_container)?;
      } else if editable && cx.hot(&hit) {
        cx.round(rr, 12.0, cx.c.layer2_hover)?;
      }
      // the mini buttons, then the keys, right to left
      let mut right = rr.right() - 8.0;
      let mut minis: Vec<(KHit, &str)> = Vec::new();
      if !k.locked && row.src != Src::Fixed && !row.combo.is_empty() && !capturing {
        minis.push((KHit::Clear(row.key.clone()), "close"));
      }
      if !k.locked && row.src == Src::Core && !row.default.is_empty() && row.combo != row.default && !capturing {
        minis.push((KHit::Undo(row.key.clone()), "undo"));
      }
      let mut mini_rects = Vec::new();
      for (h, icon) in &minis {
        let b = Rect::new(right - 26.0, rr.y + 6.0, 26.0, 26.0);
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
        cx.icon("keyboard", right - w + 9.0, rr.y + 19.0, 18.0, false, fg.alpha(pulse))?;
        cx.text(&text, Rect::new(right - w + 24.0, rr.y, w - 24.0, 38.0), st(13.0), fg.alpha(pulse))?;
        w
      } else {
        keycaps(cx, &row.combo, right, rr.y + 19.0, true)?
      };
      cx.text(&row.label, Rect::new(rr.x + 12.0, rr.y, (right - kw - 8.0 - rr.x - 12.0).max(20.0), 38.0), st(13.5), fg)?;
      cx.hit(rr, hit);
      for (b, h, icon) in mini_rects {
        let hh = Hit::Keys(h);
        if cx.hot(&hh) {
          cx.round(b, 13.0, cx.c.layer3)?;
        }
        cx.icon(icon, b.x + 13.0, b.y + 13.0, 16.0, false, if cx.hot(&hh) { cx.t.on_layer1 } else { cx.t.on_surface_variant })?;
        cx.hit(b, hh);
      }
      ry += 40.0;
    }
    y += gh + 10.0;
  }
  Ok(y - r.y)
}

impl Ui {
  pub(super) fn sb_keys_open(&mut self) {
    let k = &mut self.sidebar.keys;
    k.locked = true;
    k.capturing = None;
    k.msg = None;
    self.sidebar.field(FieldId::KeysSearch).set("");
    sb_keys_load();
  }

  pub(super) fn sb_keys_event(&mut self, e: KEv) {
    match e {
      KEv::Loaded(core, tiling) => {
        let k = &mut self.sidebar.keys;
        if let Some(Value::Array(a)) = core {
          k.core = a;
        }
        if let Some(Value::Array(a)) = tiling {
          k.tiling = a;
        }
      }
      KEv::Captured(key, combo) => {
        self.sidebar.keys.capturing = None;
        if !combo.is_empty() {
          self.sb_keys_apply(&key, combo, true);
        }
      }
      KEv::Done(ok, text) => {
        self.sidebar.keys.msg = Some((ok, text));
        sb_keys_load();
      }
    }
    self.sb_render();
  }

  fn sb_keys_apply(&mut self, key: &str, combo: String, check_clash: bool) {
    let tr = |s: &str| self.model.tr(s);
    let rows = self.sidebar.keys.rows(&tr);
    let Some(row) = rows.iter().find(|r| r.key == key).cloned() else { return };
    if check_clash && !combo.is_empty() {
      if let Some(clash) = rows.iter().find(|r| r.key != row.key && (r.combo == combo || r.extra.contains(&combo))) {
        let text = tr(&format!("{} zaten \"{}\" için kullanılıyor", combo, clash.label));
        self.sidebar.keys.msg = Some((false, text));
        return;
      }
    }
    let label = row.label.clone();
    let off = tr("kapatıldı");
    let failed = tr("kaydedilemedi");
    std::thread::spawn(move || {
      let out = match row.src {
        Src::Core => core_api::run_core_output(&["--bind", &row.id, &combo]).and_then(|s| serde_json::from_str::<Value>(&s).ok()),
        Src::Tiling => ps("keybinds-tiling.ps1", &["set", &row.id, &combo]),
        Src::Fixed => None,
      };
      let ok = out.is_some_and(|v| v["ok"].as_bool() == Some(true));
      let text = if ok { format!("{}: {}", label, if combo.is_empty() { off } else { combo }) } else { format!("{label}: {failed}") };
      send(Msg::Sidebar(Ev::Keys(KEv::Done(ok, text))));
    });
  }

  pub(super) fn sb_keys_click(&mut self, h: KHit, button: u8) {
    if button != 0 {
      return;
    }
    let k = &mut self.sidebar.keys;
    match h {
      KHit::Lock => {
        k.locked = !k.locked;
        k.msg = None;
      }
      KHit::Reset if !k.locked => {
        let (ok_text, err_text) = (self.model.tr("Tüm kısayollar varsayılana döndü"), self.model.tr("Kısayollar sıfırlanamadı"));
        std::thread::spawn(move || {
          let a = core_api::run_core_output(&["--bind-reset"]).and_then(|s| serde_json::from_str::<Value>(&s).ok()).is_some_and(|v| v["ok"].as_bool() == Some(true));
          let b = ps("keybinds-tiling.ps1", &["reset"]).is_some_and(|v| v["ok"].as_bool() == Some(true));
          let ok = a && b;
          send(Msg::Sidebar(Ev::Keys(KEv::Done(ok, if ok { ok_text } else { err_text }))));
        });
      }
      KHit::Row(key) => {
        let fixed = key == "super" || key == "dock";
        if k.locked || fixed || k.capturing.is_some() {
          return self.sb_render();
        }
        k.msg = None;
        k.capturing = Some(key.clone());
        self.sb_frames();
        std::thread::spawn(move || {
          let combo = core_api::run_core_output(&["--capture"]).unwrap_or_default();
          send(Msg::Sidebar(Ev::Keys(KEv::Captured(key, combo.trim().to_string()))));
        });
      }
      KHit::Undo(key) => {
        let tr = |s: &str| self.model.tr(s);
        if let Some(def) = self.sidebar.keys.rows(&tr).iter().find(|r| r.key == key).map(|r| r.default.clone()) {
          self.sb_keys_apply(&key, def, false);
        }
      }
      KHit::Clear(key) => self.sb_keys_apply(&key, String::new(), false),
      _ => {}
    }
    self.sb_render();
  }

  pub(super) fn sb_keys_typed(&mut self, t: Typed) {
    let _ = t;
    self.sidebar.scroll.remove(&super::ScrollId::Page);
  }
}

fn sb_keys_load() {
  std::thread::spawn(|| {
    let core = core_api::run_core_output(&["--keybinds"]).and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let tiling = ps("keybinds-tiling.ps1", &["list"]);
    send(Msg::Sidebar(Ev::Keys(KEv::Loaded(core, tiling))));
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn window_manager_commands_get_names_and_groups() {
    let c = |s: &str| s.split(" ; ").map(str::to_string).collect::<Vec<_>>();
    assert_eq!(tiling_label(&c("move --workspace 3")), "Pencereyi workspace 3'e gönder");
    assert_eq!(tiling_label(&c("move --next-workspace ; focus --next-workspace")), "Pencereyi sonraki workspace'e taşı (PageDown)");
    assert_eq!(tiling_group(&c("shell-exec wezterm")), "app");
    assert_eq!(tiling_group(&c("wm-exit")), "sys");
    assert_eq!(tiling_group(&c("toggle-fullscreen")), "win");
  }

  #[test]
  fn core_shortcuts_get_names_and_groups() {
    assert_eq!(ll_label("ws-4"), "Workspace 4");
    assert_eq!(ll_group("ws-4"), "ws");
    assert_eq!(ll_group("clipboard"), "sys");
    assert_eq!(ll_group("browser"), "app");
  }

  #[test]
  fn rows_put_the_fixed_ones_first() {
    let k = Keys { core: vec![serde_json::json!({ "id": "browser", "combo": "Super+B", "default": "Super+B" })], ..Default::default() };
    let rows = k.rows(&|s| s.to_string());
    assert_eq!(rows[0].key, "super");
    assert_eq!(rows[2].key, "ll:browser");
    assert!(rows[2].src == Src::Core);
  }
}
