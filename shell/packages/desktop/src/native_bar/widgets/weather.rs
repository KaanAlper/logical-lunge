//! The weather widget's data: Open-Meteo (no key, no account). A place
//! name is looked up with its geocoding service; with no name the city of
//! the Windows time zone is used (ICU, part of Windows 10 1903 and later,
//! turns "Turkey Standard Time" into "Europe/Istanbul"). Runs on a worker
//! thread; the UI gets one `Report`.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
  pub place: String,
  pub temp: f32,
  pub high: f32,
  pub low: f32,
  pub code: u32,
  pub day: bool,
  pub fahrenheit: bool,
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
pub fn parse_place(v: &Value) -> Option<(f64, f64, String)> {
  let r = v["results"].get(0)?;
  Some((r["latitude"].as_f64()?, r["longitude"].as_f64()?, r["name"].as_str().unwrap_or("").to_string()))
}

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

/// The weather now at `city` (empty: the time zone's city). Blocks: call it
/// on a worker thread.
#[cfg(windows)]
pub fn fetch(city: &str, language: &str, fahrenheit: bool) -> Result<Report, String> {
  let name = if city.trim().is_empty() { zone_city().ok_or_else(|| "no place".to_string())? } else { city.trim().to_string() };
  let lang = language.split('-').next().unwrap_or("en");
  let geo = get("geocoding-api.open-meteo.com", &format!("/v1/search?name={}&count=1&language={}&format=json", encode(&name), encode(lang)))?;
  let (lat, lon, found) = parse_place(&serde_json::from_str(&geo).map_err(|e| e.to_string())?).ok_or_else(|| format!("no place named {name}"))?;
  let unit = if fahrenheit { "&temperature_unit=fahrenheit" } else { "" };
  let path = format!(
    "/v1/forecast?latitude={lat:.4}&longitude={lon:.4}&current=temperature_2m,weather_code,is_day&daily=temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=1{unit}"
  );
  let body = get("api.open-meteo.com", &path)?;
  let place = if found.is_empty() { name } else { found };
  parse_forecast(&serde_json::from_str(&body).map_err(|e| e.to_string())?, &place, fahrenheit).ok_or_else(|| "bad answer".to_string())
}

/// The IANA city of the Windows time zone (ICU's mapping).
#[cfg(windows)]
fn zone_city() -> Option<String> {
  use windows::{
    core::{s, w},
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
    let f = GetProcAddress(icu, s!("ucal_getTimeZoneIDForWindowsID"))?;
    let f: ForWindowsId = std::mem::transmute(f);
    let mut out = [0u16; 64];
    let mut status = 0i32;
    let n = f(key.as_ptr(), key.len() as i32, std::ptr::null(), out.as_mut_ptr(), out.len() as i32, &mut status);
    if status > 0 || n <= 0 {
      return None;
    }
    city_of_zone(&String::from_utf16_lossy(&out[..n as usize]))
  }
}

/// HTTPS GET (WinHTTP); the body of a 200 answer.
#[cfg(windows)]
fn get(host: &str, path: &str) -> Result<String, String> {
  use windows::{
    core::{w, HSTRING, PCWSTR},
    Win32::Networking::WinHttp::*,
  };
  unsafe {
    let session = WinHttpOpen(w!("LogicalLunge"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
    if session.is_null() {
      return Err("session".into());
    }
    let _ = WinHttpSetTimeouts(session, 10_000, 10_000, 15_000, 15_000);
    let result = (|| -> Result<String, String> {
      let conn = WinHttpConnect(session, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT as u16, 0);
      if conn.is_null() {
        return Err("connect".into());
      }
      let req = WinHttpOpenRequest(conn, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE);
      if req.is_null() {
        let _ = WinHttpCloseHandle(conn);
        return Err("request".into());
      }
      let done = (|| -> Result<String, String> {
        WinHttpSendRequest(req, None, None, 0, 0, 0).map_err(|e| e.message())?;
        WinHttpReceiveResponse(req, std::ptr::null_mut()).map_err(|e| e.message())?;
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
          req,
          WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
          PCWSTR::null(),
          Some((&mut status as *mut u32).cast()),
          &mut len,
          std::ptr::null_mut(),
        )
        .map_err(|e| e.message())?;
        let mut out = Vec::new();
        loop {
          let mut avail = 0u32;
          if WinHttpQueryDataAvailable(req, &mut avail).is_err() || avail == 0 || out.len() > 256 * 1024 {
            break;
          }
          let mut buf = vec![0u8; avail as usize];
          let mut read = 0u32;
          if WinHttpReadData(req, buf.as_mut_ptr().cast(), avail, &mut read).is_err() || read == 0 {
            break;
          }
          out.extend_from_slice(&buf[..read as usize]);
        }
        if status != 200 {
          return Err(format!("HTTP {status}"));
        }
        Ok(String::from_utf8_lossy(&out).to_string())
      })();
      let _ = WinHttpCloseHandle(req);
      let _ = WinHttpCloseHandle(conn);
      done
    })();
    let _ = WinHttpCloseHandle(session);
    result
  }
}

#[cfg(not(windows))]
pub fn fetch(_city: &str, _language: &str, _fahrenheit: bool) -> Result<Report, String> {
  Err("Windows only".into())
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
}
