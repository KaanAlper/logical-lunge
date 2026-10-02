//! What the right panel remembers between starts
//! (`%LOCALAPPDATA%\LogicalLunge\state\sidebar.json`): the quick settings'
//! layout, the open tab, the to-do list, the timer, dismissed notifications,
//! the galleries' last categories and the last known hardware state (the
//! tiles show it at once instead of appearing later). The web panel kept
//! the same things in its browser storage.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Tile {
  Wifi,
  Ethernet,
  Bluetooth,
  IdleInhibitor,
  NightLight,
  DarkMode,
  ScreenSnip,
  OnScreenKeyboard,
  Mic,
  Audio,
  Notifications,
}

/// sidebar.html AVAILABLE, in its order
pub(super) const AVAILABLE: [Tile; 11] = [
  Tile::Wifi,
  Tile::Ethernet,
  Tile::Bluetooth,
  Tile::IdleInhibitor,
  Tile::NightLight,
  Tile::DarkMode,
  Tile::ScreenSnip,
  Tile::OnScreenKeyboard,
  Tile::Mic,
  Tile::Audio,
  Tile::Notifications,
];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Toggle {
  #[serde(rename = "type")]
  pub tile: Tile,
  /// 1: a round icon, 2: icon, name and state
  pub size: u8,
}

pub(super) fn default_toggles() -> Vec<Toggle> {
  let t = |tile, size| Toggle { tile, size };
  vec![
    t(Tile::Wifi, 2),
    t(Tile::Ethernet, 2),
    t(Tile::IdleInhibitor, 1),
    t(Tile::Mic, 1),
    t(Tile::Bluetooth, 2),
    t(Tile::Audio, 2),
    t(Tile::NightLight, 2),
  ]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Todo {
  pub content: String,
  pub done: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Phase {
  Focus,
  Break,
  Long,
}

/// pomodoro/PomodoroWidget.qml: 25 / 5 minutes, a long break every fourth
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Pomo {
  pub running: bool,
  pub phase: Phase,
  /// seconds left (while paused)
  pub left: u32,
  pub cycle: u32,
  /// when the phase ends (Unix ms) while running
  #[serde(default)]
  pub end: Option<i64>,
}

pub(super) const FOCUS: u32 = 25 * 60;
pub(super) const BREAK: u32 = 5 * 60;
pub(super) const LONG: u32 = 15 * 60;

pub(super) fn phase_secs(p: Phase) -> u32 {
  match p {
    Phase::Focus => FOCUS,
    Phase::Break => BREAK,
    Phase::Long => LONG,
  }
}

impl Default for Pomo {
  fn default() -> Self {
    Pomo { running: false, phase: Phase::Focus, left: FOCUS, cycle: 0, end: None }
  }
}

impl Pomo {
  /// Where a running timer is at `now` (Unix ms): phases that ended while
  /// nobody looked are passed one after another, as the web timer did.
  pub fn advance(&self, now: i64) -> Pomo {
    if !self.running {
      return *self;
    }
    let mut n = *self;
    let mut end = n.end.unwrap_or(now + n.left as i64 * 1000);
    while now >= end {
      let focus_done = n.phase == Phase::Focus;
      if focus_done {
        n.cycle += 1;
      }
      n.phase = if focus_done { if n.cycle % 4 == 0 { Phase::Long } else { Phase::Break } } else { Phase::Focus };
      end += phase_secs(n.phase) as i64 * 1000;
    }
    n.end = Some(end);
    n.left = (((end - now) + 999) / 1000).max(1) as u32;
    n
  }

  pub fn toggle(&self, now: i64) -> Pomo {
    let mut n = *self;
    if n.running {
      n.running = false;
      n.end = None;
    } else {
      n.running = true;
      n.end = Some(now + n.left as i64 * 1000);
    }
    n
  }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Store {
  pub quick_toggles: Option<Vec<Toggle>>,
  pub tab: usize,
  pub collapsed: bool,
  pub todo: Vec<Todo>,
  pub pomo: Pomo,
  /// notification ids cleared from the list (the newest 2000)
  pub notif_dismissed: Vec<String>,
  pub live_cat: Option<String>,
  pub wall_cat: Option<String>,
  /// the wallpaper page's tab
  pub wall_tab: usize,
  /// keep-awake was on: turned on again after a restart
  pub awake_want: bool,
  /// the last hardware state the quick settings saw
  pub qs_cache: Value,
}

fn path() -> std::path::PathBuf {
  super::super::state_dir().join("sidebar.json")
}

impl Store {
  pub fn load() -> Store {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
  }

  pub fn save(&self) {
    let _ = std::fs::create_dir_all(super::super::state_dir());
    let tmp = path().with_extension("json.tmp");
    if let Ok(s) = serde_json::to_string(self) {
      if std::fs::write(&tmp, s).is_ok() {
        let _ = std::fs::rename(&tmp, path());
      }
    }
  }

  /// The tiles in their order (an unknown or doubled tile is left out).
  pub fn toggles(&self) -> Vec<Toggle> {
    let mut out: Vec<Toggle> = Vec::new();
    for t in self.quick_toggles.clone().unwrap_or_else(default_toggles) {
      if !out.iter().any(|o| o.tile == t.tile) {
        out.push(Toggle { tile: t.tile, size: t.size.clamp(1, 2) });
      }
    }
    out
  }

  pub fn dismiss(&mut self, ids: impl IntoIterator<Item = String>) {
    for id in ids {
      if !self.notif_dismissed.contains(&id) {
        self.notif_dismissed.push(id);
      }
    }
    let n = self.notif_dismissed.len();
    if n > 2000 {
      self.notif_dismissed.drain(..n - 2000);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_timer_passes_phases_it_missed() {
    let p = Pomo { running: true, phase: Phase::Focus, left: 10, cycle: 3, end: Some(10_000) };
    // focus ended at 10 s (cycle 4: a long break of 15 min), now 1 min later
    let n = p.advance(70_000);
    assert_eq!(n.phase, Phase::Long);
    assert_eq!(n.cycle, 4);
    assert_eq!(n.left, 14 * 60);
  }

  #[test]
  fn pausing_keeps_the_time_left() {
    let p = Pomo::default().toggle(0);
    assert!(p.running);
    let later = p.advance(60_000);
    assert_eq!(later.left, FOCUS - 60);
    let paused = later.toggle(60_000);
    assert!(!paused.running && paused.end.is_none());
    assert_eq!(paused.advance(10_000_000).left, FOCUS - 60);
  }

  #[test]
  fn toggles_keep_one_of_each() {
    let s = Store {
      quick_toggles: Some(vec![Toggle { tile: Tile::Wifi, size: 2 }, Toggle { tile: Tile::Wifi, size: 1 }, Toggle { tile: Tile::Mic, size: 7 }]),
      ..Default::default()
    };
    assert_eq!(s.toggles(), vec![Toggle { tile: Tile::Wifi, size: 2 }, Toggle { tile: Tile::Mic, size: 2 }]);
  }

  #[test]
  fn the_web_layout_reads_back() {
    let s: Store = serde_json::from_str(r#"{"quickToggles":[{"type":"idleInhibitor","size":1},{"type":"nightLight","size":2}],"tab":2}"#).unwrap();
    assert_eq!(s.toggles()[0].tile, Tile::IdleInhibitor);
    assert_eq!(s.tab, 2);
  }
}
