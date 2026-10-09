use anyhow::Context;
use tracing::{info, warn};
use wm_common::{TilingDirection, WindowState};
use wm_platform::{Direction, Point};

use crate::{
  commands::{
    container::{
      move_container_within_tree, normalize_split_containers,
      replace_container,
    },
    window::{dwindle_place, dwindle_split, DwindlePlacement},
  },
  models::{Container, InsertionTarget, TilingContainer, TilingWindow, WindowContainer},
  traits::{CommonGetters, TilingDirectionGetters, TilingSizeGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Updates the state of a window.
///
/// Adds the window for redraw if there is a state change.
///
/// Returns the window after the state change.
pub fn update_window_state(
  window: WindowContainer,
  target_state: WindowState,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<WindowContainer> {
  if window.state() == target_state {
    return Ok(window);
  }

  info!("Updating window state: {:?}.", target_state);

  match target_state {
    WindowState::Tiling => set_tiling(&window, state, config),
    _ => set_non_tiling(window, target_state, state),
  }
}

/// Updates the state of a window to be `WindowState::Tiling`.
fn set_tiling(
  window: &WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<WindowContainer> {
  let window = window
    .as_non_tiling_window()
    .context("Invalid window state.")?
    .clone();

  let workspace =
    window.workspace().context("Window has no workspace.")?;

  // Coming out of fullscreen or maximize it goes back to its tile (as in
  // Hyprland, where fullscreen never takes a window out of its layout).
  let memory = matches!(window.state(), WindowState::Fullscreen(_))
    .then(|| window.insertion_target())
    .flatten();

  let tiling_window = window.to_tiling(config.value.gaps.clone());

  // Replace the original window with the created tiling window.
  replace_container(
    &tiling_window.clone().into(),
    &window.parent().context("No parent.")?,
    window.index(),
  )?;

  // Like Hyprland, a window that becomes tiled again (e.g. restored after
  // Win+D minimized everything, or a floating one tiled) is re-inserted
  // with the dwindle split of the last focused tiling window instead of
  // returning to its old column.
  if !restore_tile(&tiling_window, memory, state, config)? {
    dwindle_place(&tiling_window, false, state, config)?;
  }
  normalize_split_containers(&workspace.clone().into())?;

  let target_parent = tiling_window.parent().context("No parent.")?;

  state
    .pending_sync
    .queue_containers_to_redraw(target_parent.tiling_children())
    .queue_workspace_to_reorder(workspace);

  Ok(tiling_window.into())
}

/// Puts a window back where it was in its workspace's layout before it left
/// it (fullscreen, maximize): in its old split at its old index, or, when
/// that split collapsed (two tiles, one left), split again from the tile
/// that was next to it, on the same side and along the same direction.
/// Its old share of the split comes back too. False when neither place
/// exists any more (it then goes where a new window would).
fn restore_tile(
  window: &TilingWindow,
  memory: Option<InsertionTarget>,
  state: &WmState,
  config: &UserConfig,
) -> anyhow::Result<bool> {
  let Some(memory) = memory else { return Ok(false) };
  let workspace_id = window.workspace().map(|workspace| workspace.id());
  let here = |container: &Container| {
    !container.is_detached()
      && container.workspace().map(|workspace| workspace.id()) == workspace_id
  };

  if here(&memory.target_parent)
    && memory.target_parent.as_direction_container().is_ok()
  {
    let index = memory
      .target_index
      .min(memory.target_parent.child_count());
    move_container_within_tree(
      &window.clone().into(),
      &memory.target_parent,
      index,
      state,
    )?;
    restore_share(window, memory.prev_tiling_size);
    return Ok(true);
  }

  if let (Some(neighbour), Some(direction)) =
    (&memory.neighbour, &memory.direction)
  {
    if let Ok(TilingContainer::TilingWindow(neighbour)) =
      neighbour.as_tiling_container()
    {
      if here(&neighbour.clone().into()) {
        let side = match (direction, memory.was_first) {
          (TilingDirection::Horizontal, true) => Direction::Left,
          (TilingDirection::Horizontal, false) => Direction::Right,
          (TilingDirection::Vertical, true) => Direction::Up,
          (TilingDirection::Vertical, false) => Direction::Down,
        };
        dwindle_split(
          window,
          &neighbour,
          &Point { x: 0, y: 0 },
          DwindlePlacement::Swap(&side),
          config,
        )?;
        restore_share(window, memory.prev_tiling_size);
        return Ok(true);
      }
    }
  }

  Ok(false)
}

/// The window takes `size` of its split again; its siblings share the rest
/// in their current proportions.
fn restore_share(window: &TilingWindow, size: f32) {
  if !(size > 0.0 && size < 1.0) {
    return;
  }
  let siblings = window.tiling_siblings().collect::<Vec<_>>();
  let rest: f32 = siblings.iter().map(TilingSizeGetters::tiling_size).sum();
  if siblings.is_empty() || rest <= 0.0 {
    return;
  }
  for sibling in &siblings {
    sibling.set_tiling_size(sibling.tiling_size() * (1.0 - size) / rest);
  }
  window.set_tiling_size(size);
}

/// Updates the state of a window to be either `WindowState::Floating`,
/// `WindowState::Fullscreen`, or `WindowState::Minimized`.
fn set_non_tiling(
  window: WindowContainer,
  target_state: WindowState,
  state: &mut WmState,
) -> anyhow::Result<WindowContainer> {
  // A window can only be updated to a minimized state if it is
  // natively minimized.
  // TODO: Consider doing the same for maximized and fullscreen states.
  // (the live state: the cached one can be stale, e.g. a window managed as
  // shown while still minimized)
  if target_state == WindowState::Minimized
    && !window.native().is_minimized().unwrap_or(false)
  {
    info!("No window state update. Minimizing window.");

    // TODO: Instead of doing the platform call directly here, instead add
    // a `queue_state_change` method to `PendingSync`.
    if let Err(err) = window.native().minimize() {
      warn!("Failed to minimize window: {}", err);
    }

    return Ok(window);
  }

  let workspace = window.workspace().context("No workspace.")?;

  match window {
    WindowContainer::NonTilingWindow(window) => {
      let current_state = window.state();

      // Update the window's previous state if the discriminant changes.
      // TODO: Move out handling of active drag. Can then simplify calls to
      // `set_active_drag` in `handle_window_moved_or_resized_end`.
      if !current_state.is_same_state(&target_state)
        && window.active_drag().is_none()
      {
        window.set_prev_state(current_state);
        state.pending_sync.queue_workspace_to_reorder(workspace);
      }

      window.set_state(target_state);
      state.pending_sync.queue_container_to_redraw(window.clone());

      Ok(window.into())
    }
    WindowContainer::TilingWindow(window) => {
      let parent = window.parent().context("No parent")?;

      // The tile next to it, and its side of the split, for coming back
      // (Hyprland keeps a fullscreen window in the layout; here it leaves
      // the tree and returns to where it was, see `restore_tile`).
      let neighbour = window
        .prev_siblings()
        .find(|sibling| sibling.as_tiling_container().is_ok())
        .map(|sibling| (sibling, false))
        .or_else(|| {
          window
            .next_siblings()
            .find(|sibling| sibling.as_tiling_container().is_ok())
            .map(|sibling| (sibling, true))
        });
      let direction = parent
        .as_direction_container()
        .ok()
        .map(|parent| parent.tiling_direction());

      let non_tiling_window = window.to_non_tiling(
        target_state.clone(),
        Some(InsertionTarget {
          target_parent: parent.clone(),
          target_index: window.index(),
          prev_tiling_size: window.tiling_size(),
          prev_sibling_count: window.tiling_siblings().count(),
          was_first: neighbour.as_ref().is_some_and(|(_, first)| *first),
          neighbour: neighbour.map(|(sibling, _)| sibling),
          direction,
        }),
      );

      // Non-tiling windows should always be direct children of the
      // workspace.
      if parent != workspace.clone().into() {
        move_container_within_tree(
          &window.clone().into(),
          &workspace.clone().into(),
          workspace.child_count(),
          state,
        )?;
      }

      replace_container(
        &non_tiling_window.clone().into(),
        &workspace.clone().into(),
        window.index(),
      )?;

      // Logical Lunge: e.g. V[1 H[2 3]] with 3 minimized left H[2] behind
      // (a single-child split, which later moves turned into odd rows).
      normalize_split_containers(&workspace.clone().into())?;

      state
        .pending_sync
        .queue_container_to_redraw(non_tiling_window.clone())
        .queue_containers_to_redraw(workspace.tiling_children())
        .queue_workspace_to_reorder(workspace);

      Ok(non_tiling_window.into())
    }
  }
}
