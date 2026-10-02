//! `logs\live-wallpaper.log`: one line per event, cut back to its newest
//! half past 256 KB so that a day-long run cannot grow it.

use std::io::Write;

const MAX: u64 = 256 * 1024;

pub fn line(message: &str) {
  let dir = crate::config::data_dir().join("logs");
  let _ = std::fs::create_dir_all(&dir);
  let path = dir.join("live-wallpaper.log");
  if std::fs::metadata(&path)
    .map(|m| m.len() > MAX)
    .unwrap_or(false)
  {
    if let Ok(text) = std::fs::read_to_string(&path) {
      let mut half = text.len() / 2;
      while !text.is_char_boundary(half) {
        half += 1;
      }
      let keep = &text[half..];
      let keep = keep.find('\n').map(|i| &keep[i + 1..]).unwrap_or(keep);
      let _ = std::fs::write(&path, keep);
    }
  }
  if let Ok(mut f) = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&path)
  {
    let secs = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|d| d.as_secs())
      .unwrap_or(0);
    let _ = writeln!(f, "{secs} {message}");
  }
}
