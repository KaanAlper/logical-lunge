//! What the desktop widgets are and where they sit
//! (`%LOCALAPPDATA%\LogicalLunge\state\desktop-widgets.json`): each widget's
//! kind, its monitor (GDI device name, the primary one when it is gone),
//! its rectangle in DIPs from that monitor's work area and its settings.
//! Pure data: placing, snapping and keeping widgets on screen are tested
//! here without windows.

use serde::{Deserialize, Serialize};

/// Widgets move and resize in steps of this many DIPs.
pub const GRID: f32 = 8.0;
/// Space kept from the work area's edges for a new widget.
pub const MARGIN: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
  #[default]
  Clock,
  Media,
  System,
  Weather,
  Agenda,
  Note,
}

/// The order of the desktop menu's "Widget ekle" list.
pub const KINDS: [Kind; 6] = [Kind::Clock, Kind::Media, Kind::System, Kind::Weather, Kind::Agenda, Kind::Note];

impl Kind {
  /// The Turkish source text (`tr()` translates it).
  pub fn label(self) -> &'static str {
    match self {
      Kind::Clock => "Saat",
      Kind::Media => "Medya",
      Kind::System => "Sistem",
      Kind::Weather => "Hava durumu",
      Kind::Agenda => "Ajanda",
      Kind::Note => "Not",
    }
  }

  pub fn icon(self) -> &'static str {
    match self {
      Kind::Clock => "schedule",
      Kind::Media => "music_note",
      Kind::System => "memory",
      Kind::Weather => "partly_cloudy_day",
      Kind::Agenda => "event",
      Kind::Note => "sticky_note_2",
    }
  }

  pub fn id(self) -> &'static str {
    match self {
      Kind::Clock => "clock",
      Kind::Media => "media",
      Kind::System => "system",
      Kind::Weather => "weather",
      Kind::Agenda => "agenda",
      Kind::Note => "note",
    }
  }

  pub fn from_id(id: &str) -> Option<Kind> {
    KINDS.into_iter().find(|k| k.id() == id)
  }

  /// A new widget's size (DIP).
  pub fn default_size(self) -> (f32, f32) {
    match self {
      Kind::Clock => (264.0, 120.0),
      Kind::Media => (336.0, 112.0),
      Kind::System => (288.0, 128.0),
      Kind::Weather => (248.0, 128.0),
      Kind::Agenda => (264.0, 232.0),
      Kind::Note => (248.0, 200.0),
    }
  }

  /// The smallest size its contents still fit in (DIP).
  pub fn min_size(self) -> (f32, f32) {
    match self {
      Kind::Clock => (136.0, 72.0),
      Kind::Media => (240.0, 96.0),
      Kind::System => (200.0, 96.0),
      Kind::Weather => (176.0, 96.0),
      Kind::Agenda => (176.0, 136.0),
      Kind::Note => (144.0, 96.0),
    }
  }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClockStyle {
  #[default]
  Digital,
  Large,
  Analog,
}

pub const APPEARANCES: [(&str, &str); 8] = [
  ("standard", "Standart"), ("transparent", "Şeffaf"), ("outline", "Konturlu şeffaf"),
  ("glass", "Cam"), ("futuristic", "Fütüristik"), ("cartoon", "Çizgi film"),
  ("paper", "Kâğıt"), ("pixel", "Piksel"),
];

fn appearance<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
  let value = serde_json::Value::deserialize(d)?;
  Ok(value.as_str().filter(|id| APPEARANCES.iter().any(|(a, _)| a == id)).unwrap_or("standard").into())
}

fn opacity<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
  let value = serde_json::Value::deserialize(d)?;
  Ok(value.as_f64().filter(|n| n.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0) as f32)
}

fn shape<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
  let value = serde_json::Value::deserialize(d)?;
  Ok(value.as_str().filter(|id| super::shape::SHAPES.iter().any(|(shape,_)| shape == id)).unwrap_or("card").into())
}

fn shape_sizes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<std::collections::BTreeMap<String,(f32,f32)>, D::Error> {
  let value = serde_json::Value::deserialize(d)?;
  let mut sizes = std::collections::BTreeMap::new();
  for (form,_) in super::shape::SHAPES {
    if let Ok((w,h)) = serde_json::from_value::<(f32,f32)>(value[form].clone()) {
      if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 && w <= 10000.0 && h <= 10000.0 { sizes.insert(form.into(),(w,h)); }
    }
  }
  Ok(sizes)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
  pub country_code: String,
  pub country: String,
  pub city: String,
  pub district: String,
  pub latitude: f64,
  pub longitude: f64,
  pub city_latitude: f64,
  pub city_longitude: f64,
}

impl Location {
  pub fn valid(&self) -> bool {
    self.country_code.len() == 2 && self.country_code.bytes().all(|c| c.is_ascii_alphabetic())
      && !self.country.trim().is_empty() && !self.city.trim().is_empty()
      && valid_coords(self.latitude, self.longitude) && valid_coords(self.city_latitude, self.city_longitude)
  }

  pub fn display(&self) -> String {
    if self.district.is_empty() { self.city.clone() } else { format!("{}, {}", self.district, self.city) }
  }
}

pub fn valid_coords(lat: f64, lon: f64) -> bool {
  lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}

fn location<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Location>, D::Error> {
  let value = serde_json::Value::deserialize(d)?;
  Ok(serde_json::from_value::<Location>(value).ok().filter(Location::valid))
}

fn recents<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Location>, D::Error> {
  let value = serde_json::Value::deserialize(d)?;
  let mut out = Vec::new();
  for value in value.as_array().into_iter().flatten() {
    if let Ok(location) = serde_json::from_value::<Location>(value.clone()) {
      if location.valid() && !out.contains(&location) { out.push(location); }
    }
    if out.len() == 8 { break; }
  }
  Ok(out)
}

/// One widget as it is saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Spec {
  pub id: u64,
  pub kind: Kind,
  /// `\\.\DISPLAY1` (empty: the primary monitor)
  pub monitor: String,
  pub x: f32,
  pub y: f32,
  pub w: f32,
  pub h: f32,
  // clock
  pub clock: ClockStyle,
  pub seconds: bool,
  pub date: bool,
  // system
  pub temps: bool,
  // weather: a place name (empty: the city of the Windows time zone)
  pub city: String,
  pub fahrenheit: bool,
  #[serde(deserialize_with = "appearance")]
  pub appearance: String,
  #[serde(deserialize_with = "shape")]
  pub shape: String,
  /// Each silhouette keeps its own size; changing shapes is not a resize.
  #[serde(deserialize_with = "shape_sizes")]
  pub shape_sizes: std::collections::BTreeMap<String,(f32,f32)>,
  #[serde(deserialize_with = "opacity")]
  pub background_opacity: f32,
  #[serde(deserialize_with = "opacity")]
  pub content_opacity: f32,
  #[serde(deserialize_with = "location")]
  pub location: Option<Location>,
  #[serde(deserialize_with = "recents")]
  pub recent_locations: Vec<Location>,
  // note
  pub note: String,
}

impl Default for Spec {
  fn default() -> Self {
    Spec {
      id: 0,
      kind: Kind::Clock,
      monitor: String::new(),
      x: MARGIN,
      y: MARGIN,
      w: 0.0,
      h: 0.0,
      clock: ClockStyle::Digital,
      seconds: false,
      date: true,
      temps: true,
      city: String::new(),
      fahrenheit: false,
      appearance: "standard".into(),
      shape: "card".into(),
      shape_sizes: std::collections::BTreeMap::new(),
      background_opacity: 1.0,
      content_opacity: 1.0,
      location: None,
      recent_locations: Vec::new(),
      note: String::new(),
    }
  }
}

impl Spec {
  pub fn new(id: u64, kind: Kind, monitor: &str) -> Spec {
    let (w, h) = kind.default_size();
    Spec { id, kind, monitor: monitor.to_string(), w, h, ..Default::default() }
  }

  pub fn rect(&self) -> (f32, f32, f32, f32) {
    (self.x, self.y, self.w, self.h)
  }

  pub fn select_shape(&mut self, form: &str, aw: f32, ah: f32) {
    let form = super::shape::SHAPES.iter().find(|(id,_)| *id == form).map(|(id,_)| *id).unwrap_or("card");
    if self.shape == form { return; }
    self.shape_sizes.insert(self.shape.clone(),(self.w,self.h));
    let (w,h) = self.shape_sizes.get(form).copied()
      .unwrap_or_else(|| super::shape::preferred(self.kind,form));
    self.shape = form.into();
    (self.x,self.y,self.w,self.h) = super::shape::clamp(self.kind,form,(self.x,self.y,w,h),aw,ah);
  }

  pub fn save_location(&mut self, location: Location) {
    if !location.valid() { return; }
    self.city = location.display();
    self.recent_locations.retain(|r| r != &location);
    self.recent_locations.insert(0, location.clone());
    self.recent_locations.truncate(8);
    self.location = Some(location);
  }

  /// Ticks once a second (seconds on a clock face).
  pub fn per_second(&self) -> bool {
    self.kind == Kind::Clock && (self.seconds || self.clock == ClockStyle::Analog)
  }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
  pub version: u32,
  pub widgets: Vec<Spec>,
}

impl Store {
  pub fn parse(text: &str) -> Option<Store> {
    serde_json::from_str::<Store>(text).ok().map(|mut s| {
      // an id twice (a hand-edited file) would make two windows one widget
      let mut seen = std::collections::HashSet::new();
      s.widgets.retain(|w| w.id != 0 && seen.insert(w.id));
      s
    })
  }

  pub fn to_json(&self) -> String {
    serde_json::to_string_pretty(&Store { version: 1, widgets: self.widgets.clone() }).unwrap_or_default()
  }

  pub fn next_id(&self) -> u64 {
    self.widgets.iter().map(|w| w.id).max().unwrap_or(0) + 1
  }
}

pub fn snap(v: f32) -> f32 {
  (v / GRID).round() * GRID
}

/// Keeps a rectangle (DIP, from the work area's top-left) inside a work
/// area of `area_w` x `area_h`: at least its kind's smallest size, at most
/// the area, wholly on screen.
pub fn clamp(kind: Kind, r: (f32, f32, f32, f32), area_w: f32, area_h: f32) -> (f32, f32, f32, f32) {
  let (min_w, min_h) = kind.min_size();
  let w = r.2.max(min_w).min(area_w.max(min_w));
  let h = r.3.max(min_h).min(area_h.max(min_h));
  let x = r.0.min(area_w - w).max(0.0);
  let y = r.1.min(area_h - h).max(0.0);
  (x, y, w, h)
}

fn overlaps(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
  a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Where a new widget of `size` goes on a work area: the first free spot
/// on the grid from the top right corner downwards, column by column
/// (desktop icons fill the left side first); the top right corner when
/// nothing is free.
pub fn free_spot(size: (f32, f32), taken: &[(f32, f32, f32, f32)], area_w: f32, area_h: f32) -> (f32, f32) {
  let (w, h) = size;
  let right = snap((area_w - MARGIN - w).max(0.0));
  let gap = GRID * 2.0;
  let mut x = right;
  while x >= 0.0 {
    let mut y = MARGIN;
    while y + h <= area_h {
      // a little room between widgets
      let grown = (x - gap / 2.0, y - gap / 2.0, w + gap, h + gap);
      if !taken.iter().any(|t| overlaps(grown, *t)) {
        return (x, snap(y));
      }
      y += GRID * 2.0;
    }
    x -= GRID * 4.0;
  }
  (right, MARGIN)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn snaps_to_the_grid() {
    assert_eq!(snap(13.0), 16.0);
    assert_eq!(snap(11.9), 8.0);
    assert_eq!(snap(-3.0), 0.0);
  }

  #[test]
  fn clamps_into_the_work_area() {
    // off the right and bottom edges: pushed back, size kept
    assert_eq!(clamp(Kind::Clock, (1900.0, 1000.0, 264.0, 120.0), 1920.0, 1040.0), (1656.0, 920.0, 264.0, 120.0));
    // off the left / top
    assert_eq!(clamp(Kind::Clock, (-50.0, -10.0, 264.0, 120.0), 1920.0, 1040.0), (0.0, 0.0, 264.0, 120.0));
    // too small: its kind's smallest size
    assert_eq!(clamp(Kind::Media, (10.0, 10.0, 20.0, 20.0), 1920.0, 1040.0), (10.0, 10.0, 240.0, 96.0));
    // larger than a small monitor: the monitor's size
    assert_eq!(clamp(Kind::Note, (0.0, 0.0, 5000.0, 5000.0), 800.0, 600.0), (0.0, 0.0, 800.0, 600.0));
  }

  #[test]
  fn new_widgets_do_not_overlap() {
    let area = (1920.0, 1040.0);
    let a = free_spot((264.0, 120.0), &[], area.0, area.1);
    assert_eq!(a, (1632.0, MARGIN));
    let first = (a.0, a.1, 264.0, 120.0);
    let b = free_spot((264.0, 120.0), &[first], area.0, area.1);
    assert!(!overlaps((b.0, b.1, 264.0, 120.0), first), "{:?} overlaps {:?}", b, first);
    assert_eq!(b.0, a.0, "same column, lower");
    assert!(b.1 > a.1);
  }

  #[test]
  fn a_full_area_still_gives_a_spot() {
    let all = (0.0, 0.0, 400.0, 300.0);
    assert_eq!(free_spot((264.0, 120.0), &[all], 400.0, 300.0), (snap(400.0 - MARGIN - 264.0), MARGIN));
  }

  #[test]
  fn round_trips_and_drops_doubled_ids() {
    let mut s = Store::default();
    s.widgets.push(Spec::new(1, Kind::Weather, r"\\.\DISPLAY2"));
    let mut note = Spec::new(2, Kind::Note, "");
    note.note = "süt al\nekmek".into();
    s.widgets.push(note);
    let back = Store::parse(&s.to_json()).unwrap();
    assert_eq!(back.widgets, s.widgets);
    assert_eq!(back.version, 1);
    assert_eq!(back.next_id(), 3);
    let doubled = r#"{"widgets":[{"id":4,"kind":"note"},{"id":4,"kind":"clock"},{"id":0,"kind":"media"}]}"#;
    let d = Store::parse(doubled).unwrap();
    assert_eq!(d.widgets.len(), 1);
    assert_eq!(d.widgets[0].kind, Kind::Note);
  }

  #[test]
  fn reads_files_with_missing_or_unknown_fields() {
    let s = Store::parse(r#"{"widgets":[{"id":7,"kind":"clock","x":40,"future":true}]}"#).unwrap();
    assert_eq!(s.widgets[0].x, 40.0);
    assert!(s.widgets[0].date, "defaults fill the rest");
    assert!(Store::parse("not json").is_none());
  }

  #[test]
  fn kinds_round_trip_their_ids() {
    for k in KINDS {
      assert_eq!(Kind::from_id(k.id()), Some(k));
    }
    assert_eq!(Kind::from_id("nope"), None);
  }

  #[test]
  fn appearance_defaults_and_unknown_styles_do_not_lose_the_store() {
    let store = Store::parse(r#"{"widgets":[{"id":1,"appearance":"future-unknown","backgroundOpacity":-2,"contentOpacity":4}]}"#).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&store.to_json()).unwrap();
    assert_eq!(saved["widgets"][0]["appearance"], "standard");
    assert_eq!(saved["widgets"][0]["backgroundOpacity"].as_f64(), Some(0.0));
    assert_eq!(saved["widgets"][0]["contentOpacity"].as_f64(), Some(1.0));
    let old = Store::parse(r#"{"widgets":[{"id":2,"city":"İzmir"}]}"#).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&old.to_json()).unwrap();
    assert_eq!(saved["widgets"][0]["backgroundOpacity"].as_f64(), Some(1.0));
    assert_eq!(saved["widgets"][0]["contentOpacity"].as_f64(), Some(1.0));
    assert_eq!(saved["widgets"][0]["location"], serde_json::Value::Null);
    assert_eq!(saved["widgets"][0]["city"], "İzmir");
  }

  #[test]
  fn invalid_coordinates_clear_only_the_location() {
    let store = Store::parse(r#"{"widgets":[{"id":1,"kind":"weather","location":{"countryCode":"TR","country":"Türkiye","city":"İzmir","district":"","latitude":null,"longitude":27,"cityLatitude":38,"cityLongitude":27}}]}"#).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&store.to_json()).unwrap();
    assert!(saved["widgets"][0]["location"].is_null());
    assert_eq!(store.widgets.len(), 1);
  }

  #[test]
  fn shapes_round_trip_independently_and_unknown_shapes_fall_back() {
    let store = Store::parse(r#"{"widgets":[{"id":1,"shape":"circle","appearance":"paper","backgroundOpacity":0.25,"contentOpacity":0.75,"city":"Berlin"},{"id":2,"shape":"future-shape"},{"id":3}]}"#).unwrap();
    let json: serde_json::Value = serde_json::from_str(&store.to_json()).unwrap();
    assert_eq!(json["widgets"][0]["shape"], "circle");
    assert_eq!(json["widgets"][0]["appearance"], "paper");
    assert_eq!(json["widgets"][0]["contentOpacity"].as_f64(), Some(0.75));
    assert_eq!(json["widgets"][0]["city"], "Berlin");
    assert_eq!(json["widgets"][1]["shape"], "card");
    assert_eq!(json["widgets"][2]["shape"], "card");
  }
}
