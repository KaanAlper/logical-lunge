use anyhow::Context;
use tracing::info;

use super::activate_workspace;
use crate::{
  commands::{
    container::set_focused_descendant, workspace::deactivate_workspace,
  },
  models::WorkspaceTarget,
  traits::CommonGetters,
  user_config::UserConfig,
  wm_state::WmState,
};

/// Focuses a workspace by a given target.
///
/// This target can be a workspace name, the most recently focused
/// workspace, the next workspace, the previous workspace, or the workspace
/// in a given direction from the currently focused workspace.
///
/// The workspace will be activated if it isn't already active.
pub fn focus_workspace(
  target: WorkspaceTarget,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let focused_workspace = state
    .focused_container()
    .and_then(|focused| focused.workspace())
    .context("No workspace is currently focused.")?;

  let (target_workspace_name, target_workspace) =
    state.workspace_by_target(&focused_workspace, target, config)?;

  // Retrieve or activate the target workspace by its name.
  let target_workspace = match target_workspace {
    Some(_) => anyhow::Ok(target_workspace),
    _ => match target_workspace_name {
      Some(name) => {
        activate_workspace(Some(&name), None, state, config)?;

        Ok(state.workspace_by_name(&name))
      }
      _ => Ok(None),
    },
  }?;

  if let Some(target_workspace) = target_workspace {
    info!("Focusing workspace: {target_workspace}");

    // Get the currently displayed workspace on the same monitor that the
    // workspace to focus is on.
    let displayed_workspace = target_workspace
      .monitor()
      .and_then(|monitor| monitor.displayed_workspace())
      .context("No workspace is currently displayed.")?;

    // Set focus to whichever window last had focus in workspace. If the
    // workspace has no windows, then set focus to the workspace itself.
    let container_to_focus = target_workspace
      .descendant_focus_order()
      .next()
      .unwrap_or_else(|| target_workspace.clone().into());

    set_focused_descendant(&container_to_focus, None);
    state.pending_sync.queue_focus_change();

    // Display the workspace to switch focus to.
    state
      .pending_sync
      .queue_container_to_redraw(displayed_workspace)
      .queue_container_to_redraw(target_workspace);

    destroy_empty_workspaces(state)?;

    // Save the currently focused workspace as recent.
    state.recent_workspace_name = Some(focused_workspace.config().name);
    state.pending_sync.queue_cursor_jump();
  }

  Ok(())
}

/// Destroys every empty, hidden workspace that isn't kept alive (only one
/// went per focus change, so empty workspaces piled up and the "active
/// workspace" navigation still stopped at them). A displayed workspace
/// stays, so a monitor always keeps one.
pub fn destroy_empty_workspaces(state: &mut WmState) -> anyhow::Result<()> {
  let empty: Vec<_> = state
    .workspaces()
    .into_iter()
    .filter(|workspace| is_disposable(
      workspace.config().keep_alive,
      workspace.has_children(),
      workspace.is_displayed(),
    ))
    .collect();

  for workspace in empty {
    deactivate_workspace(workspace, state)?;
  }

  Ok(())
}

fn is_disposable(keep_alive: bool, has_children: bool, displayed: bool) -> bool {
  !keep_alive && !has_children && !displayed
}

#[cfg(test)]
mod tests {
  use super::is_disposable;

  #[test]
  fn only_empty_hidden_unkept_workspaces_go() {
    assert!(is_disposable(false, false, false));
    assert!(!is_disposable(true, false, false));
    assert!(!is_disposable(false, true, false));
    assert!(!is_disposable(false, false, true));
  }
}
