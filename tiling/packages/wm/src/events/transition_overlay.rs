use wm_common::WindowState;
use wm_platform::{NativeWindow, NativeWindowWindowsExt, WindowZOrder,
  SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, WS_CAPTION, WS_CHILD,
  WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT};
use crate::{traits::{CommonGetters, PositionGetters, WindowGetters}, wm_state::WmState};
use super::overlay_policy::{is_transition_overlay, OverlayFacts};

/// Keep a passive monitor-sized effect inside the tile that owns it.
/// Effects drawn inside the app's surface follow the ordinary tiled geometry path.
pub(crate) fn constrain_transition_overlay(native: &NativeWindow, state: &mut WmState) -> anyhow::Result<bool> {
  let handle = native.hwnd().0;
  if let Some(tile) = state.transition_moves.get(&handle) {
    if native.frame()? == *tile {
      state.transition_moves.remove(&handle);
    }
    return Ok(true);
  }
  if state.is_paused || state.window_from_native(native).is_some() { return Ok(false); }
  // Cheap checks first: avoid querying geometry/processes for ordinary windows.
  if !native.has_window_style_ex(WS_EX_LAYERED)
    || !native.has_window_style_ex(WS_EX_TRANSPARENT)
    || !native.has_window_style_ex(WS_EX_NOACTIVATE) { return Ok(false); }
  let Some(window) = state.focused_container().and_then(|c| c.as_window_container().ok()) else { return Ok(false) };
  if window.state() != WindowState::Tiling { return Ok(false); }
  // Only a spoofed window's own fullscreen is kept in its tile; any other
  // app's fullscreen covers the monitor, effects included
  if !state.keeps_fullscreen_in_tile(window.native().hwnd().0) { return Ok(false); }
  let Some(monitor) = window.monitor() else { return Ok(false) };
  let frame = native.frame()?;
  let bounds = monitor.native_properties().bounds;
  let tile = window.to_rect()?;
  let target = window.native();
  let pid = native.process_id();
  let mut related = pid != 0 && pid == target.process_id();
  let mut owner = native.owner_window();
  let mut visited = std::collections::HashSet::new();
  while let Some(parent) = owner {
    if !visited.insert(parent.hwnd().0) { break; }
    if parent.hwnd() == target.hwnd() { related = true; break; }
    owner = parent.owner_window();
  }
  if !is_transition_overlay(OverlayFacts {
    related_to_tile: related,
    covers_monitor: frame.contains_rect(&bounds) && frame != tile,
    layered: true, click_through: true, no_activate: true,
    caption: native.has_window_style(WS_CAPTION),
    taskbar_entry: native.has_window_style_ex(WS_EX_APPWINDOW),
    child: native.has_window_style(WS_CHILD),
  }) { return Ok(false); }
  native.set_slot(Some(&tile))?;
  // SetWindowRgn sends synchronous position messages to the foreign UI thread.
  // Traces showed ~180 ms before a move could even be requested. Queue it first.
  state.transition_moves.insert(handle, tile.clone());
  if let Err(err) = native.set_window_pos(&WindowZOrder::TopMost, &tile, SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE) {
    state.transition_moves.remove(&handle);
    return Err(err.into());
  }
  Ok(true)
}
