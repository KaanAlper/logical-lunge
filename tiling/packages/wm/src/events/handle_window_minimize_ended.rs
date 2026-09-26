use tracing::info;
use wm_common::{try_warn, WindowState};
use wm_platform::NativeWindow;

use crate::{
  commands::window::{manage_window, update_window_state},
  traits::WindowGetters,
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn handle_window_minimize_ended(
  native_window: &NativeWindow,
  state: &mut WmState,
  config: &mut UserConfig,
) -> anyhow::Result<()> {
  let found_window = state.window_from_native(native_window);

  // Logical Lunge: a window restored while nobody managed it (it was
  // skipped while minimized) joins the layout now instead of floating
  // unmanaged over it.
  if found_window.is_none() {
    let workspace = state
      .nearest_monitor(native_window)
      .and_then(|monitor| monitor.displayed_workspace());
    return manage_window(
      native_window.clone(),
      workspace.map(Into::into),
      state,
      config,
    );
  }

  // Update the window's state to not be minimized.
  if let Some(window) = found_window {
    let is_minimized = try_warn!(window.native().is_minimized());

    window.update_native_properties(|properties| {
      properties.is_minimized = is_minimized;
    });

    // Kept in the layout while minimized (see `handle_window_minimized`):
    // put it back exactly into its tile.
    if !is_minimized && window.state() != WindowState::Minimized {
      state.pending_sync.queue_container_to_redraw(window.clone());
    }

    if !is_minimized && window.state() == WindowState::Minimized {
      info!("Window minimize ended: {window}");

      let target_state = window
        .prev_state()
        .unwrap_or(WindowState::default_from_config(&config.value));

      update_window_state(window.clone(), target_state, state, config)?;
    }
  }

  Ok(())
}
