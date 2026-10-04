//! The weather widget's data via the shared core API (Open-Meteo).
//! A saved location supplies coordinates; a legacy city is still supported.
//! With no name the city of
//! the Windows time zone is used (ICU, part of Windows 10 1903 and later,
//! turns "Turkey Standard Time" into "Europe/Istanbul"). Runs on a worker
//! thread; the UI gets one `Report`.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Report {
  pub place: String,
  pub temp: f32,
  pub high: f32,
  pub low: f32,
  pub code: u32,
  pub day: bool,
  pub fahrenheit: bool,
}

/// The complete request identity also guards delayed weather answers.
pub fn query(spec: &super::layout::Spec, language: &str) -> String {
  let city = if spec.city.trim().is_empty() { zone_city().unwrap_or_default() } else { spec.city.clone() };
  let mut path = format!("/widgets/weather?city={}&language={}&fahrenheit={}", encode(&city), encode(language), u8::from(spec.fahrenheit));
  if let Some(location) = spec.location.as_ref().filter(|l| l.valid()) {
    path.push_str(&format!("&latitude={}&longitude={}&place={}", location.latitude, location.longitude, encode(&location.display())));
  }
  path
}

pub fn fetch_query(path: &str) -> Result<Report, String> {
  let (status, body) = super::core_api::post_waiting(path, std::time::Duration::from_secs(18))
    .map_err(|e| e.to_string())?.ok_or("Hava durumu bağlantısı zaman aşımına uğradı")?;
  if status != 200 { return Err(format!("HTTP {status}")); }
  let value: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
  if let Some(error) = value["error"].as_str() { return Err(error.into()); }
  serde_json::from_value(value).map_err(|e| e.to_string())
}

pub fn request_uri(identity: &str, refresh: bool) -> String {
  if refresh { format!("{identity}&refresh=1") } else { identity.into() }
}

/// What the sky's WMO code looks like: an icon of our symbol font and the
/// Turkish source text.
pub fn describe(code: u32, day: bool) -> (&'static str, &'static str) {
  match code {
    0 => (if day { "sunny" } else { "bedtime" }, "Açık hava"),
    1 | 2 => (if day { "partly_cloudy_day" } else { "partly_cloudy_night" }, "Parçalı bulutlu"),
    3 => ("cloud", "Bulutlu"),
    45 | 48 => ("foggy", "Sisli"),
    51..=57 => ("rainy", "Çisenti"),
    61..=67 | 80..=82 => ("rainy", "Yağmurlu"),
    71..=77 | 85 | 86 => ("weather_snowy", "Karlı"),
    95..=99 => ("thunderstorm", "Gök gürültülü"),
    _ => ("cloud", "Bulutlu"),
  }
}

/// A query parameter (UTF-8, percent-encoded).
pub fn encode(s: &str) -> String {
  s.bytes()
    .map(|b| match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
      _ => format!("%{:02X}", b),
    })
    .collect()
}

/// "Europe/Istanbul" -> "Istanbul", "America/Argentina/Buenos_Aires" -> "Buenos Aires"
pub fn city_of_zone(zone: &str) -> Option<String> {
  let last = zone.rsplit('/').next()?.replace('_', " ");
  (zone.contains('/') && !last.is_empty() && !last.starts_with("GMT")).then_some(last)
}

/// The first place of a geocoding answer: (latitude, longitude, name).
#[cfg(test)]
pub fn parse_place(v: &Value) -> Option<(f64, f64, String)> {
  let r = v["results"].get(0)?;
  Some((r["latitude"].as_f64()?, r["longitude"].as_f64()?, r["name"].as_str().unwrap_or("").to_string()))
}

#[cfg(test)]
pub fn parse_forecast(v: &Value, place: &str, fahrenheit: bool) -> Option<Report> {
  let c = &v["current"];
  let d = &v["daily"];
  Some(Report {
    place: place.to_string(),
    temp: c["temperature_2m"].as_f64()? as f32,
    code: c["weather_code"].as_u64().unwrap_or(3) as u32,
    day: c["is_day"].as_u64().unwrap_or(1) == 1,
    high: d["temperature_2m_max"].get(0).and_then(Value::as_f64).unwrap_or(f64::NAN) as f32,
    low: d["temperature_2m_min"].get(0).and_then(Value::as_f64).unwrap_or(f64::NAN) as f32,
    fahrenheit,
  })
}

/// The IANA city of the Windows time zone (ICU's mapping).
#[cfg(windows)]
fn zone_city() -> Option<String> {
  use windows::{
    core::{s, w},
    Win32::Foundation::FreeLibrary,
    Win32::System::{
      LibraryLoader::{GetProcAddress, LoadLibraryW},
      Time::{GetDynamicTimeZoneInformation, DYNAMIC_TIME_ZONE_INFORMATION},
    },
  };
  type ForWindowsId = unsafe extern "C" fn(*const u16, i32, *const u8, *mut u16, i32, *mut i32) -> i32;
  unsafe {
    let mut tz = DYNAMIC_TIME_ZONE_INFORMATION::default();
    GetDynamicTimeZoneInformation(&mut tz);
    let key: Vec<u16> = tz.TimeZoneKeyName.iter().copied().take_while(|&c| c != 0).collect();
    if key.is_empty() {
      return None;
    }
    let icu = LoadLibraryW(w!("icu.dll")).ok()?;
    let Some(f) = GetProcAddress(icu, s!("ucal_getTimeZoneIDForWindowsID")) else { let _ = FreeLibrary(icu); return None; };
    let f: ForWindowsId = std::mem::transmute(f);
    let mut out = [0u16; 64];
    let mut status = 0i32;
    let n = f(key.as_ptr(), key.len() as i32, std::ptr::null(), out.as_mut_ptr(), out.len() as i32, &mut status);
    let _ = FreeLibrary(icu);
    if status > 0 || n <= 0 {
      return None;
    }
    city_of_zone(&String::from_utf16_lossy(&out[..n as usize]))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn zone_names_become_cities() {
    assert_eq!(city_of_zone("Europe/Istanbul").as_deref(), Some("Istanbul"));
    assert_eq!(city_of_zone("America/Argentina/Buenos_Aires").as_deref(), Some("Buenos Aires"));
    assert_eq!(city_of_zone("Etc/GMT+3"), None);
    assert_eq!(city_of_zone("UTC"), None);
  }

  #[test]
  fn encodes_place_names() {
    assert_eq!(encode("İzmir"), "%C4%B0zmir");
    assert_eq!(encode("New York"), "New%20York");
  }

  #[test]
  fn reads_open_meteo_answers() {
    let geo: Value = serde_json::from_str(r#"{"results":[{"name":"İstanbul","latitude":41.01,"longitude":28.95}]}"#).unwrap();
    assert_eq!(parse_place(&geo), Some((41.01, 28.95, "İstanbul".to_string())));
    assert_eq!(parse_place(&serde_json::json!({"generationtime_ms":0.1})), None);
    let f: Value = serde_json::from_str(
      r#"{"current":{"temperature_2m":17.4,"weather_code":2,"is_day":0},"daily":{"temperature_2m_max":[21.0],"temperature_2m_min":[12.5]}}"#,
    )
    .unwrap();
    let r = parse_forecast(&f, "İstanbul", false).unwrap();
    assert_eq!((r.temp, r.high, r.low, r.code, r.day), (17.4, 21.0, 12.5, 2, false));
    assert!(parse_forecast(&serde_json::json!({"current":{}}), "x", false).is_none());
  }

  #[test]
  fn every_code_has_a_look() {
    for code in [0, 1, 3, 45, 51, 63, 71, 81, 86, 95, 99, 1234] {
      let (icon, text) = describe(code, true);
      assert!(!icon.is_empty() && !text.is_empty());
    }
    assert_eq!(describe(0, false).0, "bedtime");
  }
  #[test]
  fn weather_identity_tracks_coordinates_place_units_and_language() {
    let mut spec = super::super::layout::Spec::new(1, super::super::layout::Kind::Weather, "");
    spec.location = Some(super::super::layout::Location { country_code: "TR".into(), country: "Türkiye".into(), city: "İzmir".into(), district: "Konak".into(),
      latitude: 38.4, longitude: 27.1, city_latitude: 38.42, city_longitude: 27.14 });
    let before = query(&spec, "tr");
    assert!(before.contains("latitude=38.4&longitude=27.1&place=Konak%2C%20%C4%B0zmir"));
    spec.location.as_mut().unwrap().latitude = 38.45;
    assert_ne!(before, query(&spec, "tr"));
    spec.location.as_mut().unwrap().latitude = 38.4; spec.fahrenheit = true;
    assert_ne!(before, query(&spec, "tr"));
    spec.fahrenheit = false; assert_ne!(before, query(&spec, "en"));
  }
  #[test]
  fn refresh_bypasses_cache_without_changing_weather_identity() {
    let identity = "/widgets/weather?city=Berlin&language=tr&fahrenheit=0";
    assert_eq!(request_uri(identity, false), identity);
    assert_eq!(request_uri(identity, true), format!("{identity}&refresh=1"));
  }
}
