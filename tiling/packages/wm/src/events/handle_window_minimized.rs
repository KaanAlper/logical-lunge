use tracing::info;
use wm_common::{try_warn, WindowState};
use wm_platform::NativeWindow;

use crate::{
  commands::{
    container::set_focused_descendant, window::update_window_state,
  },
  traits::WindowGetters,
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn handle_window_minimized(
  native_window: &NativeWindow,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let found_window = state.window_from_native(native_window);

  // Update the window's state to be minimized.
  if let Some(window) = found_window {
    let is_minimized = try_warn!(window.native().is_minimized());

    window.update_native_properties(|properties| {
      properties.is_minimized = is_minimized;
    });

    // Logical Lunge: an open window never leaves the layout -- it is on a
    // workspace or closed to the tray. Without a taskbar, a minimized window
    // was only reachable with alt+tab. A tiled or floating window that gets
    // minimized (its minimize button, the app itself) keeps its place and is
    // shown again, without taking the focus. Minimize-to-tray apps hide
    // themselves right after minimizing: they are left alone (not visible
    // any more when the check runs). Fullscreen windows (games) still
    // minimize normally.
    #[cfg(target_os = "windows")]
    if is_minimized
      && matches!(window.state(), WindowState::Tiling | WindowState::Floating(_))
      && {
        use wm_platform::NativeWindowWindowsExt;
        window.native().is_controllable()
      }
    {
      use wm_platform::NativeWindowWindowsExt;

      info!("Window minimized, keeping it in the layout: {window}");
      let native = window.native().clone();
      tokio::task::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        if native.is_visible().unwrap_or(false)
          && native.is_minimized().unwrap_or(false)
        {
          let _ = native.show_no_activate();
        }
      });
      return Ok(());
    }

    if is_minimized && window.state() != WindowState::Minimized {
      info!("Window minimized: {window}");

      let window = update_window_state(
        window.clone(),
        WindowState::Minimized,
        state,
        config,
      )?;

      // Clear the drag state, as a window can be minimized while
      // being dragged (e.g. via `toggle-minimized`).
      // TODO: Investigate other code paths where the drag state should be
      // cleared (e.g. most commands that call `update_window_state`).
      window.set_active_drag(None);

      // Focus should be reassigned after a window has been minimized.
      if let Some(focus_target) = state.focus_target_after_removal(&window)
      {
        set_focused_descendant(&focus_target, None);
        state.pending_sync.queue_focus_change().queue_cursor_jump();
        state.unmanaged_or_minimized_timestamp =
          Some(std::time::Instant::now());
      }
    }
  }

  Ok(())
}
