use wm_common::{FloatingStateConfig, WindowState};

use super::{set_window_position, update_window_state, WindowPositionTarget};
use crate::{
  commands::{
    container::move_container_within_tree,
    workspace::destroy_empty_workspaces,
  },
  models::WindowContainer,
  traits::{CommonGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Hyprland's pin: a floating window shown on every workspace of its
/// monitor, above the tiled windows. A tiled window floats first (ii binds
/// Super+P for any window); only a floating window stays pinned, so
/// tiling or fullscreening it ends the pin.
pub fn set_pinned(
  window: WindowContainer,
  pinned: bool,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let window = if pinned && !matches!(window.state(), WindowState::Floating(_)) {
    let window = update_window_state(
      window,
      // shown on top: a pinned window stays above the tiled ones
      WindowState::Floating(FloatingStateConfig { centered: true, shown_on_top: true }),
      state,
      config,
    )?;

    if !window.has_custom_floating_placement() {
      set_window_position(window.clone(), &WindowPositionTarget::Centered, state)?;
    }

    window
  } else {
    window
  };

  let WindowContainer::NonTilingWindow(non_tiling) = &window else {
    return Ok(());
  };

  if !matches!(non_tiling.state(), WindowState::Floating(_)) {
    return Ok(());
  }

  non_tiling.set_pinned(pinned);
  // the z-order (shown on top) is applied by the redraw
  state.pending_sync.queue_container_to_redraw(window.clone());

  Ok(())
}

/// Whether `window` is a pinned floating window.
pub fn is_pinned(window: &WindowContainer) -> bool {
  match window {
    WindowContainer::NonTilingWindow(window) => {
      window.is_pinned() && matches!(window.state(), WindowState::Floating(_))
    }
    WindowContainer::TilingWindow(_) => false,
  }
}

/// Pinned windows follow their monitor's displayed workspace: run before
/// every redraw, after whatever command changed the displayed workspace,
/// so the window is never hidden with the workspace it was on (and never
/// slides with it: it is already on the new one when the switch is drawn).
pub fn carry_pinned_windows(state: &mut WmState) -> anyhow::Result<()> {
  let mut carried = false;

  for window in state.windows() {
    if !is_pinned(&window) {
      continue;
    }

    let (Some(monitor), Some(workspace)) = (window.monitor(), window.workspace()) else {
      continue;
    };

    // the special workspace keeps its windows (a pinned one sent there
    // stays there, as in Hyprland)
    if workspace.is_special() {
      continue;
    }

    let Some(displayed) = monitor.displayed_workspace() else {
      continue;
    };

    if displayed.id() == workspace.id() {
      continue;
    }

    move_container_within_tree(
      &window.clone().into(),
      &displayed.clone().into(),
      displayed.child_count(),
      state,
    )?;
    carried = true;
  }

  // the workspace it left may be empty now
  if carried {
    destroy_empty_workspaces(state)?;
  }

  Ok(())
}
