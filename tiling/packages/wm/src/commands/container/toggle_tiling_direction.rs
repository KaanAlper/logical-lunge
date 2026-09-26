use anyhow::Context;
use wm_common::{TilingDirection, WmEvent};

use super::flatten_child_split_containers;
use crate::{
  models::{Container, DirectionContainer, TilingWindow},
  traits::{CommonGetters, TilingDirectionGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn toggle_tiling_direction(
  container: Container,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let direction_container = match container {
    Container::TilingWindow(tiling_window) => {
      toggle_window_direction(tiling_window, config)
    }
    Container::Workspace(workspace) => {
      workspace
        .set_tiling_direction(workspace.tiling_direction().inverse());

      Ok(workspace.into())
    }
    // Can only toggle tiling direction from a tiling window or workspace.
    _ => return Ok(()),
  }?;

  // The split turned: its windows move.
  if let Some(workspace) = direction_container.workspace() {
    state
      .pending_sync
      .queue_containers_to_redraw(workspace.tiling_children());
  }

  state.emit_event(WmEvent::TilingDirectionChanged {
    direction_container: direction_container.to_dto()?,
    new_tiling_direction: direction_container.tiling_direction(),
  });

  Ok(())
}

/// Logical Lunge: Hyprland's dwindle `togglesplit` (Super+J in ii). The
/// split that the window is in turns the other way, e.g. two windows side
/// by side become stacked. Upstream wrapped the window in a new split
/// container with the other direction instead; in the binary dwindle
/// layout that's a split with a single child, which is removed right away.
fn toggle_window_direction(
  tiling_window: TilingWindow,
  _config: &UserConfig,
) -> anyhow::Result<DirectionContainer> {
  let parent = tiling_window
    .direction_container()
    .context("No direction container.")?;

  parent.set_tiling_direction(parent.tiling_direction().inverse());

  if let Some(workspace) = tiling_window.workspace() {
    flatten_child_split_containers(&workspace.into())?;
  }

  Ok(parent)
}

pub fn set_tiling_direction(
  container: Container,
  state: &mut WmState,
  config: &UserConfig,
  tiling_direction: &TilingDirection,
) -> anyhow::Result<()> {
  let direction_container = container
    .direction_container()
    .context("No direction container.")?;

  if direction_container.tiling_direction() == *tiling_direction {
    Ok(())
  } else {
    toggle_tiling_direction(container, state, config)
  }
}
