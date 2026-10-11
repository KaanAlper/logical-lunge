//! Issue reports that could not reach the server (no connection, DNS, the
//! project offline) are kept in `%LOCALAPPDATA%\LogicalLunge\state\bug-queue`
//! and sent later: a while after the shell starts and then every ten
//! minutes, backing off while the server stays unreachable. Each file goes
//! once the server accepts it. The queue is bounded (oldest dropped first).

use std::{
  fs,
  path::{Path, PathBuf},
  time::Duration,
};

use serde_json::{json, Value};

use super::bug::{upload, Logs, SendError};

const MAX_REPORTS: usize = 20;
const MAX_BYTES: u64 = 50 * 1024 * 1024;
const FIRST_TRY: Duration = Duration::from_secs(30);
const INTERVAL: Duration = Duration::from_secs(10 * 60);
const MAX_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// waiting to be sent
const PENDING: &str = "json";
/// refused by the server as it is (kept for the record, not sent again)
const REFUSED: &str = "refused";

fn dir() -> PathBuf {
  super::super::state_dir().join("bug-queue")
}

fn unix_millis() -> u128 {
  std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

/// Keeps a report for later (written whole, then renamed, so a half
/// written file is never sent).
pub(super) fn enqueue(base: &Value, logs: &Logs) -> Result<(), String> {
  let dir = dir();
  fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
  let name = format!("{:020}.{PENDING}", unix_millis());
  let tmp = dir.join(format!("{name}.tmp"));
  fs::write(&tmp, json!({ "base": base, "logs": logs }).to_string()).map_err(|e| e.to_string())?;
  fs::rename(&tmp, dir.join(&name)).map_err(|e| e.to_string())?;
  prune(&dir);
  Ok(())
}

/// The queue's files, oldest first (names start with the time they were
/// queued), with their sizes.
fn entries(dir: &Path) -> Vec<(String, u64)> {
  let mut out: Vec<(String, u64)> = fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .filter_map(|e| {
      let name = e.file_name().to_string_lossy().to_string();
      let kept = name.ends_with(&format!(".{PENDING}")) || name.ends_with(&format!(".{REFUSED}"));
      kept.then(|| (name, e.metadata().map_or(0, |m| m.len())))
    })
    .collect();
  out.sort();
  out
}

/// Which files go so that at most `max_count` files and `max_bytes` bytes
/// stay: the oldest first.
fn to_drop(entries: &[(String, u64)], max_count: usize, max_bytes: u64) -> Vec<String> {
  let mut count = entries.len();
  let mut bytes: u64 = entries.iter().map(|(_, size)| size).sum();
  let mut out = Vec::new();
  for (name, size) in entries {
    if count <= max_count && bytes <= max_bytes {
      break;
    }
    out.push(name.clone());
    count -= 1;
    bytes = bytes.saturating_sub(*size);
  }
  out
}

fn prune(dir: &Path) {
  for name in to_drop(&entries(dir), MAX_REPORTS, MAX_BYTES) {
    tracing::warn!("Issue report queue full: dropping {}", name);
    let _ = fs::remove_file(dir.join(name));
  }
}

/// The wait before the next try after `failures` unreachable rounds in a
/// row: ten minutes, doubling up to an hour.
fn backoff(failures: u32) -> Duration {
  let factor = 1u32 << failures.saturating_sub(1).min(8);
  (INTERVAL * factor).min(MAX_INTERVAL)
}

fn logs_of(v: &Value) -> Option<Logs> {
  let item = |i: usize| -> Option<(String, Option<String>)> {
    let pair = v.get(i)?;
    Some((pair.get(0)?.as_str()?.to_string(), pair.get(1).and_then(Value::as_str).map(str::to_string)))
  };
  Some([item(0)?, item(1)?, item(2)?])
}

/// Sends the waiting reports, oldest first. Stops at the first one the
/// server can't be reached for. (sent, whether the server was reachable)
fn flush(dir: &Path) -> (usize, bool) {
  let mut sent = 0;
  for (name, _) in entries(dir) {
    if !name.ends_with(&format!(".{PENDING}")) {
      continue;
    }
    let path = dir.join(&name);
    let Some((base, logs)) = fs::read_to_string(&path)
      .ok()
      .and_then(|text| serde_json::from_str::<Value>(&text).ok())
      .and_then(|v| Some((v.get("base")?.clone(), logs_of(v.get("logs")?)?)))
    else {
      tracing::warn!("Issue report queue: unreadable {}, removed", name);
      let _ = fs::remove_file(&path);
      continue;
    };
    match upload(base, logs, |_| {}) {
      Ok(()) => {
        let _ = fs::remove_file(&path);
        sent += 1;
      }
      Err(SendError::Unreachable(err)) => {
        tracing::info!("Issue report queue: server unreachable ({}), later", err);
        return (sent, false);
      }
      // The same report would be refused again: kept aside, not resent.
      Err(SendError::Refused(status, body)) => {
        tracing::warn!("Issue report queue: {} refused ({}: {})", name, status, body);
        let _ = fs::rename(&path, path.with_extension(REFUSED));
      }
    }
  }
  (sent, true)
}

fn has_pending(dir: &Path) -> bool {
  entries(dir).iter().any(|(name, _)| name.ends_with(&format!(".{PENDING}")))
}

/// Starts the sender (once per process); `sent(n)` reports reports that
/// went out.
pub(in crate::native_bar) fn start(sent: impl Fn(usize) + Send + 'static) {
  static STARTED: std::sync::Once = std::sync::Once::new();
  STARTED.call_once(|| {
    let spawned = std::thread::Builder::new().name("ll-bug-queue".into()).spawn(move || {
      std::thread::sleep(FIRST_TRY);
      let dir = dir();
      let mut failures = 0u32;
      loop {
        if has_pending(&dir) {
          let (count, reachable) = flush(&dir);
          if count > 0 {
            sent(count);
          }
          failures = if reachable { 0 } else { failures.saturating_add(1) };
        }
        std::thread::sleep(if failures == 0 { INTERVAL } else { backoff(failures) });
      }
    });
    if let Err(err) = spawned {
      tracing::warn!("Issue report queue: no sender thread: {}", err);
    }
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  fn e(name: &str, size: u64) -> (String, u64) {
    (name.to_string(), size)
  }

  #[test]
  fn the_oldest_reports_go_first_when_the_queue_is_full() {
    let entries: Vec<_> = (0..22).map(|i| e(&format!("{i:020}.json"), 10)).collect();
    let dropped = to_drop(&entries, 20, 1_000);
    assert_eq!(dropped, vec![format!("{:020}.json", 0), format!("{:020}.json", 1)]);
  }

  #[test]
  fn big_reports_are_dropped_until_the_size_fits() {
    let entries = vec![e("1.json", 30), e("2.json", 30), e("3.json", 30)];
    assert_eq!(to_drop(&entries, 20, 60), vec!["1.json".to_string()]);
    assert!(to_drop(&entries, 20, 90).is_empty());
  }

  #[test]
  fn unreachable_rounds_back_off_to_an_hour() {
    assert_eq!(backoff(1), Duration::from_secs(600));
    assert_eq!(backoff(2), Duration::from_secs(1200));
    assert_eq!(backoff(3), Duration::from_secs(2400));
    assert_eq!(backoff(4), MAX_INTERVAL);
    assert_eq!(backoff(40), MAX_INTERVAL);
  }

  #[test]
  fn a_queued_report_reads_back() {
    let logs: Logs = [("core.log".into(), Some("a".into())), ("shell.log".into(), None), ("tiling.log".into(), Some("c".into()))];
    let v = json!({ "base": { "description": "x" }, "logs": logs });
    assert_eq!(logs_of(&v["logs"]), Some(logs));
  }

  #[test]
  fn a_report_is_kept_and_sent_files_stay_bounded() {
    let dir = std::env::temp_dir().join(format!("ll-bug-queue-test-{}", unix_millis()));
    fs::create_dir_all(&dir).unwrap();
    for i in 0..(MAX_REPORTS + 3) {
      fs::write(dir.join(format!("{i:020}.json")), "{}").unwrap();
    }
    prune(&dir);
    let left = entries(&dir);
    assert_eq!(left.len(), MAX_REPORTS);
    assert_eq!(left[0].0, format!("{:020}.json", 3));
    assert!(has_pending(&dir));
    let _ = fs::remove_dir_all(&dir);
  }
}
