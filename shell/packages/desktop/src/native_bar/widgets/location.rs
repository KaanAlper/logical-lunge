//! A location draft belongs to the picker, never to a provider refresh.
use super::layout::{Location, Spec};
use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field { Country, City, District }

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Country { pub code: String, pub name: String, pub english_name: String }

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
  pub name: String, pub label: String, pub country_code: String, pub country: String,
  pub city: String, pub district: String, pub latitude: f64, pub longitude: f64,
}

#[derive(Clone, Debug)]
pub enum Choice { Country(Country), Place(Place) }

pub struct Draft {
  pub text: [String; 3],
  pub country: Option<Country>,
  pub selected: Option<Location>,
  pub field: Field,
  pub generation: u64,
  pub due: Instant,
  pub inflight: Option<u64>,
  pub results: Vec<Choice>,
  pub error: Option<String>,
  pub recent: Vec<Location>,
  pending: bool,
}

impl Draft {
  pub fn new(spec: &Spec) -> Self {
    let mut draft = Self {
      text: [String::new(), spec.city.clone(), String::new()], country: None, selected: None,
      field: Field::Country, generation: 1, due: Instant::now(), inflight: None,
      results: Vec::new(), error: None, recent: spec.recent_locations.clone(), pending: true,
    };
    if let Some(location) = &spec.location { draft.restore(location.clone()); }
    draft
  }
  fn invalidate(&mut self) {
    self.generation += 1;
    self.results.clear(); self.error = None;
    self.due = Instant::now() + Duration::from_millis(350);
    self.pending = true;
  }
  pub fn focus(&mut self, field: Field) {
    if self.field != field { self.field = field; self.invalidate(); }
  }
  pub fn retry(&mut self) { self.invalidate(); self.due = Instant::now(); }
  pub fn change(&mut self, field: Field, text: String) {
    self.field = field;
    self.text[field as usize] = text;
    match field {
      Field::Country => {
        self.country = None; self.selected = None;
        self.text[1].clear(); self.text[2].clear();
      }
      Field::City => { self.selected = None; self.text[2].clear(); }
      Field::District => if let Some(location) = &mut self.selected {
        location.district.clear();
        location.latitude = location.city_latitude; location.longitude = location.city_longitude;
      },
    }
    self.invalidate();
  }
  pub fn restore(&mut self, location: Location) {
    if !location.valid() { return; }
    self.text = [location.country.clone(), location.city.clone(), location.district.clone()];
    self.country = Some(Country { code: location.country_code.clone(), name: location.country.clone(), english_name: String::new() });
    self.selected = Some(location);
    self.invalidate(); self.pending = false;
  }
  pub fn choose(&mut self, choice: Choice) -> bool {
    match choice {
      Choice::Country(mut country) if self.field == Field::Country && country.code.len() == 2 && country.code.bytes().all(|c| c.is_ascii_alphabetic()) && !country.name.trim().is_empty() => {
        country.code.make_ascii_uppercase();
        self.text = [country.name.clone(), String::new(), String::new()];
        self.country = Some(country); self.selected = None; self.field = Field::City;
      }
      Choice::Place(place) => {
        let Some(country) = &self.country else { return false };
        if !place.country_code.eq_ignore_ascii_case(&country.code) || !super::layout::valid_coords(place.latitude, place.longitude) { return false; }
        if self.field == Field::City {
          let city = if place.city.is_empty() { place.name } else { place.city };
          if city.trim().is_empty() { return false; }
          self.selected = Some(Location {
            country_code: country.code.clone(), country: country.name.clone(), city: city.clone(), district: String::new(),
            latitude: place.latitude, longitude: place.longitude, city_latitude: place.latitude, city_longitude: place.longitude,
          });
          self.text[1] = city; self.text[2].clear(); self.field = Field::District;
        } else if self.field == Field::District {
          let Some(location) = &mut self.selected else { return false };
          if !place.city.eq_ignore_ascii_case(&location.city) { return false; }
          let district = if place.district.is_empty() { place.name } else { place.district };
          if district.trim().is_empty() { return false; }
          location.district = district.clone(); location.latitude = place.latitude; location.longitude = place.longitude;
          self.text[2] = district;
        } else { return false; }
      }
      _ => return false,
    }
    self.invalidate(); self.pending = false; true
  }
  pub fn can_save(&self) -> bool {
    self.selected.as_ref().is_some_and(|p| p.valid() && self.text == [p.country.clone(), p.city.clone(), p.district.clone()])
  }
  pub fn request(&mut self, language: &str, now: Instant) -> Option<(u64, String)> {
    if !self.pending || self.inflight.is_some() || now < self.due { return None; }
    let query = self.text[self.field as usize].trim();
    if query.is_empty() && self.field != Field::Country { self.pending = false; return None; }
    let enc = super::weather::encode;
    let path = match self.field {
      Field::Country => format!("/widgets/countries?q={}&language={}", enc(query), enc(language)),
      Field::City | Field::District => {
        let country = self.country.as_ref()?;
        let mut path = format!("/widgets/places?q={}&countryCode={}&kind={}&language={}", enc(query), enc(&country.code),
          if self.field == Field::City { "city" } else { "district" }, enc(language));
        if self.field == Field::District {
          let city = self.selected.as_ref()?;
          path.push_str(&format!("&city={}&latitude={}&longitude={}", enc(&city.city), city.city_latitude, city.city_longitude));
        }
        path
      }
    };
    self.pending = false; self.inflight = Some(self.generation);
    Some((self.generation, path))
  }
  pub fn answer(&mut self, generation: u64, answer: Result<Vec<Choice>, String>) {
    if self.inflight != Some(generation) { return; }
    self.inflight = None;
    if generation != self.generation { return; }
    match answer { Ok(results) => self.results = results, Err(error) => self.error = Some(error) }
  }
}

pub fn fetch(path: &str) -> Result<Vec<Choice>, String> {
  let (status, body) = super::core_api::post_waiting(path, Duration::from_secs(18))
    .map_err(|e| e.to_string())?.ok_or("Bağlantı zaman aşımına uğradı")?;
  parse_answer(path, status, &body)
}

fn parse_answer(path: &str, status: u16, body: &[u8]) -> Result<Vec<Choice>, String> {
  let value: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
  if let Some(error) = value["error"].as_str() { return Err(error.into()); }
  if status != 200 { return Err(format!("HTTP {status}")); }
  let results = value["results"].as_array().ok_or("Geçersiz konum yanıtı")?;
  Ok(results.iter().filter_map(|v| {
    if path.starts_with("/widgets/countries") { serde_json::from_value(v.clone()).ok().map(Choice::Country) }
    else { serde_json::from_value(v.clone()).ok().map(Choice::Place) }
  }).take(12).collect())
}

#[cfg(test)]
mod tests {
  use super::*;
  fn place() -> Location {
    Location { country_code: "TR".into(), country: "Türkiye".into(), city: "İzmir".into(), district: "Konak".into(),
      latitude: 38.4, longitude: 27.1, city_latitude: 38.42, city_longitude: 27.14 }
  }
  #[test]
  fn saved_and_recent_locations_are_valid_offline_drafts() {
    let mut spec = Spec::default(); spec.save_location(place());
    let draft = Draft::new(&spec);
    assert_eq!(draft.text, ["Türkiye", "İzmir", "Konak"]);
    assert!(draft.can_save());
    let mut draft = Draft::new(&Spec::default());
    draft.restore(place()); assert!(draft.can_save());
  }
  #[test]
  fn typing_invalidates_selection_without_accepting_freeform() {
    let mut draft = Draft::new(&Spec::default()); draft.restore(place());
    draft.change(Field::City, "İzm".into());
    assert_eq!(draft.text[1], "İzm"); assert_eq!(draft.text[2], ""); assert!(!draft.can_save());
    draft.restore(place()); draft.change(Field::District, "".into());
    assert!(draft.can_save());
    let selected = draft.selected.as_ref().unwrap();
    assert_eq!((selected.latitude, selected.longitude), (38.42, 27.14));
  }
  #[test]
  fn one_request_at_a_time_discards_stale_answers_and_keeps_latest_text() {
    let mut draft = Draft::new(&Spec::default());
    draft.change(Field::Country, "T".into());
    let now = Instant::now() + Duration::from_secs(1);
    let (old, _) = draft.request("tr", now).unwrap();
    draft.change(Field::Country, "Tür".into());
    assert!(draft.request("tr", now).is_none());
    draft.answer(old, Err("offline".into()));
    assert_eq!(draft.text[0], "Tür"); assert!(draft.error.is_none());
    let (latest, path) = draft.request("tr", now).unwrap();
    assert!(path.contains("q=T%C3%BCr"));
    draft.answer(latest, Err("offline".into())); assert!(draft.error.is_some());
  }
  #[test]
  fn leaving_a_field_and_provider_answers_preserve_the_draft() {
    let mut draft = Draft::new(&Spec::default()); draft.restore(place());
    draft.change(Field::District, "Kar".into());
    let now = Instant::now() + Duration::from_secs(1);
    let (generation, _) = draft.request("tr", now).unwrap();
    // Blur never commits or tears down Draft; a later focus switch only cancels lookup.
    let text_at_blur = draft.text.clone();
    draft.focus(Field::City);
    draft.answer(generation, Ok(Vec::new()));
    assert_eq!(draft.text, text_at_blur);
    assert!(!draft.can_save());
  }
  #[test]
  fn search_errors_show_the_server_reason() {
    assert_eq!(parse_answer("/widgets/places", 200, br#"{"error":"Provider busy; retry"}"#).unwrap_err(), "Provider busy; retry");
    assert_eq!(parse_answer("/widgets/places", 503, br#"{"error":"Offline"}"#).unwrap_err(), "Offline");
  }
  #[test]
  fn district_queries_keep_city_coordinates_and_reject_wrong_scopes() {
    let mut draft = Draft::new(&Spec::default()); draft.restore(place());
    draft.change(Field::District, "K".into());
    let (_, query) = draft.request("tr", Instant::now() + Duration::from_secs(1)).unwrap();
    assert!(query.contains("latitude=38.42&longitude=27.14"));
    let mut district = Place { name: "Karşıyaka".into(), label: "Karşıyaka".into(), country_code: "DE".into(), country: "Germany".into(),
      city: "İzmir".into(), district: "Karşıyaka".into(), latitude: 38.45, longitude: 27.11 };
    assert!(!draft.choose(Choice::Place(district.clone())));
    district.country_code = "TR".into(); district.city = "Ankara".into();
    assert!(!draft.choose(Choice::Place(district.clone())));
    district.city = "İzmir".into(); assert!(draft.choose(Choice::Place(district)));
    let selected = draft.selected.as_ref().unwrap();
    assert_eq!((selected.latitude, selected.city_latitude), (38.45, 38.42));
    assert!(draft.can_save());
  }
  #[test]
  fn save_bounds_and_deduplicates_recents() {
    let mut spec = Spec::default();
    for index in 0..12 { let mut location = place(); location.district = format!("{index}"); spec.save_location(location); }
    assert_eq!(spec.recent_locations.len(), 8);
    let again = spec.recent_locations[3].clone(); spec.save_location(again.clone());
    assert_eq!(spec.recent_locations[0], again);
    assert_eq!(spec.recent_locations.len(), 8);
    spec.location = None; spec.city.clear();
    assert_eq!(spec.recent_locations.len(), 8);
  }
}
