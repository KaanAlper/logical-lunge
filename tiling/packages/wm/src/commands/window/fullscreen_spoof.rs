//! Logical Lunge: an app's own fullscreen as in ii (Hyprland). It is real
//! fullscreen; ii's "fullscreen spoof" (Super+Alt+F, Hyprland's
//! `fullscreen_state` internal 0 / client fullscreen) keeps it in its tile
//! instead. Hyprland tells the client its fullscreen state through the
//! protocol; on Windows the app is asked the way a user asks it: F11.
use wm_common::{FullscreenStateConfig, WindowState};
use wm_platform::NativeWindowWindowsExt;

use crate::{
  commands::{
    general::window_sync_policy::is_app_fullscreen, window::update_window_state,
  },
  models::WindowContainer,
  traits::{CommonGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Whether the window is in its app's own fullscreen right now: it covers
/// its monitor and has dropped its frame.
fn in_app_fullscreen(window: &WindowContainer) -> bool {
  let native = window.native();
  let framed = native.has_window_style(wm_platform::WS_CAPTION)
    || native.has_window_style(wm_platform::WS_THICKFRAME);
  match (native.frame(), window.monitor()) {
    (Ok(frame), Some(monitor)) => {
      is_app_fullscreen(&frame, &monitor.native_properties().bounds, framed)
    }
    _ => false,
  }
}

/// ii's `misc:on_focus_under_fullscreen = 2`: another window focused on a
/// workspace that has a fullscreen (or maximized) window takes that window
/// out of it, back to its tile or float, and the layout is normal again.
/// Hovering moves no focus to or from a fullscreen window (the core's
/// focus-follows-mouse), so only a click, a key or a new window does this.
pub fn leave_fullscreen_for_focus(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  use crate::commands::general::window_sync_policy::leaves_fullscreen_for_focus;

  let Some(focused) = state
    .focused_container()
    .and_then(|container| container.as_window_container().ok())
  else {
    return Ok(());
  };
  let Some(workspace) = focused.workspace() else {
    return Ok(());
  };

  let leaving = workspace
    .descendants()
    .filter_map(|descendant| descendant.as_window_container().ok())
    .filter(|window| {
      leaves_fullscreen_for_focus(
        &window.state(),
        window.id() == focused.id(),
        true,
        &focused.state(),
      )
    })
    .collect::<Vec<_>>();

  for window in leaving {
    tracing::info!("Leaving fullscreen for the focused window: {window}");
    state.restore_maximized.remove(&window.native().hwnd().0);
    state.fake_fullscreen.remove(&window.native().hwnd().0);
    let target = window.toggled_state(window.state(), config);
    update_window_state(window.clone(), target, state, config)?;
  }

  Ok(())
}

/// `toggle-fullscreen-spoof` (ii: Super+Alt+F). On: the window's own
/// fullscreen stays in its tile from now on, and the app is asked into its
/// fullscreen (or, already fullscreen, comes down into its tile). Off: the
/// app is asked out of its fullscreen, and its next one is real fullscreen.
pub fn toggle_fullscreen_spoof(
  window: WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let handle = window.native().hwnd().0;

  if state.spoof_fullscreen.remove(&handle) {
    if state.fake_fullscreen.contains(&handle) || in_app_fullscreen(&window) {
      window.native().press_fullscreen_key();
    }
    return Ok(());
  }

  match window.state() {
    // No tile to keep the fullscreen in
    WindowState::Floating(_) | WindowState::Minimized => {}
    WindowState::Fullscreen(_) => {
      let app = in_app_fullscreen(&window);
      state.spoof_fullscreen.insert(handle);
      if app {
        state.fake_fullscreen.insert(handle);
      }
      update_window_state(window.clone(), WindowState::Tiling, state, config)?;
      // Super+F's fullscreen: the app goes into its own, in its tile
      if !app {
        window.native().press_fullscreen_key();
      }
    }
    WindowState::Tiling => {
      state.spoof_fullscreen.insert(handle);
      if !state.fake_fullscreen.contains(&handle) {
        window.native().press_fullscreen_key();
      }
    }
  }

  Ok(())
}

/// Super+F (`toggle-fullscreen`) on a window in its own fullscreen inside
/// its tile (spoofed, Super+Alt+F): it becomes real fullscreen. An app's
/// real fullscreen takes the usual toggle, as in Hyprland: it leaves
/// fullscreen for its tile or float, the window's previous place restored
/// in one step (if the app insists on its fullscreen, its next own
/// fullscreen is real again: no fight). Returns whether it handled the
/// toggle; other windows take the usual one.
pub fn toggle_app_fullscreen(
  window: &WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<bool> {
  let handle = window.native().hwnd().0;

  match window.state() {
    WindowState::Tiling if state.fake_fullscreen.contains(&handle) => {
      state.fake_fullscreen.remove(&handle);
      state.spoof_fullscreen.remove(&handle);
      let defaults = &config.value.window_behavior.state_defaults.fullscreen;
      update_window_state(
        window.clone(),
        WindowState::Fullscreen(FullscreenStateConfig {
          shown_on_top: false,
          ..defaults.clone()
        }),
        state,
        config,
      )?;
      Ok(true)
    }
    _ => Ok(false),
  }
}
