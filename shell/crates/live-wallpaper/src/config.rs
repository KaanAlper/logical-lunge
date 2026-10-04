//! `state\live-wallpaper.json`, written by the core:
//!
//! ```text
//! {"wallpapers":[{"monitor":"\\\\.\\DISPLAY1","file":"C:\\...\\a.mp4"}],
//!  "pauseFullscreen":true,"pauseOnBattery":true,"pauseIdleMinutes":10,
//!  "reduceVideo":true}
//! ```
//!
//! `monitor` is a monitor's GDI device name or its device interface path
//! (the id IDesktopWallpaper uses for the static wallpaper), or `*` for
//! every monitor without its own entry. An empty `file` turns its monitor
//! off while a video is set for every monitor.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
  pub monitor: String,
  pub file: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
  pub wallpapers: Vec<Entry>,
  /// a monitor whose wallpaper can't be seen (a fullscreen app, or windows
  /// covering it all but the gaps) pauses its video
  pub pause_fullscreen: bool,
  /// running on battery pauses every wallpaper
  pub pause_on_battery: bool,
  /// no input for this many minutes pauses every wallpaper on its frame
  /// (nobody is watching); 0: never
  pub pause_idle_minutes: u32,
  /// a video much bigger than its monitors plays from a copy at their size
  /// (copies.rs)
  pub reduce_video: bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      wallpapers: Vec::new(),
      pause_fullscreen: true,
      pause_on_battery: true,
      pause_idle_minutes: 10,
      reduce_video: true,
    }
  }
}

pub fn data_dir() -> PathBuf {
  std::env::var_os("LOCALAPPDATA")
    .map(PathBuf::from)
    .unwrap_or_default()
    .join("LogicalLunge")
}

impl Config {
  pub fn path() -> PathBuf {
    data_dir().join("state").join("live-wallpaper.json")
  }

  /// None while the file is there but cannot be read or parsed (it is
  /// being replaced, a scanner holds it): the caller keeps what it has. No
  /// file: nothing is set.
  pub fn load() -> Option<Config> {
    match std::fs::read_to_string(Self::path()) {
      Ok(text) => Self::parse(&text),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
        Some(Config::default())
      }
      Err(_) => None,
    }
  }

  /// None for text that is not JSON; unknown or malformed parts fall back
  /// to the defaults, entries without a monitor are skipped.
  pub fn parse(text: &str) -> Option<Config> {
    let mut c = Config::default();
    let v = serde_json::from_str::<serde_json::Value>(
      text.trim_start_matches('\u{feff}'),
    )
    .ok()?;
    if let Some(list) = v["wallpapers"].as_array() {
      for e in list {
        let (Some(monitor), Some(file)) =
          (e["monitor"].as_str(), e["file"].as_str())
        else {
          continue;
        };
        if monitor.is_empty() {
          continue;
        }
        c.wallpapers.push(Entry {
          monitor: monitor.to_string(),
          file: PathBuf::from(file),
        });
      }
    }
    if let Some(b) = v["pauseFullscreen"].as_bool() {
      c.pause_fullscreen = b;
    }
    if let Some(b) = v["pauseOnBattery"].as_bool() {
      c.pause_on_battery = b;
    }
    if let Some(m) = v["pauseIdleMinutes"].as_u64() {
      c.pause_idle_minutes = m.min(24 * 60) as u32;
    }
    if let Some(b) = v["reduceVideo"].as_bool() {
      c.reduce_video = b;
    }
    Some(c)
  }

  /// A video is set for some monitor (else the player has nothing to do).
  pub fn active(&self) -> bool {
    self
      .wallpapers
      .iter()
      .any(|e| !e.file.as_os_str().is_empty())
  }

  /// The video of a monitor known by any of `names`: its own entry (none
  /// when that one is off), else the one for every monitor.
  pub fn file_for(&self, names: &[&str]) -> Option<&PathBuf> {
    self
      .wallpapers
      .iter()
      .find(|e| {
        names
          .iter()
          .any(|n| !n.is_empty() && e.monitor.eq_ignore_ascii_case(n))
      })
      .or_else(|| self.wallpapers.iter().find(|e| e.monitor == "*"))
      .map(|e| &e.file)
      .filter(|f| !f.as_os_str().is_empty())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_entries_and_switches() {
    let c = Config::parse(
      r#"{"wallpapers":[{"monitor":"\\\\.\\DISPLAY1","file":"C:\\a.mp4"},{"monitor":"*","file":"C:\\b.mp4"},{"monitor":"","file":"x"}],"pauseOnBattery":false}"#,
    )
    .unwrap();
    assert_eq!(c.wallpapers.len(), 2);
    assert!(c.active());
    assert_eq!(
      c.file_for(&[r"\\.\DISPLAY1"]),
      Some(&PathBuf::from(r"C:\a.mp4"))
    );
    assert_eq!(
      c.file_for(&[r"\\.\display1"]),
      Some(&PathBuf::from(r"C:\a.mp4"))
    );
    assert_eq!(
      c.file_for(&[r"\\.\DISPLAY2", ""]),
      Some(&PathBuf::from(r"C:\b.mp4"))
    );
    assert!(c.pause_fullscreen && !c.pause_on_battery);
    assert_eq!(c.pause_idle_minutes, 10, "unset: the default");
    for (text, minutes) in [(r#"{"pauseIdleMinutes":0}"#, 0), (r#"{"pauseIdleMinutes":3}"#, 3), (r#"{"pauseIdleMinutes":-1}"#, 10), (r#"{"pauseIdleMinutes":"5"}"#, 10), (r#"{"pauseIdleMinutes":999999}"#, 24 * 60)] {
      assert_eq!(Config::parse(text).unwrap().pause_idle_minutes, minutes, "{text}");
    }
  }

  #[test]
  fn not_json_is_none_and_empty_is_inactive() {
    // a file being rewritten: the player keeps the settings it has
    assert_eq!(Config::parse("not json"), None);
    assert_eq!(Config::parse("\u{feff}{}"), Some(Config::default()));
    assert!(!Config::default().active());
    assert_eq!(Config::default().file_for(&[r"\\.\DISPLAY1"]), None);
  }

  #[test]
  fn matches_the_static_wallpaper_id() {
    let id = r"\\?\DISPLAY#BOE0812#4&2a3b5c7d&0&UID8388688#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
    let c = Config::parse(&format!(
      r#"{{"wallpapers":[{{"monitor":{},"file":"C:\\a.mp4"}}]}}"#,
      serde_json::to_string(id).unwrap()
    ))
    .unwrap();
    assert_eq!(
      c.file_for(&[r"\\.\DISPLAY1", id]),
      Some(&PathBuf::from(r"C:\a.mp4"))
    );
    assert_eq!(c.file_for(&[r"\\.\DISPLAY2", "other"]), None);
  }

  #[test]
  fn one_monitor_off_under_a_video_for_all() {
    let c = Config::parse(
      r#"{"wallpapers":[{"monitor":"*","file":"C:\\b.mp4"},{"monitor":"\\\\.\\DISPLAY2","file":""}]}"#,
    )
    .unwrap();
    assert_eq!(
      c.file_for(&[r"\\.\DISPLAY1"]),
      Some(&PathBuf::from(r"C:\b.mp4"))
    );
    assert_eq!(c.file_for(&[r"\\.\DISPLAY2"]), None);
  }
}
