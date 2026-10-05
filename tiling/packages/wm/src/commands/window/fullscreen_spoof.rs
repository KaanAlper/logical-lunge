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

/// Super+F (`toggle-fullscreen`) on an app's own fullscreen. Down: the
/// window goes into its tile for this fullscreen (the app stays in it; it
/// would otherwise be made fullscreen again at its next move). Up: a window
/// in its own fullscreen inside its tile becomes real fullscreen. Returns
/// whether it handled the toggle; other windows take the usual one.
pub fn toggle_app_fullscreen(
  window: &WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<bool> {
  let handle = window.native().hwnd().0;

  match window.state() {
    WindowState::Fullscreen(_) if in_app_fullscreen(window) => {
      state.fake_fullscreen.insert(handle);
      update_window_state(window.clone(), WindowState::Tiling, state, config)?;
      Ok(true)
    }
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
