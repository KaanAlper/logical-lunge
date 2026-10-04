//! `tiling.log`, turned over to `tiling.log.old` past a size, as the core
//! does with `core.log`. It kept growing for good before (a desktop that
//! runs for months); the name stays, since the bug report reads it.

use std::{
  fs::{self, File, OpenOptions},
  io::{self, Write},
  path::PathBuf,
  sync::{Mutex, PoisonError},
};

use tracing_subscriber::fmt::MakeWriter;

/// Where the log turns over (the core's log uses the same).
pub const TURN_OVER_AT: u64 = 4 * 1024 * 1024;

pub struct SizedLog {
  path: PathBuf,
  limit: u64,
  file: Mutex<Option<(File, u64)>>,
}

impl SizedLog {
  pub fn new(path: PathBuf, limit: u64) -> Self {
    Self { path, limit, file: Mutex::new(None) }
  }

  fn old_path(&self) -> PathBuf {
    let mut old = self.path.clone().into_os_string();
    old.push(".old");
    PathBuf::from(old)
  }

  fn open(&self) -> io::Result<(File, u64)> {
    if let Some(dir) = self.path.parent() {
      let _ = fs::create_dir_all(dir);
    }
    let file = OpenOptions::new().create(true).append(true).open(&self.path)?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok((file, len))
  }

  fn write_all(&self, buf: &[u8]) -> io::Result<()> {
    let mut slot = self.file.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.as_ref().is_some_and(|(_, len)| *len >= self.limit) {
      *slot = None;
      let _ = fs::remove_file(self.old_path());
      // Someone holding it open (a viewer) keeps it from moving: then it
      // grows on until it can.
      let _ = fs::rename(&self.path, self.old_path());
    }
    if slot.is_none() {
      *slot = Some(self.open()?);
    }
    let (file, len) = slot.as_mut().expect("opened above");
    file.write_all(buf)?;
    *len += buf.len() as u64;
    Ok(())
  }
}

pub struct SizedLogWriter<'a>(&'a SizedLog);

impl Write for SizedLogWriter<'_> {
  fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    self.0.write_all(buf)?;
    Ok(buf.len())
  }

  fn flush(&mut self) -> io::Result<()> {
    Ok(())
  }
}

impl<'a> MakeWriter<'a> for SizedLog {
  type Writer = SizedLogWriter<'a>;

  fn make_writer(&'a self) -> Self::Writer {
    SizedLogWriter(self)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn turns_over_at_its_size_and_keeps_its_name() {
    let dir = std::env::temp_dir().join(format!("lunge-sized-log-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let log = SizedLog::new(dir.join("tiling.log"), 1000);
    let line = [b'x'; 99];
    for i in 0..25 {
      let mut w = log.make_writer();
      w.write_all(&line).unwrap();
      w.write_all(b"\n").unwrap();
      assert!(fs::metadata(dir.join("tiling.log")).unwrap().len() <= 1000 + 100, "line {i}: grew past its size");
    }
    let old = fs::metadata(dir.join("tiling.log.old")).unwrap().len();
    assert!(old >= 1000 && old <= 1100, "the turned-over part is the size's worth: {old}");
    // what is already there counts: a new log (the next start) goes on from it
    drop(log);
    let again = SizedLog::new(dir.join("tiling.log"), 1000);
    let before = fs::metadata(dir.join("tiling.log")).unwrap().len();
    again.make_writer().write_all(b"next start\n").unwrap();
    assert_eq!(fs::metadata(dir.join("tiling.log")).unwrap().len(), before + 11);
    let _ = fs::remove_dir_all(&dir);
  }
}
