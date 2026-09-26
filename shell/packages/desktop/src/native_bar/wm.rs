//! Window manager IPC client (`ws://127.0.0.1:6123`), the Rust twin of
//! `ui/lib/tiling-client.js` + the `tiling` provider in `shell-client.js`:
//! subscribes to the events the bar cares about, folds bursts of events into
//! one query round, and sends a fresh `WmState` only when something changed.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const URL: &str = "ws://127.0.0.1:6123";
const EVENTS: &str = "focus_changed focused_container_moved workspace_activated \
  workspace_deactivated workspace_updated window_managed window_unmanaged monitor_added \
  monitor_updated monitor_removed binding_modes_changed tiling_direction_changed pause_changed";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmWindow {
  pub process: String,
  pub title: String,
  pub handle: i64,
  pub area: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WmWorkspace {
  pub name: String,
  pub has_focus: bool,
  pub displayed: bool,
  /// Biggest non-minimized window (ii: showAppIcons, biggestWindow).
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
}

/// Commands the bar sends (`command focus --workspace 3` ...).
pub type CommandTx = mpsc::UnboundedSender<String>;

/// Runs forever: connects (retrying 0.5 s .. 5 s), reports every new state.
pub fn spawn(on_state: impl Fn(WmState) + Send + 'static) -> CommandTx {
  let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<String>();
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
                Some(c) => ok = tx.send(Message::Text(c.into())).await.is_ok(),
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
        .map(|w| WmWorkspace {
          name: w["name"].as_str().unwrap_or("").to_string(),
          has_focus: w["hasFocus"].as_bool().unwrap_or(false),
          displayed: w["isDisplayed"].as_bool().unwrap_or(false),
          biggest: biggest_window(w),
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

fn biggest_window(node: &Value) -> Option<WmWindow> {
  let mut best: Option<WmWindow> = None;
  for c in node["children"].as_array().into_iter().flatten() {
    let cand = if c["type"] == "window" {
      if c["state"]["type"] == "minimized" {
        continue;
      }
      Some(WmWindow {
        process: c["processName"].as_str().unwrap_or("").to_string(),
        title: c["title"].as_str().unwrap_or("").to_string(),
        handle: c["handle"].as_i64().unwrap_or(0),
        area: c["width"].as_f64().unwrap_or(0.0) * c["height"].as_f64().unwrap_or(0.0),
      })
    } else {
      biggest_window(c)
    };
    if let Some(w) = cand {
      if best.as_ref().map_or(true, |b| w.area > b.area) {
        best = Some(w);
      }
    }
  }
  best
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
