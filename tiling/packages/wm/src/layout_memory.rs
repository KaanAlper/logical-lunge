//! Logical Lunge: remembers which workspace every window is on, in which
//! order, and which workspace each monitor shows. A restart of the window
//! manager (an update, "reload desktop", a crash) puts the windows back
//! instead of piling them all onto the workspaces that happen to be shown.
//!
//! Written after a sync only when something it records changed; read once
//! on startup. A window is recognised by its handle *and* its process and
//! class (handles are reused), and a snapshot from before the last boot is
//! ignored.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use wm_common::WindowState;
use wm_platform::NativeWindow;

use crate::{
  models::WindowContainer,
  traits::{CommonGetters, WindowGetters},
  wm_state::WmState,
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
  /// Boot time (unix seconds): windows of an earlier boot are other windows.
  boot: i64,
  workspaces: Vec<SavedWorkspace>,
  focused_workspace: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SavedWorkspace {
  name: String,
  /// Device name of the monitor it was on (`\\.\DISPLAY1`).
  monitor: String,
  displayed: bool,
  windows: Vec<SavedWindow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SavedWindow {
  handle: isize,
  process: String,
  class: String,
  floating: bool,
}

/// Where a remembered window goes.
pub struct Placement {
  pub workspace: String,
  pub monitor: String,
  /// (workspace index, position) -- windows are managed in this order so
  /// that each workspace's dwindle is rebuilt in the same order.
  pub order: (usize, usize),
  pub floating: bool,
}

#[derive(Default)]
pub struct LayoutMemory {
  last: Option<Snapshot>,
}

fn path() -> Option<PathBuf> {
  let base = std::env::var_os("LOCALAPPDATA")?;
  Some(PathBuf::from(base).join("LogicalLunge").join("state").join("layout.json"))
}

#[cfg(target_os = "windows")]
fn boot_time() -> i64 {
  #[link(name = "kernel32")]
  extern "system" {
    fn GetTickCount64() -> u64;
  }
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_secs());
  #[allow(clippy::cast_possible_wrap)]
  {
    now as i64 - (unsafe { GetTickCount64() } / 1000) as i64
  }
}

#[cfg(not(target_os = "windows"))]
fn boot_time() -> i64 {
  0
}

/// From what the WM already keeps for a managed window: no system calls on
/// every sync.
fn cached_identity(window: &WindowContainer) -> Option<(isize, String, String)> {
  #[cfg(target_os = "windows")]
  {
    use wm_platform::NativeWindowWindowsExt;
    let props = window.native_properties();
    Some((window.native().hwnd().0, props.process_name, props.class_name))
  }
  #[cfg(not(target_os = "windows"))]
  {
    let _ = window;
    None
  }
}

fn identity(window: &NativeWindow) -> Option<(isize, String, String)> {
  #[cfg(target_os = "windows")]
  {
    use wm_platform::NativeWindowWindowsExt;
    Some((
      window.hwnd().0,
      window.process_name().ok()?,
      window.class_name().ok()?,
    ))
  }
  #[cfg(not(target_os = "windows"))]
  {
    let _ = window;
    None
  }
}

impl LayoutMemory {
  /// The snapshot of this boot, if there is one.
  pub fn load() -> Option<Snapshot> {
    let text = std::fs::read_to_string(path()?).ok()?;
    let snapshot: Snapshot = serde_json::from_str(&text).ok()?;
    // a few seconds of rounding either way
    ((snapshot.boot - boot_time()).abs() <= 10).then_some(snapshot)
  }

  /// Records `state` if what it would record changed. The file is written
  /// by a background thread, at most every 500 ms (the latest version wins):
  /// the WM's loop never waits for the disk.
  pub fn save(&mut self, state: &WmState) {
    let Some(snapshot) = snapshot_of(state) else { return };
    if self.last.as_ref() == Some(&snapshot) {
      return;
    }
    let Ok(json) = serde_json::to_string(&snapshot) else { return };
    self.last = Some(snapshot);
    writer().send(json).ok();
  }
}

impl LayoutMemory {
  /// On exit: written now, not after the writer's pause.
  pub fn save_now(&mut self, state: &WmState) {
    let Some(snapshot) = snapshot_of(state) else { return };
    let Ok(json) = serde_json::to_string(&snapshot) else { return };
    match write_atomic(&json) {
      Ok(()) => self.last = Some(snapshot),
      Err(err) => tracing::warn!("Failed to save the window layout: {}", err),
    }
  }
}

/// The background writer: keeps only the newest snapshot of a burst.
fn writer() -> &'static std::sync::mpsc::Sender<String> {
  use std::sync::{mpsc, OnceLock};
  static TX: OnceLock<mpsc::Sender<String>> = OnceLock::new();
  TX.get_or_init(|| {
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::Builder::new()
      .name("layout-memory".into())
      .spawn(move || {
        while let Ok(mut json) = rx.recv() {
          std::thread::sleep(std::time::Duration::from_millis(500));
          while let Ok(newer) = rx.try_recv() {
            json = newer;
          }
          if let Err(err) = write_atomic(&json) {
            tracing::warn!("Failed to save the window layout: {}", err);
          }
        }
      })
      .ok();
    tx
  })
}

/// Written next to the file, then renamed over it: a crash or a power cut
/// never leaves half a file.
fn write_atomic(json: &str) -> std::io::Result<()> {
  let path = path().ok_or_else(|| std::io::Error::other("no LOCALAPPDATA"))?;
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let tmp = path.with_extension("json.tmp");
  std::fs::write(&tmp, json)?;
  std::fs::rename(&tmp, &path)
}

impl Snapshot {
  pub fn placement(&self, window: &NativeWindow) -> Option<Placement> {
    let (handle, process, class) = identity(window)?;
    self.workspaces.iter().enumerate().find_map(|(i, ws)| {
      ws.windows
        .iter()
        .position(|w| w.handle == handle && w.process == process && w.class == class)
        .map(|pos| Placement {
          workspace: ws.name.clone(),
          monitor: ws.monitor.clone(),
          order: (i, pos),
          floating: ws.windows[pos].floating,
        })
    })
  }

  /// (monitor, workspace) shown on each monitor; the focused one last.
  pub fn displayed(&self) -> Vec<(String, String)> {
    let mut shown: Vec<(String, String)> = self
      .workspaces
      .iter()
      .filter(|ws| ws.displayed)
      .map(|ws| (ws.monitor.clone(), ws.name.clone()))
      .collect();
    if let Some(focused) = &self.focused_workspace {
      shown.sort_by_key(|(_, name)| name == focused);
    }
    shown
  }
}

fn snapshot_of(state: &WmState) -> Option<Snapshot> {
  let mut workspaces = Vec::new();
  for ws in state.workspaces() {
    let monitor = ws.monitor()?.native_properties().device_name;
    let windows = ws
      .descendants()
      .filter_map(|c| c.as_window_container().ok())
      .filter_map(|w: WindowContainer| {
        let (handle, process, class) = cached_identity(&w)?;
        Some(SavedWindow {
          handle,
          process,
          class,
          floating: matches!(w.state(), WindowState::Floating(_)),
        })
      })
      .collect();
    workspaces.push(SavedWorkspace {
      name: ws.config().name,
      monitor,
      displayed: ws.is_displayed(),
      windows,
    });
  }
  let focused_workspace = state
    .focused_container()
    .and_then(|c| c.workspace())
    .map(|ws| ws.config().name);
  Some(Snapshot { boot: boot_time(), workspaces, focused_workspace })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn displayed_puts_the_focused_workspace_last() {
    let ws = |name: &str, monitor: &str, displayed| SavedWorkspace {
      name: name.into(),
      monitor: monitor.into(),
      displayed,
      windows: Vec::new(),
    };
    let s = Snapshot {
      boot: 0,
      workspaces: vec![ws("2", "A", true), ws("3", "A", false), ws("5", "B", true)],
      focused_workspace: Some("2".into()),
    };
    assert_eq!(
      s.displayed(),
      vec![("B".to_string(), "5".to_string()), ("A".to_string(), "2".to_string())]
    );
  }
}
