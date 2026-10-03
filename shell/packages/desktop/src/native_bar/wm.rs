//! Window manager IPC client (`ws://127.0.0.1:6123`): subscribes to the events the bar cares about, folds bursts of events into
//! one query round, and sends a fresh `WmState` only when something changed.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const URL: &str = "ws://127.0.0.1:6123";
const EVENTS: &str = "focus_changed focused_container_moved workspace_activated \
  workspace_deactivated workspace_updated window_managed window_unmanaged monitor_added \
  monitor_updated monitor_removed binding_modes_changed tiling_direction_changed pause_changed user_config_changed";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmWindow {
  pub id: String,
  pub process: String,
  pub title: String,
  pub handle: i64,
  pub x: i32,
  pub y: i32,
  pub width: i32,
  pub height: i32,
  pub has_focus: bool,
  pub area: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmWorkspace {
  pub name: String,
  pub has_focus: bool,
  pub displayed: bool,
  /// Every non-minimized window, including those inside split containers.
  pub windows: Vec<WmWindow>,
  /// Biggest non-minimized window (its icon stands for the workspace).
  pub biggest: Option<WmWindow>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmMonitor {
  pub device_name: String,
  pub x: i32,
  pub y: i32,
  pub width: i32,
  pub height: i32,
  pub has_focus: bool,
  pub workspaces: Vec<WmWorkspace>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmState {
  pub connected: bool,
  pub monitors: Vec<WmMonitor>,
  pub workspace_order: Vec<String>,
  /// Focused container: `Some((process, title))` for a window, `None` for a workspace.
  pub focused_window: Option<(String, String)>,
  pub paused: bool,
  /// (name, display name)
  pub binding_modes: Vec<(String, String)>,
}

impl WmState {
  /// Globally focused workspace (the web bar shows this one on every monitor).
  pub fn focused_workspace(&self) -> Option<&WmWorkspace> {
    self
      .monitors
      .iter()
      .find(|m| m.has_focus)
      .and_then(|m| m.workspaces.iter().find(|w| w.has_focus))
  }

  pub fn all_workspaces(&self) -> impl Iterator<Item = &WmWorkspace> {
    self.monitors.iter().flat_map(|m| m.workspaces.iter())
  }

  /// The bar uses one representative window per workspace. Geometry for
  /// every other window belongs to the overview and must not redraw the bar.
  pub fn hash_bar_visible(&self, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    self.connected.hash(h);
    self.focused_window.hash(h);
    self.paused.hash(h);
    self.binding_modes.hash(h);
    self.workspace_order.hash(h);
    self.monitors.len().hash(h);
    for m in &self.monitors {
      m.device_name.hash(h);
      (m.x, m.y, m.width, m.height, m.has_focus).hash(h);
      m.workspaces.len().hash(h);
      for w in &m.workspaces {
        (&w.name, w.has_focus, w.displayed).hash(h);
        if let Some(b) = &w.biggest {
          (&b.process, &b.title, b.handle, b.area.to_bits()).hash(h);
        }
      }
    }
  }
}

/// The overview needs both focus commands to complete in order. Sending the
/// second command before the workspace is displayed exposes only that window.
pub enum Command {
  Raw(String),
  FocusWindow { workspace: String, id: String },
  MoveWindow { workspace: String, id: String },
}

pub type CommandTx = mpsc::UnboundedSender<Command>;

/// Runs forever: connects (retrying 0.5 s .. 5 s), reports every new state.
pub fn spawn(on_state: impl Fn(WmState) + Send + 'static) -> CommandTx {
  let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Command>();
  tokio::spawn(async move {
    let mut retry = Duration::from_millis(500);
    let mut last: Option<WmState> = None;
    loop {
      match connect_async(URL).await {
        Ok((ws, _)) => {
          retry = Duration::from_millis(500);
          let (mut tx, mut rx) = ws.split();
          let mut ok = tx
            .send(Message::Text(format!("sub --events {}", EVENTS).into()))
            .await
            .is_ok();
          let mut dirty = true;
          while ok {
            if dirty {
              let mut again = false;
              match query_all(&mut tx, &mut rx, &mut again).await {
                Some(state) => {
                  // an event arrived while querying: the answer may already be stale
                  dirty = again;
                  if last.as_ref() != Some(&state) {
                    last = Some(state.clone());
                    on_state(state);
                  }
                }
                None => break,
              }
              if dirty {
                tokio::time::sleep(Duration::from_millis(8)).await;
                continue;
              }
            }
            tokio::select! {
              msg = rx.next() => match msg {
                // an event (or a stray reply): wait a moment so that a burst
                // (workspace switch = deactivated + activated + focus) is one query
                Some(Ok(Message::Text(_))) => {
                  tokio::time::sleep(Duration::from_millis(8)).await;
                  dirty = true;
                }
                Some(Ok(_)) => {}
                _ => ok = false,
              },
              cmd = cmd_rx.recv() => match cmd {
                Some(Command::Raw(c)) => ok = tx.send(Message::Text(c.into())).await.is_ok(),
                Some(Command::FocusWindow { workspace, id }) => {
                  let mut again = false;
                  let switched = request(&mut tx, &mut rx, &format!("command focus --workspace {workspace}"), &mut again).await.is_some();
                  if switched {
                    if request(&mut tx, &mut rx, &format!("command focus --container-id {id}"), &mut again).await.is_none() {
                      tracing::warn!("Native overview: could not focus window {id}");
                    }
                  } else {
                    tracing::warn!("Native overview: could not switch to workspace {workspace}");
                  }
                  dirty = true;
                }
                Some(Command::MoveWindow { workspace, id }) => {
                  let mut again = false;
                  if request(&mut tx, &mut rx, &format!("command --id {id} move --workspace {workspace}"), &mut again).await.is_none() {
                    tracing::warn!("Native overview: could not move window {id} to workspace {workspace}");
                  }
                  dirty = true;
                }
                None => return,
              },
            }
          }
        }
        Err(_) => {}
      }
      let off = WmState::default();
      if last.as_ref() != Some(&off) {
        last = Some(off.clone());
        on_state(off);
      }
      tokio::time::sleep(retry).await;
      retry = (retry * 2).min(Duration::from_secs(5));
    }
  });
  cmd_tx
}

type Tx = futures_util::stream::SplitSink<
  tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
  Message,
>;
type Rx = futures_util::stream::SplitStream<
  tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

async fn query_all(tx: &mut Tx, rx: &mut Rx, again: &mut bool) -> Option<WmState> {
  let monitors = request(tx, rx, "query monitors", again).await?;
  let focused = request(tx, rx, "query focused", again).await?;
  let modes = request(tx, rx, "query binding-modes", again).await?;
  let paused = request(tx, rx, "query paused", again).await;

  let mut state = WmState { connected: true, ..Default::default() };
  state.workspace_order = monitors["workspaceOrder"].as_array().into_iter().flatten()
    .filter_map(|name| name.as_str().map(str::to_string)).collect();
  for m in monitors["monitors"].as_array().into_iter().flatten() {
    state.monitors.push(WmMonitor {
      device_name: m["deviceName"].as_str().unwrap_or("").to_string(),
      x: m["x"].as_i64().unwrap_or(0) as i32,
      y: m["y"].as_i64().unwrap_or(0) as i32,
      width: m["width"].as_i64().unwrap_or(0) as i32,
      height: m["height"].as_i64().unwrap_or(0) as i32,
      has_focus: m["hasFocus"].as_bool().unwrap_or(false),
      workspaces: m["children"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| {
          let windows = workspace_windows(w);
          WmWorkspace {
            name: w["name"].as_str().unwrap_or("").to_string(),
            has_focus: w["hasFocus"].as_bool().unwrap_or(false),
            displayed: w["isDisplayed"].as_bool().unwrap_or(false),
            biggest: windows.iter().max_by(|a, b| a.area.total_cmp(&b.area)).cloned(),
            windows,
          }
        })
        .collect(),
    });
  }
  let f = &focused["focused"];
  if f["type"] == "window" {
    state.focused_window = Some((
      f["processName"].as_str().unwrap_or("").to_string(),
      f["title"].as_str().unwrap_or("").to_string(),
    ));
  }
  for m in modes["bindingModes"].as_array().into_iter().flatten() {
    let name = m["name"].as_str().unwrap_or("").to_string();
    let display = m["displayName"].as_str().map(str::to_string).unwrap_or_else(|| name.clone());
    state.binding_modes.push((name, display));
  }
  state.paused = paused.map(|p| p["paused"].as_bool().unwrap_or(false)).unwrap_or(false);
  Some(state)
}

fn workspace_windows(node: &Value) -> Vec<WmWindow> {
  let mut windows = Vec::new();
  collect_windows(node, &mut windows);
  windows
}

fn collect_windows(node: &Value, windows: &mut Vec<WmWindow>) {
  for c in node["children"].as_array().into_iter().flatten() {
    if c["type"] == "window" {
      if c["state"]["type"] == "minimized" {
        continue;
      }
      windows.push(WmWindow {
        id: c["id"].as_str().unwrap_or("").to_string(),
        process: c["processName"].as_str().unwrap_or("").to_string(),
        title: c["title"].as_str().unwrap_or("").to_string(),
        handle: c["handle"].as_i64().unwrap_or(0),
        x: c["x"].as_i64().unwrap_or(0) as i32,
        y: c["y"].as_i64().unwrap_or(0) as i32,
        width: c["width"].as_i64().unwrap_or(0) as i32,
        height: c["height"].as_i64().unwrap_or(0) as i32,
        has_focus: c["hasFocus"].as_bool().unwrap_or(false),
        area: c["width"].as_f64().unwrap_or(0.0) * c["height"].as_f64().unwrap_or(0.0),
      });
    } else {
      collect_windows(c, windows);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn nested_workspace_windows_skip_minimized_and_keep_geometry() {
    let tree = serde_json::json!({"children": [
      {"type":"split", "children": [
        {"type":"window", "id":"a", "processName":"Explorer", "x":10, "y":20, "width":500, "height":600, "hasFocus":true},
        {"type":"window", "id":"b", "state":{"type":"minimized"}}
      ]},
      {"type":"window", "id":"c", "width":100, "height":200}
    ]});
    let found = workspace_windows(&tree);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].id, "a");
    assert_eq!((found[0].x, found[0].y, found[0].width, found[0].height), (10, 20, 500, 600));
    assert!(found[0].has_focus);
    assert_eq!(found[1].id, "c");
  }

  #[test]
  fn bar_hash_ignores_other_window_geometry() {
    use std::hash::Hasher;
    let biggest = WmWindow { id: "a".into(), process: "Explorer".into(), title: "Files".into(), area: 200.0, ..Default::default() };
    let small = WmWindow { id: "b".into(), area: 30.0, ..Default::default() };
    let mut a = WmState { connected: true, monitors: vec![WmMonitor { workspaces: vec![WmWorkspace {
      name: "1".into(), biggest: Some(biggest.clone()), windows: vec![biggest, small], ..Default::default()
    }], ..Default::default() }], ..Default::default() };
    let hash = |state: &WmState| {
      let mut h = std::collections::hash_map::DefaultHasher::new();
      state.hash_bar_visible(&mut h);
      h.finish()
    };
    let before = hash(&a);
    a.monitors[0].workspaces[0].windows[1].x = 99;
    assert_eq!(hash(&a), before);
    a.monitors[0].workspaces[0].biggest.as_mut().unwrap().title = "Changed".into();
    assert_ne!(hash(&a), before);
  }
}

/// Sends `message` and waits for its `client_response` (events arriving in
/// between are skipped; they only mean "query again", which is happening).
async fn request(tx: &mut Tx, rx: &mut Rx, message: &str, again: &mut bool) -> Option<Value> {
  tx.send(Message::Text(message.to_string().into())).await.ok()?;
  let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
  loop {
    let msg = tokio::time::timeout_at(deadline, rx.next()).await.ok()??.ok()?;
    let Message::Text(text) = msg else { continue };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
    if v["messageType"] == "event_subscription" {
      *again = true;
      continue;
    }
    if v["messageType"] == "client_response" && v["clientMessage"] == message {
      if v["error"].is_string() {
        return None;
      }
      return Some(v["data"].clone());
    }
  }
}
