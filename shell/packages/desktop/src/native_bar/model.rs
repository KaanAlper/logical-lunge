//! What the bar shows: provider outputs, window manager state, clock and the
//! user's preferences (language, 12 / 24 h clock, theme).

use std::{collections::HashMap, path::Path};

use windows::{
  core::{HSTRING, PCWSTR, PWSTR},
  Win32::Globalization::{
    GetDateFormatEx, GetTimeFormatEx, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME,
    TIME_FORMAT_FLAGS,
  },
};

use super::wm::WmState;
use crate::providers::{
  audio::AudioOutput, battery::BatteryOutput, cpu::CpuOutput, media::MediaOutput,
  memory::MemoryOutput, network::{InterfaceType, NetworkOutput},
  systray::{SystrayOutput, SystrayOutputIcon},
  ProviderOutput,
};

#[derive(Default)]
pub struct Model {
  pub wm: WmState,
  pub cpu: Option<CpuOutput>,
  pub memory: Option<MemoryOutput>,
  pub battery: Option<BatteryOutput>,
  pub network: Option<NetworkOutput>,
  pub audio: Option<AudioOutput>,
  pub media: Option<MediaOutput>,
  pub systray: Option<SystrayOutput>,
  pub time: String,
  pub date: String,
  pub light: bool,
  /// Tray icons shown in the bar, by `pin_key` (None: not chosen yet).
  pub pins: Option<Vec<String>>,
  hour12: bool,
  /// Language of the date and of `tr()`: prefs.json, else the Windows UI language.
  locale: String,
  /// Turkish source text -> translation (empty for Turkish).
  dict: HashMap<String, String>,
}

impl Model {
  pub fn new(pack_dir: &Path) -> Self {
    let prefs: serde_json::Value = std::fs::read_to_string(pack_dir.join("prefs.json"))
      .ok()
      .and_then(|s| serde_json::from_str(&s).ok())
      .unwrap_or_default();
    let locale = match prefs["language"].as_str() {
      Some(l) if !l.is_empty() && l != "system" => l.to_string(),
      _ => ui_language(),
    };
    let mut m = Model {
      hour12: prefs["clock"].as_str() == Some("12"),
      dict: load_dict(pack_dir, &locale),
      locale,
      ..Default::default()
    };
    m.tick_clock();
    m
  }

  pub fn tr(&self, s: &str) -> String {
    self.dict.get(s).cloned().unwrap_or_else(|| s.to_string())
  }

  /// Updates the time and date strings; true when they changed.
  pub fn tick_clock(&mut self) -> bool {
    let time = format_time(if self.hour12 { "h:mm tt" } else { "HH:mm" });
    let date = format_date(&self.locale, "dddd, dd/MM");
    let changed = time != self.time || date != self.date;
    self.time = time;
    self.date = date;
    changed
  }

  pub fn apply(&mut self, output: ProviderOutput) {
    match output {
      ProviderOutput::Cpu(o) => self.cpu = Some(o),
      ProviderOutput::Memory(o) => self.memory = Some(o),
      ProviderOutput::Battery(o) => self.battery = Some(o),
      ProviderOutput::Network(o) => self.network = Some(o),
      ProviderOutput::Audio(o) => self.audio = Some(o),
      ProviderOutput::Media(o) => self.media = Some(o),
      ProviderOutput::Systray(o) => self.systray = Some(o),
      _ => {}
    }
  }

  /// ("title • artist", progress 0..1, playing) of the current session.
  pub fn media_title(&self) -> Option<(String, f32, bool)> {
    let s = self.media.as_ref()?.current_session.as_ref()?;
    let title = s.title.as_deref().filter(|t| !t.is_empty())?;
    let text = match s.artist.as_deref().filter(|a| !a.is_empty()) {
      Some(a) => format!("{} • {}", title, a),
      None => title.to_string(),
    };
    let progress = if s.end_time > 0 { (s.position as f32 / s.end_time as f32).min(1.0) } else { 0.0 };
    Some((text, progress, s.is_playing))
  }

  pub fn volume_muted(&self) -> bool {
    self
      .audio
      .as_ref()
      .and_then(|a| a.default_playback_device.as_ref())
      .map_or(false, |d| d.is_muted || d.volume == 0)
  }

  pub fn mic_muted(&self) -> bool {
    self
      .audio
      .as_ref()
      .and_then(|a| a.default_recording_device.as_ref())
      .map_or(false, |d| d.is_muted)
  }

  pub fn network_icon(&self) -> &'static str {
    let Some(net) = &self.network else { return "signal_wifi_off" };
    match net.default_interface.as_ref().map(|i| &i.interface_type) {
      Some(InterfaceType::Ethernet) => "lan",
      Some(InterfaceType::Wifi) => {
        let s = net.default_gateway.as_ref().and_then(|g| g.signal_strength).unwrap_or(0);
        if s >= 75 {
          "signal_wifi_4_bar"
        } else if s >= 50 {
          "network_wifi_3_bar"
        } else if s >= 25 {
          "network_wifi_2_bar"
        } else {
          "network_wifi_1_bar"
        }
      }
      _ => "signal_wifi_off",
    }
  }

  pub fn tray_count(&self) -> usize {
    self.systray.as_ref().map_or(0, |s| s.icons.len())
  }

  /// Everything the bar draws, as it is drawn (rounded like the labels): two
  /// equal keys paint the same pixels.
  pub fn visible_key(&self) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", self.wm).hash(&mut h);
    self.cpu.as_ref().map(|c| c.usage.round() as i32).hash(&mut h);
    self
      .memory
      .as_ref()
      .map(|m| {
        let swap = if m.total_swap > 0 { (m.used_swap as f64 / m.total_swap as f64 * 100.0).round() as i32 } else { -1 };
        (m.usage.round() as i32, swap)
      })
      .hash(&mut h);
    self.battery.as_ref().map(|b| (b.charge_percent.round() as i32, b.is_charging)).hash(&mut h);
    self.network_icon().hash(&mut h);
    (self.volume_muted(), self.mic_muted()).hash(&mut h);
    // the ring is ~60 px round: finer steps are invisible
    self.media_title().map(|(t, p, playing)| (t, (p * 240.0) as i32, playing)).hash(&mut h);
    for ic in self.pinned_icons() {
      (&ic.id, &ic.icon_hash).hash(&mut h);
    }
    self.tray_count().hash(&mut h);
    (&self.time, &self.date, self.light, &self.pins).hash(&mut h);
    h.finish()
  }

  pub fn pinned_icons(&self) -> Vec<&SystrayOutputIcon> {
    let (Some(tray), Some(pins)) = (&self.systray, &self.pins) else { return Vec::new() };
    pins.iter().filter_map(|k| tray.icons.iter().find(|ic| &pin_key(ic) == k)).collect()
  }
}

/// Pins follow the tooltip's first word, not the icon id (ids change between
/// runs) -- same key as the web bar.
pub fn pin_key(ic: &SystrayOutputIcon) -> String {
  let src = if ic.tooltip.trim().is_empty() { ic.id.as_str() } else { ic.tooltip.as_str() };
  src
    .split(|c: char| c.is_whitespace() || c == '-' || c == '–' || c == ':')
    .next()
    .unwrap_or("")
    .to_lowercase()
}

/// First Windows display language (`tr-TR`, `en-US` ...).
fn ui_language() -> String {
  unsafe {
    let mut count = 0u32;
    let mut len = 0u32;
    if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, PWSTR::null(), &mut len).is_err() || len == 0 {
      return "en-US".into();
    }
    let mut buf = vec![0u16; len as usize];
    if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, PWSTR(buf.as_mut_ptr()), &mut len).is_err() {
      return "en-US".into();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
  }
}

/// i18n.json (the web widgets' file): exact matches only; the bar has no patterns.
fn load_dict(pack_dir: &Path, locale: &str) -> HashMap<String, String> {
  let code = locale.get(..2).unwrap_or("en").to_lowercase();
  if code == "tr" {
    return HashMap::new();
  }
  let Ok(text) = std::fs::read_to_string(pack_dir.join("i18n.json")) else {
    return HashMap::new();
  };
  let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
    return HashMap::new();
  };
  let langs: Vec<&str> = v["langs"].as_array().into_iter().flatten().filter_map(|l| l.as_str()).collect();
  let code = if langs.contains(&code.as_str()) { code } else { "en".into() };
  let keys = v["keys"].as_array().cloned().unwrap_or_default();
  let vals = v[code.as_str()].as_array().cloned().unwrap_or_default();
  keys
    .iter()
    .zip(vals.iter())
    .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
    .collect()
}

fn format_time(pattern: &str) -> String {
  unsafe {
    let mut buf = [0u16; 64];
    // invariant locale: "3:05 PM" like the web bar, whatever the UI language
    let n = GetTimeFormatEx(
      &HSTRING::from(""),
      TIME_FORMAT_FLAGS(0),
      None,
      &HSTRING::from(pattern),
      Some(&mut buf),
    );
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
  }
}

fn format_date(locale: &str, pattern: &str) -> String {
  unsafe {
    let mut buf = [0u16; 128];
    let n = GetDateFormatEx(
      &HSTRING::from(locale),
      windows::Win32::Globalization::ENUM_DATE_FORMATS_FLAGS(0),
      None,
      &HSTRING::from(pattern),
      Some(&mut buf),
      PCWSTR::null(),
    );
    if n <= 0 {
      return String::new();
    }
    String::from_utf16_lossy(&buf[..(n - 1) as usize])
  }
}
