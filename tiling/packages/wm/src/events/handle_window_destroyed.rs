use anyhow::Context;
use tracing::info;
use wm_platform::WindowId;

use crate::{
  commands::{window::unmanage_window, workspace::deactivate_workspace},
  traits::{CommonGetters, WindowGetters},
  wm_state::WmState,
};

pub fn handle_window_destroyed(
  native_window_id: WindowId,
  state: &mut WmState,
) -> anyhow::Result<()> {
  state.transition_moves.remove(&native_window_id.0);
  #[cfg(target_os = "windows")]
  {
    // A hidden/unmanaged HWND can be destroyed and reused too.
    state.fullscreen_marks.remove(&native_window_id.0);
    state.fake_fullscreen.remove(&native_window_id.0);
    state.spoof_fullscreen.remove(&native_window_id.0);
    state.background_fullscreen_frames.remove(&native_window_id.0);
    state.self_resizes.remove(&native_window_id.0);
    state.restore_maximized.remove(&native_window_id.0);
  }
  // Logical Lunge: what was hidden with it comes back (if it is still there).
  #[cfg(target_os = "windows")]
  {
    use wm_platform::NativeWindowWindowsExt;
    for companion in state.hidden_companions.remove(&native_window_id.0).unwrap_or_default() {
      if companion.is_valid() {
        let _ = companion.show();
      }
    }
  }
  // Logical Lunge: forget an ignored window when it closes. The list only
  // grew (every PiP, Office popup and shell window stayed in it), each focus
  // event searched it, and a reused handle made a new window "ignored".
  state
    .ignored_windows
    .retain(|window| window.id() != native_window_id);

  let found_window = state
    .windows()
    .into_iter()
    .find(|window| window.native().id() == native_window_id);

  // Unmanage the window if it's currently managed.
  if let Some(window) = found_window {
    let workspace = window.workspace().context("No workspace.")?;

    info!("Window closed: {window}");
    unmanage_window(window, state)?;

    // Hyprland: the special workspace closes with its last window
    if workspace.is_special() && !workspace.has_children() {
      if let Some(monitor) = workspace.monitor() {
        crate::commands::workspace::hide_special_workspace(&monitor, state)?;
      }
    }

    // Destroy parent workspace if window was killed while its workspace
    // was not displayed (e.g. via task manager).
    if !workspace.config().keep_alive
      && !workspace.is_detached()
      && !workspace.has_children()
      && !workspace.is_displayed()
    {
      deactivate_workspace(workspace, state)?;
    }
  }

  Ok(())
}
