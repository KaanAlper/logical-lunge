use anyhow::Context;
use tracing::info;
use wm_common::{TilingDirection, WmEvent, WorkspaceConfig};

use super::deactivate_workspace;
use crate::{
  commands::{
    container::{
      attach_container, move_container_within_tree, set_focused_descendant,
    },
    window::move_window_into_workspace,
  },
  models::{Monitor, WindowContainer, Workspace, SPECIAL_WORKSPACE},
  traits::{CommonGetters, PositionGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Hyprland's `togglespecialworkspace` (ii: Super+S): the scratchpad opens
/// over the focused monitor's workspace, dimming it, and closes again.
pub fn toggle_special_workspace(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let monitor = state
    .focused_container()
    .and_then(|focused| focused.monitor())
    .context("No focused monitor.")?;

  if monitor.special_shown() {
    hide_special_workspace(&monitor, state)
  } else {
    show_special_workspace(&monitor, state, config)
  }
}

/// Opens the special workspace on `monitor` (taking it from another monitor
/// if it is open there) and focuses its last focused window.
pub fn show_special_workspace(
  monitor: &Monitor,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  for other in state.monitors() {
    if other.id() != monitor.id() && other.special_shown() {
      hide_special_workspace(&other, state)?;
    }
  }

  let special = special_workspace_on(monitor, state, config)?;
  info!("Showing special workspace on {monitor}");
  monitor.set_special_shown(true);

  let to_focus = special
    .descendant_focus_order()
    .next()
    .unwrap_or_else(|| special.clone().into());
  set_focused_descendant(&to_focus, None);

  state
    .pending_sync
    .queue_focus_change()
    .queue_container_to_redraw(special.clone());

  sync_backdrop(state);
  state.emit_event(WmEvent::WorkspaceUpdated {
    updated_workspace: special.to_dto()?,
  });

  Ok(())
}

/// Closes the special workspace on `monitor`: its windows are hidden and
/// the focus goes back to the displayed workspace. An empty one is removed
/// (it is made again when needed).
pub fn hide_special_workspace(
  monitor: &Monitor,
  state: &mut WmState,
) -> anyhow::Result<()> {
  if !monitor.special_shown() {
    return Ok(());
  }

  info!("Hiding special workspace on {monitor}");
  monitor.set_special_shown(false);

  let Some(special) = monitor.special_workspace() else {
    sync_backdrop(state);
    return Ok(());
  };

  let focus_was_inside = state
    .focused_container()
    .and_then(|focused| focused.workspace())
    .is_some_and(|workspace| workspace.id() == special.id());

  if focus_was_inside {
    let displayed = monitor
      .displayed_workspace()
      .context("No displayed workspace.")?;
    let to_focus = displayed
      .descendant_focus_order()
      .next()
      .unwrap_or_else(|| displayed.clone().into());
    set_focused_descendant(&to_focus, None);
    state.pending_sync.queue_focus_change();
  }

  state.pending_sync.queue_container_to_redraw(special.clone());
  sync_backdrop(state);

  state.emit_event(WmEvent::WorkspaceUpdated {
    updated_workspace: special.to_dto()?,
  });

  if !special.has_children() {
    deactivate_workspace(special, state)?;
  }

  Ok(())
}

/// Hyprland's `movetoworkspacesilent special` (ii: Super+Alt+S): the window
/// goes to the special workspace, which stays as it is (open or closed);
/// the focus stays where the window was.
pub fn move_window_to_special_workspace(
  window: WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let monitor = window.monitor().context("No monitor.")?;

  // the special workspace where it is, else made on the window's monitor
  let special = match state.special_workspace() {
    Some(special) => special,
    None => special_workspace_on(&monitor, state, config)?,
  };

  if window.workspace().is_some_and(|workspace| workspace.id() == special.id()) {
    return Ok(());
  }

  move_window_into_workspace(window, special.clone(), state, config)?;

  state.emit_event(WmEvent::WorkspaceUpdated {
    updated_workspace: special.to_dto()?,
  });

  Ok(())
}

/// The special workspace attached to `monitor`: made on first use, or
/// moved from the monitor it was on (its tiled windows are laid out again
/// in this monitor's area; floating ones keep their place relative to it).
fn special_workspace_on(
  monitor: &Monitor,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<Workspace> {
  if let Some(special) = monitor.special_workspace() {
    return Ok(special);
  }

  if let Some(special) = state.special_workspace() {
    let old_monitor = special.monitor().context("No monitor.")?;
    old_monitor.set_special_shown(false);

    let old_rect = old_monitor.to_rect()?;
    let new_rect = monitor.to_rect()?;

    move_container_within_tree(
      &special.clone().into(),
      &monitor.clone().into(),
      monitor.child_count(),
      state,
    )?;

    for window in special
      .descendants()
      .filter_map(|c| c.as_window_container().ok())
    {
      let placement = window.floating_placement();
      window.set_floating_placement(placement.translate_to_coordinates(
        placement.x() + new_rect.x() - old_rect.x(),
        placement.y() + new_rect.y() - old_rect.y(),
      ));
    }

    state
      .pending_sync
      .queue_container_to_redraw(special.clone());

    return Ok(special);
  }

  let rect = monitor.to_rect()?;
  let tiling_direction = if rect.height() > rect.width() {
    TilingDirection::Vertical
  } else {
    TilingDirection::Horizontal
  };

  let special = Workspace::new(
    WorkspaceConfig {
      name: SPECIAL_WORKSPACE.to_string(),
      display_name: None,
      bind_to_monitor: None,
      keep_alive: false,
    },
    config.value.gaps.clone(),
    tiling_direction,
  );

  attach_container(
    &special.clone().into(),
    &monitor.clone().into(),
    None,
  )?;

  info!("Activating special workspace on {monitor}");

  Ok(special)
}

/// The dim behind the open special workspace (Hyprland's `dim_special`,
/// 0.2 in ii): over the monitor below the bar, under the special
/// workspace's windows.
pub fn sync_backdrop(state: &WmState) {
  #[cfg(target_os = "windows")]
  {
    use wm_platform::NativeWindowWindowsExt;

    let shown = state
      .monitors()
      .into_iter()
      .find(|monitor| monitor.special_shown());

    let target = shown.and_then(|monitor| {
      let bounds = monitor.to_rect().ok()?;
      // the bar's band stays undimmed (it is above the workspace in
      // Hyprland): the area starts where the displayed workspace starts,
      // less the outer gap the other edges keep
      let top = monitor
        .displayed_workspace()
        .and_then(|workspace| {
          let rect = workspace.to_rect().ok()?;
          let gap = (rect.x() - bounds.x()).max(0);
          Some((rect.y() - gap).max(bounds.y()))
        })
        .unwrap_or(bounds.y());
      let windows = monitor
        .special_workspace()
        .map(|special| {
          special
            .descendants()
            .filter_map(|c| c.as_window_container().ok())
            .map(|window| window.native().hwnd().0)
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();
      Some((
        (bounds.left, top, bounds.right, bounds.bottom),
        windows,
      ))
    });

    wm_borders::set_special_backdrop(target);
  }
  #[cfg(not(target_os = "windows"))]
  let _ = state;
}

/// Hyprland's `hide_special_on_workspace_change` (on in ii): a workspace
/// switch on a monitor closes the special workspace there.
pub fn hide_special_for_switch(
  monitor: Option<&Monitor>,
  state: &mut WmState,
) -> anyhow::Result<()> {
  match monitor {
    Some(monitor) if monitor.special_shown() => {
      hide_special_workspace(monitor, state)
    }
    _ => Ok(()),
  }
}
