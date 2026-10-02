//! What the bar shows: provider outputs, window manager state, clock and the
//! user's preferences (language, 12 / 24 h clock, theme).

use std::{collections::HashMap, path::Path, time::{SystemTime, UNIX_EPOCH}};

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
  /// The tray panel is open (the arrow points up).
  pub tray_open: bool,
  /// the volume mixer is open (its bar button stays pressed)
  pub mixer_open: bool,
  pub hour12: bool,
  /// prefs.json "animations" (on unless turned off): the panels move as the
  /// web ones do
  pub animations: bool,
  /// prefs.json "dnd": do not disturb, no notification cards
  pub dnd: bool,
  /// seconds a notification card stays: information, warnings and errors
  pub toast_info: u32,
  pub toast_error: u32,
  /// Language of the date and of `tr()`: prefs.json, else the Windows UI language.
  locale: String,
  /// Turkish source text -> translation (empty for Turkish).
  dict: HashMap<String, String>,
  /// "$1 uygulama"-style texts: the pattern, its translation, which `$n`
  /// each group of the pattern is
  patterns: Vec<(regex::Regex, String, Vec<usize>)>,
}

impl Model {
  pub fn new(pack_dir: &Path) -> Self {
    let prefs = prefs(pack_dir);
    let locale = match prefs["language"].as_str() {
      Some(l) if !l.is_empty() && l != "system" => l.to_string(),
      _ => ui_language(),
    };
    let (dict, patterns) = load_dict(pack_dir, &locale);
    let mut m = Model {
      hour12: prefs["clock"].as_str() == Some("12"),
      animations: prefs["animations"].as_bool() != Some(false),
      light: prefs["theme"].as_str() == Some("light"),
      dict,
      patterns,
      locale,
      ..Default::default()
    };
    m.read_prefs(&prefs);
    m.tick_clock();
    m
  }

  /// The preferences that apply at once (the core announces a change with
  /// ll:prefs): do not disturb and how long notification cards stay.
  pub fn read_prefs(&mut self, prefs: &serde_json::Value) {
    let secs = |key: &str, default: u32| {
      prefs[key].as_u64().filter(|n| (1..=60).contains(n)).map_or(default, |n| n as u32)
    };
    self.dnd = prefs["dnd"].as_bool() == Some(true);
    self.toast_info = secs("toastInfo", 3);
    self.toast_error = secs("toastError", 5);
  }

  /// A text in the user's language, as the web widgets translate it: the
  /// exact entry, else a pattern ("$1 uygulama") whose parts are translated
  /// too. Texts without letters stay as they are.
  pub fn tr(&self, s: &str) -> String {
    let k = s.trim();
    if k.is_empty() || !k.chars().any(char::is_alphabetic) {
      return s.to_string();
    }
    if let Some(v) = self.dict.get(k) {
      return s.replacen(k, v, 1);
    }
    for (re, val, order) in &self.patterns {
      if let Some(m) = re.captures(k) {
        let mut v = val.clone();
        for (j, n) in order.iter().enumerate() {
          let part = m.get(j + 1).map_or("", |g| g.as_str());
          v = v.replacen(&format!("${n}"), &self.tr(part), 1);
        }
        return s.replacen(k, &v, 1);
      }
    }
    s.to_string()
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
    let mut position = s.position_seconds;
    if s.is_playing && s.timeline_updated_at > 0 {
      let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
      position += (now - s.timeline_updated_at as f64 / 1000.0).max(0.0) * s.playback_rate;
    }
    let progress = if s.end_time > 0 { (position / s.end_time as f64).clamp(0.0, 1.0) as f32 } else { 0.0 };
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
    self.wm.hash_bar_visible(&mut h);
    self.cpu.as_ref().map(|c| c.usage.round() as i32).hash(&mut h);
    self
      .memory
      .as_ref()
      .map(|m| {
        let swap = if m.total_swap > 0 { (m.used_swap as f64 / m.total_swap as f64 * 100.0).round() as i32 } else { -1 };
        (m.usage.round() as i32, swap)
      })
      .hash(&mut h);
    self.battery.as_ref().map(|b| (b.charge_percent.round() as i32, b.is_plugged)).hash(&mut h);
    self.network_icon().hash(&mut h);
    (self.volume_muted(), self.mic_muted()).hash(&mut h);
    // the ring is ~60 px round: finer steps are invisible
    self.media_title().map(|(t, p, playing)| (t, (p * 240.0) as i32, playing)).hash(&mut h);
    for ic in self.pinned_icons() {
      (&ic.id, &ic.icon_hash).hash(&mut h);
    }
    self.tray_count().hash(&mut h);
    (&self.time, &self.date, self.light, &self.pins, self.tray_open).hash(&mut h);
    h.finish()
  }

  pub fn pinned_icons(&self) -> Vec<&SystrayOutputIcon> {
    let (Some(tray), Some(pins)) = (&self.systray, &self.pins) else { return Vec::new() };
    pins.iter().filter_map(|k| tray.icons.iter().find(|ic| &pin_key(ic) == k)).collect()
  }

  /// Tray icons in the panel (everything not pinned).
  pub fn unpinned_icons(&self) -> Vec<&SystrayOutputIcon> {
    let Some(tray) = &self.systray else { return Vec::new() };
    let pins = self.pins.as_deref().unwrap_or(&[]);
    tray.icons.iter().filter(|ic| !pins.contains(&pin_key(ic))).collect()
  }

  pub fn tray_icon(&self, id: &str) -> Option<&SystrayOutputIcon> {
    self.systray.as_ref()?.icons.iter().find(|ic| ic.id == id)
  }
}

/// A tray icon's pin key: its tooltip's first word; without a tooltip the
/// owner's exe name (the icon id changes between runs). The old web bar used
/// the same key, so its saved pins still match.
pub fn pin_key(ic: &SystrayOutputIcon) -> String {
  let src = if !ic.tooltip.trim().is_empty() {
    ic.tooltip.as_str()
  } else if !ic.process_name.is_empty() {
    ic.process_name.as_str()
  } else {
    ic.id.as_str()
  };
  src
    .split(|c: char| c.is_whitespace() || c == '-' || c == '–' || c == ':')
    .next()
    .unwrap_or("")
    .to_lowercase()
}

/// The user's preferences (`~\.config\logical-lunge\prefs.json`, the core's
/// file: language, clock, theme, bar); the pack's copy only as a fallback.
pub fn prefs(pack_dir: &Path) -> serde_json::Value {
  let user = std::env::var_os("USERPROFILE")
    .map(|h| Path::new(&h).join(".config").join("logical-lunge").join("prefs.json"));
  for path in user.into_iter().chain(std::iter::once(pack_dir.join("prefs.json"))) {
    if let Some(v) = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()) {
      return v;
    }
  }
  serde_json::Value::Null
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

type Patterns = Vec<(regex::Regex, String, Vec<usize>)>;

/// i18n.json (the web widgets' file): exact texts, and the ones with `$1`
/// as patterns (a text that starts with a variable part is tried last, as
/// the web widgets do).
fn load_dict(pack_dir: &Path, locale: &str) -> (HashMap<String, String>, Patterns) {
  let code = locale.get(..2).unwrap_or("en").to_lowercase();
  if code == "tr" {
    return Default::default();
  }
  let Ok(text) = std::fs::read_to_string(pack_dir.join("i18n.json")) else {
    return Default::default();
  };
  let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
    return Default::default();
  };
  let langs: Vec<&str> = v["langs"].as_array().into_iter().flatten().filter_map(|l| l.as_str()).collect();
  let code = if langs.contains(&code.as_str()) { code } else { "en".into() };
  let keys = v["keys"].as_array().cloned().unwrap_or_default();
  let vals = v[code.as_str()].as_array().cloned().unwrap_or_default();
  let var = regex::Regex::new(r"\$(\d)").expect("pattern");
  let mut dict = HashMap::new();
  let mut patterns: Vec<(bool, (regex::Regex, String, Vec<usize>))> = Vec::new();
  for (k, v) in keys.iter().zip(vals.iter()) {
    let (Some(k), Some(v)) = (k.as_str(), v.as_str()) else { continue };
    if !var.is_match(k) {
      dict.insert(k.to_string(), v.to_string());
      continue;
    }
    let mut source = String::from("^");
    let mut order = Vec::new();
    let mut last = 0;
    for m in var.captures_iter(k) {
      let whole = m.get(0).expect("match");
      source.push_str(&regex::escape(&k[last..whole.start()]));
      source.push_str("(.+?)");
      order.push(m[1].parse().unwrap_or(1));
      last = whole.end();
    }
    source.push_str(&regex::escape(&k[last..]));
    source.push('$');
    if let Ok(re) = regex::Regex::new(&source) {
      patterns.push((k.starts_with('$'), (re, v.to_string(), order)));
    }
  }
  patterns.sort_by_key(|(variable_first, _)| *variable_first);
  (dict, patterns.into_iter().map(|(_, p)| p).collect())
}

/// Local clock time of a Unix timestamp, in the user's 12 / 24 h setting
/// (clipboard entries in the Super menu).
pub fn clock_at(unix: i64, hour12: bool) -> String {
  use windows::Win32::{
    Foundation::{FILETIME, SYSTEMTIME},
    System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
  };
  let ticks = ((unix.max(0) as u64) + 11_644_473_600) * 10_000_000;
  let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
  let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
  unsafe {
    if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
      return String::new();
    }
    let mut buf = [0u16; 64];
    let n = GetTimeFormatEx(
      &HSTRING::from(""),
      TIME_FORMAT_FLAGS(0),
      Some(&local),
      &HSTRING::from(if hour12 { "h:mm tt" } else { "HH:mm" }),
      Some(&mut buf),
    );
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
  }
}

pub(super) fn format_time(pattern: &str) -> String {
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
