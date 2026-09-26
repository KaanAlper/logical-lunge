use anyhow::Context;
use wm_common::{try_warn, FullscreenStateConfig, WindowState};
use wm_platform::LengthValue;

use crate::{
  commands::{
    container::move_container_within_tree,
    window::{
      dwindle_split, set_window_size, update_window_state, DwindlePlacement,
    },
  },
  events::update_floating_window_position,
  models::{NonTilingWindow, TilingContainer, TilingWindow, WindowContainer},
  traits::{CommonGetters, PositionGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

/// Handles the event for when a window is finished being moved or resized
/// by the user (e.g. via the window's drag handles).
///
/// This resizes the window if it's a tiling window and attach a dragged
/// floating window.
///
/// TODO: Move this to a better location - maybe a new `active_drag_ext`
/// mod.
pub fn handle_window_moved_or_resized_end(
  window: &WindowContainer,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let Some(active_drag) = window.active_drag() else {
    return Ok(());
  };

  match &window {
    WindowContainer::NonTilingWindow(window) => {
      let is_maximized = try_warn!(window.native().is_maximized());

      window.update_native_properties(|properties| {
        properties.is_maximized = is_maximized;
      });

      let nearest_monitor = state
        .nearest_monitor(&window.native())
        .context("Failed to get workspace of nearest monitor.")?;

      let should_fullscreen = window.should_fullscreen(
        &nearest_monitor
          .displayed_workspace()
          .context("No workspace.")?,
      )?;

      if is_maximized || should_fullscreen {
        let fullscreen_state = if let WindowState::Fullscreen(
          fullscreen_state,
        ) = window.state()
        {
          fullscreen_state
        } else {
          config
            .value
            .window_behavior
            .state_defaults
            .fullscreen
            .clone()
        };

        let window = update_window_state(
          window.clone().into(),
          WindowState::Fullscreen(FullscreenStateConfig {
            maximized: is_maximized,
            ..fullscreen_state
          }),
          state,
          config,
        )?;

        window.set_active_drag(None);

        if is_maximized {
          // Dequeue the window from redraw if it's maximized, since the
          // window is already in the correct state.
          state
            .pending_sync
            .dequeue_container_from_redraw(window.clone());
        } else {
          // Force a redraw to snap the window to the monitor edges.
          // TODO: Skip redraw if it's already matches fullscreen frame.
          state.pending_sync.queue_container_to_redraw(window.clone());
        }

        return Ok(());
      }

      if active_drag.is_from_floating {
        update_floating_window_position(
          window,
          window.native_properties().frame,
          &nearest_monitor,
          state,
        )?;
        window.set_active_drag(None);
      } else {
        // Window is a temporary floating window that should be
        // reverted back to tiling.
        let window = drop_as_tiling_window(window, state, config)?;
        window.set_active_drag(None);
      }
    }
    WindowContainer::TilingWindow(window) => {
      tracing::info!(
        "Tiling window move/resize ended: {}",
        window.as_window_container()?
      );

      let frame = window.native_properties().frame;

      // Update the window's size based on the new frame position. This
      // means we use the actual window dimensions as the source of truth.
      set_window_size(
        window.clone().into(),
        Some(LengthValue::from_px(frame.width())),
        Some(LengthValue::from_px(frame.height())),
        state,
      )?;

      window.set_active_drag(None);

      // Force a redraw of the window to snap it back to its original
      // position. This is necessary when:
      // - The window is the only tiling window in the workspace.
      // - The window is not past the movement threshold for transitioning
      //   to floating while being dragged.
      // - Resizing in a direction that doesn't change the window's tiling
      //   size.
      state.pending_sync.queue_container_to_redraw(window.clone());
    }
  }

  Ok(())
}

/// Handles transition from temporary floating window to tiling window on
/// drag end.
///
/// Logical Lunge: the window is dropped like a new window in Hyprland's
/// dwindle layout: the tiling window under the cursor (or the nearest
/// one) is split along its longer side and the dropped window takes the
/// half under the cursor. Upstream inserted it next to the nearest
/// container, which made splits with more than two children.
fn drop_as_tiling_window(
  moved_window: &NonTilingWindow,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<WindowContainer> {
  tracing::info!(
    "Tiling window drag ended: {}",
    moved_window.as_window_container()?
  );

  let mouse_pos = state.dispatcher.cursor_position()?;
  let mouse_workspace = state
    .monitor_at_point(&mouse_pos)
    .and_then(|monitor| monitor.displayed_workspace())
    .or_else(|| moved_window.workspace())
    .context("Couldn't find workspace for window drop.")?;

  // The tiling window under the cursor, or else the nearest one.
  let mut target: Option<(i64, TilingWindow)> = None;

  for container in mouse_workspace.descendants() {
    let Ok(TilingContainer::TilingWindow(window)) =
      container.as_tiling_container()
    else {
      continue;
    };

    if window.id() == moved_window.id() {
      continue;
    }

    // Squared distance from the cursor to the window (0 when over it).
    let rect = window.to_rect()?;
    let dx = i64::from((rect.left - mouse_pos.x).max(mouse_pos.x - rect.right).max(0));
    let dy = i64::from((rect.top - mouse_pos.y).max(mouse_pos.y - rect.bottom).max(0));
    let distance = dx * dx + dy * dy;

    if target.as_ref().is_none_or(|(nearest, _)| distance < *nearest) {
      target = Some((distance, window));
    }
  }

  // An empty workspace: the window fills it.
  let Some((_, target)) = target else {
    move_container_within_tree(
      &moved_window.clone().into(),
      &mouse_workspace.clone().into(),
      0,
      state,
    )?;

    moved_window.set_insertion_target(None);

    return update_window_state(
      moved_window.as_window_container()?,
      WindowState::Tiling,
      state,
      config,
    );
  };

  let moved_window = update_window_state(
    moved_window.clone().into(),
    WindowState::Tiling,
    state,
    config,
  )?;

  if let WindowContainer::TilingWindow(tiling_window) = &moved_window {
    let old_workspace = tiling_window.workspace();

    dwindle_split(
      tiling_window,
      &target,
      &mouse_pos,
      DwindlePlacement::New,
      config,
    )?;

    if let Some(old_workspace) = old_workspace {
      state
        .pending_sync
        .queue_containers_to_redraw(old_workspace.tiling_children());
    }
  }

  state
    .pending_sync
    .queue_containers_to_redraw(mouse_workspace.tiling_children());

  Ok(moved_window)
}
