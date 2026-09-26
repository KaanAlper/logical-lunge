//! Icons for the workspace dots: the app list's icon for a window's process
//! (`ui/lib/app-icons.js`, same matching rules), else the window's own icon
//! from the core (`/winicon`). Decoded bitmaps are cached.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;

use super::gfx::Gfx;

#[derive(Deserialize)]
pub struct App {
  #[serde(default)]
  name: String,
  #[serde(default)]
  exe: Option<String>,
  #[serde(default)]
  icon: Option<String>,
}

#[derive(Default)]
pub struct Icons {
  apps: Vec<App>,
  /// process name -> index in `apps` (None: no match)
  matches: HashMap<String, Option<usize>>,
  /// "app:<index>" / "win:<handle>" / "tray:<hash>" -> bitmap (None: undecodable)
  bitmaps: HashMap<String, Option<ID2D1Bitmap1>>,
  /// window icons asked from the core, pending or answered
  asked: HashSet<i64>,
  win_icons: HashMap<i64, Vec<u8>>,
}

const MIN: usize = 4;
const GENERIC: &[&str] = &[
  "setup", "install", "installer", "uninstall", "launcher", "client", "helper", "service",
  "update", "updater", "host", "app", "main", "game", "games", "tool", "tools", "server",
  "runtime", "manager", "windows", "microsoft", "system",
];

fn flat(s: &str) -> String {
  s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

fn words(s: &str) -> Vec<String> {
  s.to_lowercase()
    .split(|c: char| !c.is_ascii_alphanumeric())
    .filter(|w| !w.is_empty())
    .map(str::to_string)
    .collect()
}

/// "WindowsTerminal" -> windows, terminal; "wezterm-gui" -> wezterm, gui
fn proc_words(s: &str) -> Vec<String> {
  let mut spaced = String::new();
  let mut prev: Option<char> = None;
  for c in s.chars() {
    if let Some(p) = prev {
      if (p.is_ascii_lowercase() || p.is_ascii_digit()) && c.is_ascii_uppercase() {
        spaced.push(' ');
      }
    }
    spaced.push(c);
    prev = Some(c);
  }
  words(&spaced)
}

fn usable(w: &str) -> bool {
  w.len() >= MIN && !GENERIC.contains(&w)
}

impl Icons {
  pub fn set_apps(&mut self, apps: Vec<App>) {
    self.apps = apps;
    self.matches.clear();
    self.bitmaps.retain(|k, _| !k.starts_with("app:"));
  }

  pub fn has_apps(&self) -> bool {
    !self.apps.is_empty()
  }

  fn match_app(&self, proc: &str) -> Option<usize> {
    let p = proc.to_lowercase();
    let fp = flat(&p);
    if fp.is_empty() || self.apps.is_empty() {
      return None;
    }
    let has_icon = |a: &App| a.icon.as_deref().map_or(false, |i| !i.is_empty());
    if let Some(i) = self.apps.iter().position(|a| has_icon(a) && a.exe.as_deref() == Some(p.as_str())) {
      return Some(i);
    }
    if let Some(i) = self.apps.iter().position(|a| has_icon(a) && flat(&a.name) == fp) {
      return Some(i);
    }
    if GENERIC.contains(&fp.as_str()) {
      return None;
    }
    let pw: Vec<String> = proc_words(proc).into_iter().filter(|w| usable(w)).collect();
    let mut best = None;
    let mut best_score = 0;
    for (i, a) in self.apps.iter().enumerate() {
      if !has_icon(a) {
        continue;
      }
      let name = flat(&a.name);
      let mut score = 0;
      for cand in [name.clone(), flat(a.exe.as_deref().unwrap_or(""))] {
        if usable(&cand) && fp.starts_with(&cand) {
          score = score.max(cand.len() * 2);
        }
      }
      if score == 0 && usable(&name) && pw.contains(&name) {
        score = name.len();
      }
      if score == 0 && usable(&fp) && words(&a.name).contains(&fp) {
        score = fp.len();
      }
      if score > best_score {
        best_score = score;
        best = Some(i);
      }
    }
    best
  }

  /// Bitmap for a window: app list first, then the window's own icon.
  /// Returns the handle to ask the core for when neither is known yet.
  pub fn for_window(&mut self, gfx: &Gfx, proc: &str, handle: i64) -> (Option<ID2D1Bitmap1>, Option<i64>) {
    let key = proc.to_lowercase();
    let m = match self.matches.get(&key) {
      Some(m) => *m,
      None => {
        let m = self.match_app(proc);
        if self.has_apps() {
          self.matches.insert(key, m);
        }
        m
      }
    };
    if let Some(i) = m {
      let k = format!("app:{}", i);
      if !self.bitmaps.contains_key(&k) {
        let bmp = self.apps[i].icon.as_deref().and_then(data_url_bytes).and_then(|b| gfx.bitmap(&b).ok());
        self.bitmaps.insert(k.clone(), bmp);
      }
      if let Some(Some(b)) = self.bitmaps.get(&k) {
        return (Some(b.clone()), None);
      }
    }
    if handle == 0 {
      return (None, None);
    }
    let k = format!("win:{}", handle);
    if let Some(bytes) = self.win_icons.get(&handle) {
      if !self.bitmaps.contains_key(&k) {
        let bmp = gfx.bitmap(bytes).ok();
        self.bitmaps.insert(k.clone(), bmp);
      }
      return (self.bitmaps.get(&k).cloned().flatten(), None);
    }
    if self.asked.insert(handle) {
      if self.asked.len() > 400 {
        self.asked.clear();
        self.asked.insert(handle);
      }
      return (None, Some(handle));
    }
    (None, None)
  }

  /// The core answered for a window (None: it had none; asked again later).
  pub fn set_win_icon(&mut self, handle: i64, png: Option<Vec<u8>>) {
    match png {
      Some(b) => {
        self.win_icons.insert(handle, b);
      }
      None => {
        self.asked.remove(&handle);
      }
    }
  }

  /// Tray icon bitmap by the provider's icon hash.
  pub fn tray(&mut self, gfx: &Gfx, hash: &str, bytes: &[u8]) -> Option<ID2D1Bitmap1> {
    let k = format!("tray:{}", hash);
    if !self.bitmaps.contains_key(&k) {
      if self.bitmaps.len() > 600 {
        self.bitmaps.retain(|k, _| !k.starts_with("tray:"));
      }
      self.bitmaps.insert(k.clone(), gfx.bitmap(bytes).ok());
    }
    self.bitmaps.get(&k).cloned().flatten()
  }
}

/// `data:image/png;base64,...` -> bytes
pub fn data_url_bytes(url: &str) -> Option<Vec<u8>> {
  let (_, b64) = url.split_once(";base64,")?;
  base64_decode(b64)
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
  let mut out = Vec::with_capacity(s.len() * 3 / 4);
  let mut acc = 0u32;
  let mut bits = 0;
  for c in s.bytes() {
    let v = match c {
      b'A'..=b'Z' => c - b'A',
      b'a'..=b'z' => c - b'a' + 26,
      b'0'..=b'9' => c - b'0' + 52,
      b'+' | b'-' => 62,
      b'/' | b'_' => 63,
      b'=' | b'\r' | b'\n' | b' ' => continue,
      _ => return None,
    } as u32;
    acc = (acc << 6) | v;
    bits += 6;
    if bits >= 8 {
      bits -= 8;
      out.push((acc >> bits) as u8);
    }
  }
  Some(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn app(name: &str, exe: Option<&str>) -> App {
    App { name: name.into(), exe: exe.map(str::to_string), icon: Some("data:image/png;base64,AA==".into()) }
  }

  #[test]
  fn matching() {
    let mut i = Icons::default();
    i.set_apps(vec![
      app("EA", None),
      app("Steam", None),
      app("Terminal", None),
      app("WezTerm", None),
      app("File Explorer", None),
      app("Google Chrome", None),
      app("Monochrome", None),
      app("League of Legends", Some("leagueclient.exe")),
    ]);
    let name = |i: &Icons, p: &str| i.match_app(p).map(|x| i.apps[x].name.clone());
    assert_eq!(name(&i, "steamwebhelper"), Some("Steam".into()));
    assert_eq!(name(&i, "WindowsTerminal"), Some("Terminal".into()));
    assert_eq!(name(&i, "wezterm-gui"), Some("WezTerm".into()));
    assert_eq!(name(&i, "explorer"), Some("File Explorer".into()));
    assert_eq!(name(&i, "chrome"), Some("Google Chrome".into()));
    // "ea" is inside "leagueclientux" and "steamwebhelper": too short to count
    assert_eq!(name(&i, "leagueclientux"), None);
    assert_eq!(name(&i, "mintty"), None);
  }

  #[test]
  fn base64() {
    assert_eq!(base64_decode("aGVsbG8="), Some(b"hello".to_vec()));
    assert_eq!(data_url_bytes("data:image/png;base64,aGk="), Some(b"hi".to_vec()));
  }
}
