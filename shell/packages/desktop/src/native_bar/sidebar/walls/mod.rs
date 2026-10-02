//! The wallpaper page (sidebar.html WallpaperPage, LiveSection,
//! ScreenSaverSection), split into tabs so one gallery shows at a time:
//! - Duvar kâğıdı: the monitors, wallhaven.cc's suggestions by category and
//!   the library ("Kütüphanem"), Superscreen across monitors;
//! - Canlı: the video library, import, pause rules;
//! - Ekran koruyucu: Windows' settings, the screen savers and our video
//!   screen saver's videos (the same video library);
//! - Mağaza: Sucrose Store's videos (moving preview on hover), one store
//!   for both: an item becomes a live wallpaper or the screen saver.
//! Every tab is the same parts: a category row that scrolls sideways
//! (`chip_row`) and one gallery (`gallery`); every item has our right-click
//! menu and, on hover, its main actions.
//!
//! The core does the work (`--wall-*`, `--live-*`, `--saver-*`,
//! `/library-remove`); Windows' screen saver settings are the shell's
//! (`screensaver.rs`).

mod actions;
mod paint;

pub(in crate::native_bar::sidebar) use paint::paint;

use std::{collections::HashMap, time::Instant};

use serde_json::{json, Value};
use windows::{core::Interface, Win32::Graphics::Direct2D::ID2D1Image};

use super::{
  super::{
    core_api,
    gfx::{Rect, Rgba},
    menu::Item as MenuItem,
    model::Model,
    send, Msg, Ui,
  },
  images::{frame_now, Img},
  kit::{st, stw, Cx},
  store::Store,
  text::{TextField, Typed},
  Ev, FieldId, Hit, ScrollId, Sidebar,
};

const TABS: [(&str, &str); 4] = [("Duvar kâğıdı", "wallpaper"), ("Canlı", "motion_photos_on"), ("Ekran koruyucu", "ambient_screen"), ("Mağaza", "storefront")];
pub(in crate::native_bar::sidebar) const TAB_WALL: usize = 0;
pub(in crate::native_bar::sidebar) const TAB_LIVE: usize = 1;
pub(in crate::native_bar::sidebar) const TAB_SAVER: usize = 2;
pub(in crate::native_bar::sidebar) const TAB_STORE: usize = 3;
/// the scrolling category rows
const ROW_WALL: u8 = 1;
const ROW_STORE: u8 = 2;
const WALL_CATS: [(&str, &str, &str); 6] = [
  ("anime", "Anime", "animation"),
  ("nature", "Doğa", "forest"),
  ("space", "Uzay", "rocket"),
  ("city", "Şehir", "location_city"),
  ("minimal", "Minimal", "crop_square"),
  ("local", "Kütüphanem", "photo_library"),
];
const SAVER_SUBS: [(&str, &str); 2] = [("Ekran koruyucular", "ambient_screen"), ("Videolar", "video_library")];

/// The galleries of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::native_bar) enum G {
  Wall,
  Span,
  Live,
  Store,
  Savers,
  Videos,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum WHit {
  Tab(usize),
  Sub(usize),
  Mon(String),
  Target(String),
  Cat(&'static str),
  StoreCat(String),
  Tile(G, String),
  /// a hover action of an item: (gallery, item, menu id)
  TileAct(G, String, &'static str),
  /// an arrow of a category row: (row, direction)
  RowStep(u8, i8),
  Custom(G),
  LiveClear,
  LiveOpt(bool),
  SaverEnabled,
  SaverSecure,
  SaverPreview,
  SaverOptions,
  Shuffle,
  Confirm(bool),
}

pub(in crate::native_bar) enum WEv {
  Info(Value),
  Items(String, Vec<Tile>),
  Live(Vec<Tile>),
  StoreCats(Option<Vec<String>>),
  Store(String, Vec<Tile>),
  /// a long job ended: (ok, message, reload)
  Done(bool, String, bool),
  Progress(String),
  Saver(Result<Value, String>),
  SaverIcons(Value),
  Videos(Value),
  Removed(G, String, bool),
}

#[derive(Clone, Debug, Default)]
pub(in crate::native_bar) struct Tile {
  pub key: String,
  /// picture: a URL, a library file or a data URL
  pub thumb: String,
  /// a moving preview (store)
  pub preview: String,
  pub name: String,
  pub res: String,
  /// a library file
  pub path: String,
  /// wallhaven's full picture
  pub full: String,
  /// store category and id
  pub cat: String,
  pub id: String,
}

struct Confirm {
  text: String,
  g: G,
  path: String,
}

#[derive(Default)]
pub(super) struct Walls {
  pub tab: usize,
  pub sub: usize,
  /// a category row whose selected chip must be brought into view
  reveal: Option<u8>,
  info: Option<Value>,
  target: String,
  cat: String,
  items: HashMap<String, Option<Vec<Tile>>>,
  live: Option<Vec<Tile>>,
  store_cats: Option<Vec<String>>,
  store_cat: String,
  store: HashMap<String, Option<Vec<Tile>>>,
  busy: Option<String>,
  progress: String,
  msg: Option<(bool, String)>,
  saver: Option<Value>,
  saver_err: String,
  saving: bool,
  icons: Value,
  videos: Value,
  confirm: Option<Confirm>,
  /// the tile under the pointer and since when (moving previews start over)
  hover: Option<(String, Instant)>,
  store_failed: bool,
  /// the moving preview last shown (its frames go when the pointer leaves)
  pub last_gif: Option<String>,
}

fn s(v: &Value) -> &str {
  v.as_str().unwrap_or("")
}

/// Percent-encodes a query value.
pub(super) fn enc(v: &str) -> String {
  let mut out = String::new();
  for b in v.bytes() {
    if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

fn live_cat_name(id: &str) -> &str {
  match id {
    "Anime" => "Anime",
    "Game" => "Oyun",
    "Vehicles" => "Taşıtlar",
    "Landscape" => "Manzara",
    "Lifestyle" => "Yaşam tarzı",
    "Fantasy" => "Fantastik",
    "Film and TV" => "Film ve dizi",
    "Abstract" => "Soyut",
    "Animals" => "Hayvanlar",
    "Nature" => "Doğa",
    "Science Fiction" => "Bilim kurgu",
    "Retro" => "Retro",
    "Ambience" => "Atmosfer",
    "Galaxy" => "Galaksi",
    "Space" => "Uzay",
    "Technology" => "Teknoloji",
    other => other,
  }
}

/// The core's error codes; network errors come with Windows' own text.
fn live_error(e: &str, download: bool) -> &'static str {
  match e {
    "type" => "Bu duvar kağıdı video değil",
    "size" => "Video çok büyük",
    "decode" => "Bu video oynatılamıyor",
    _ if download => "İndirilemedi",
    _ => "Canlı duvar kağıdı ayarlanamadı",
  }
}

impl Walls {
  pub fn new(store: &Store) -> Self {
    Walls {
      tab: store.wall_page_tab.min(TAB_STORE),
      reveal: Some(ROW_WALL),
      target: "all".into(),
      cat: store.wall_cat.clone().unwrap_or_else(|| "anime".into()),
      store_cat: store.live_cat.clone().filter(|c| c != "local").unwrap_or_else(|| "Anime".into()),
      icons: Value::Null,
      videos: Value::Null,
      ..Default::default()
    }
  }

  fn monitors(&self) -> Vec<Value> {
    self.info.as_ref().and_then(|i| i["monitors"].as_array().cloned()).unwrap_or_default()
  }

  fn live_on(&self) -> bool {
    self.monitors().iter().any(|m| !s(&m["live"]).is_empty() && (self.target == "all" || s(&m["id"]) == self.target))
  }

  fn tiles(&self, g: G) -> Option<Vec<Tile>> {
    match g {
      G::Wall => self.items.get(&self.cat).cloned().flatten(),
      G::Span => self.items.get("span").cloned().flatten(),
      G::Live | G::Videos => self.live.clone(),
      G::Store => self.store.get(&self.store_cat).cloned().flatten(),
      G::Savers => self.saver.as_ref().map(|st| {
        st["choices"]
          .as_array()
          .map(|a| {
            a.iter()
              .map(|c| {
                let path = s(&c["path"]).to_string();
                Tile { key: path.clone(), thumb: s(&self.icons[path.to_lowercase()]).to_string(), name: s(&c["name"]).to_string(), path, ..Default::default() }
              })
              .collect()
          })
          .unwrap_or_default()
      }),
    }
  }

  /// The actions shown on an item under the pointer (the menu's main ones:
  /// (icon, menu id, label)).
  pub(super) fn tile_actions(g: G) -> &'static [(&'static str, &'static str, &'static str)] {
    match g {
      G::Store => &[("motion_photos_on", "apply", "Duvar kâğıdı yap"), ("ambient_screen", "saver", "Ekran koruyucu yap")],
      G::Live => &[("ambient_screen", "saver", "Ekran koruyucu yap")],
      _ => &[],
    }
  }

  fn tile(&self, g: G, key: &str) -> Option<Tile> {
    self.tiles(g)?.into_iter().find(|t| t.key == key)
  }

  fn in_videos(&self, path: &str) -> bool {
    self.videos["videos"].as_array().is_some_and(|a| a.iter().any(|v| s(v).eq_ignore_ascii_case(path)))
  }
}

// ------------------------------------------------------------------ paint


// ------------------------------------------------------------------ workers

fn ev(e: WEv) {
  send(Msg::Sidebar(Ev::Walls(e)));
}

fn job(f: impl FnOnce() -> WEv + Send + 'static) {
  std::thread::spawn(move || ev(f()));
}

fn core_json(args: &[&str]) -> Value {
  core_api::run_core_output(args).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn load_info() {
  job(|| WEv::Info(core_json(&["--wall-info"])));
}

fn load_cat(cat: String) {
  job(move || {
    let list = if cat == "local" {
      core_json(&["--wall-local"])
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(60)
        .map(|f| Tile { key: s(&f["path"]).into(), thumb: s(&f["path"]).into(), path: s(&f["path"]).into(), name: s(&f["name"]).into(), ..Default::default() })
        .collect()
    } else {
      core_json(&["--wall-browse", &cat])
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|x| {
          let id = match &x["id"] {
            Value::String(s) => s.clone(),
            v => v.to_string(),
          };
          Tile { key: id.clone(), id, thumb: s(&x["thumb"]).into(), full: s(&x["full"]).into(), res: s(&x["res"]).into(), ..Default::default() }
        })
        .collect()
    };
    WEv::Items(cat, list)
  });
}

fn load_live() {
  job(|| {
    let list = core_json(&["--live-local"])
      .as_array()
      .cloned()
      .unwrap_or_default()
      .iter()
      .map(|f| {
        let name = if s(&f["author"]).is_empty() { s(&f["name"]).to_string() } else { format!("{} · {}", s(&f["name"]), s(&f["author"])) };
        Tile { key: s(&f["path"]).into(), path: s(&f["path"]).into(), thumb: s(&f["thumb"]).into(), name, ..Default::default() }
      })
      .collect();
    WEv::Live(list)
  });
}

fn load_store_cats() {
  job(|| {
    let v = core_json(&["--live-store"]);
    WEv::StoreCats(v.as_array().map(|a| a.iter().map(|c| s(&c["id"]).to_string()).filter(|c| !c.is_empty()).collect()))
  });
}

fn load_store(cat: String) {
  job(move || {
    let list = core_json(&["--live-store", &cat])
      .as_array()
      .cloned()
      .unwrap_or_default()
      .iter()
      .map(|x| {
        let id = s(&x["id"]).to_string();
        Tile {
          key: format!("{cat}/{id}"),
          id,
          cat: cat.clone(),
          name: s(&x["title"]).into(),
          thumb: s(&x["cover"]).into(),
          preview: s(&x["preview"]).into(),
          ..Default::default()
        }
      })
      .collect();
    WEv::Store(cat, list)
  });
}

fn load_saver() {
  job(|| WEv::Saver(crate::screensaver::state().and_then(|s| serde_json::to_value(s).map_err(|e| e.to_string()))));
  job(|| WEv::SaverIcons(core_json(&["--saver-icons"])));
  job(|| WEv::Videos(core_json(&["--saver-videos"])));
}


fn file_name(path: &str) -> String {
  std::path::Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
}

/// A screen saver imported into our library (never one of Windows' own).
fn is_imported(path: &str) -> bool {
  let Some(base) = std::env::var_os("LOCALAPPDATA") else { return false };
  let lib = std::path::Path::new(&base).join("LogicalLunge").join("screensavers");
  let lib = lib.to_string_lossy().to_lowercase();
  path.to_lowercase().starts_with(&format!("{}\\", lib.trim_end_matches('\\')))
}

#[cfg(test)]
mod tests {
  use super::*;
  use super::paint::tile_size;

  #[test]
  fn query_values_are_encoded() {
    assert_eq!(enc(r"C:\Users\a b\ş.png"), "C%3A%5CUsers%5Ca%20b%5C%C5%9F.png");
    assert_eq!(enc("x&path=y"), "x%26path%3Dy");
  }

  #[test]
  fn tiles_fit_the_page() {
    let (w, h, cols) = tile_size(G::Wall, 410.0);
    assert_eq!(cols, 2);
    assert_eq!(w, 201.0);
    assert_eq!(h, 113.0);
    assert_eq!(tile_size(G::Savers, 410.0).2, 3);
  }

  #[test]
  fn store_categories_are_named() {
    assert_eq!(live_cat_name("Film and TV"), "Film ve dizi");
    assert_eq!(live_cat_name("Unknown"), "Unknown");
  }
}
