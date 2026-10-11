use anyhow::Context;
use uuid::Uuid;
use wm_common::WindowState;
use wm_platform::Rect;

use crate::{
  commands::window::place_free_floating,
  models::{WindowContainer, Workspace},
  traits::{CommonGetters, PositionGetters, WindowGetters},
  wm_state::WmState,
};

pub enum WindowPositionTarget {
  Centered,
  Coordinates(Option<i32>, Option<i32>),
  /// Logical Lunge: centred when that spot is free, otherwise where it
  /// covers the least of the workspace's other floating windows (see
  /// `place_free_floating`).
  Free,
}

pub fn set_window_position(
  window: WindowContainer,
  target: &WindowPositionTarget,
  state: &mut WmState,
) -> anyhow::Result<()> {
  if matches!(window.state(), WindowState::Floating(_)) {
    let placement = window.floating_placement();

    let new_placement = match target {
      WindowPositionTarget::Centered => placement.translate_to_center(
        &window.workspace().context("No workspace.")?.to_rect()?,
      ),
      WindowPositionTarget::Free => free_floating_placement(
        &placement,
        &window.workspace().context("No workspace.")?,
        Some(window.id()),
      )?,
      WindowPositionTarget::Coordinates(target_x, target_y) => {
        Rect::from_xy(
          target_x.unwrap_or(placement.x()),
          target_y.unwrap_or(placement.y()),
          placement.width(),
          placement.height(),
        )
      }
    };

    window.set_floating_placement(new_placement);

    // TODO: `has_custom_floating_placement` should be marked `true` if
    // manually positioned to be centered (e.g. via `position --centered`).
    let is_centered = matches!(
      target,
      WindowPositionTarget::Centered | WindowPositionTarget::Free
    );
    window.set_has_custom_floating_placement(!is_centered);

    state.pending_sync.queue_container_to_redraw(window);
  }

  Ok(())
}

/// Logical Lunge: where a floating window with no owner and no position of
/// its own opens on `workspace`: in the middle when that spot is free,
/// otherwise beside the workspace's other floating windows (all but
/// `window_id`), so floating windows don't open on top of each other.
pub fn free_floating_placement(
  frame: &Rect,
  workspace: &Workspace,
  window_id: Option<Uuid>,
) -> anyhow::Result<Rect> {
  let others = workspace
    .descendants()
    .filter_map(|container| container.as_window_container().ok())
    .filter(|other| {
      matches!(other.state(), WindowState::Floating(_))
        && Some(other.id()) != window_id
    })
    .map(|other| other.floating_placement())
    .collect::<Vec<_>>();

  Ok(place_free_floating(frame, &workspace.to_rect()?, &others))
}
