use anyhow::Context;
use wm_common::{TilingDirection, WindowState, WmEvent};
use wm_platform::{Direction, Point, Rect};

use crate::{
  commands::container::{
    attach_container, detach_container, flatten_child_split_containers,
    flatten_split_container,
    normalize_split_containers, set_focused_descendant, wrap_in_split_container,
  },
  models::{
    Monitor, NonTilingWindow, SplitContainer, TilingContainer,
    TilingWindow, WindowContainer, Workspace,
  },
  traits::{
    CommonGetters, PositionGetters, TilingDirectionGetters, WindowGetters,
  },
  user_config::UserConfig,
  wm_state::WmState,
};

/// The distance in pixels to snap the window to the monitor's edge.
const SNAP_DISTANCE: i32 = 15;

/// How far (in pixels) a window may be from the focal point across the
/// move to count as the window in that direction: the focal point can
/// land in the gap between two windows.
const GAP_TOLERANCE: i32 = 50;

pub fn move_window_in_direction(
  window: WindowContainer,
  direction: &Direction,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  match window {
    WindowContainer::TilingWindow(window) => {
      let workspace = window.workspace();
      move_tiling_window(window, direction, state, config)?;

      // Leftover single-child / same-direction splits would make later
      // moves produce extra columns.
      if let Some(workspace) = workspace {
        normalize_split_containers(&workspace.clone().into())?;
        state
          .pending_sync
          .queue_containers_to_redraw(workspace.tiling_children());
      }

      Ok(())
    }
    WindowContainer::NonTilingWindow(non_tiling_window) => {
      match non_tiling_window.state() {
        WindowState::Floating(_) => {
          move_floating_window(non_tiling_window, direction, state)
        }
        // A fullscreen window stays on its monitor: changing monitor is the
        // monitor shortcuts' job (move --workspace-in-direction), never a
        // side effect of Super+Shift+arrow.
        _ => Ok(()),
      }
    }
  }
}

/// Moves a tiling window like Hyprland's dwindle layout (`movewindow`).
///
/// A focal point is taken 1px outside the window's edge in the given
/// direction. The window is removed from the tree (its split partner
/// takes its place), and the window at the focal point is split along
/// its longer side; the moved window takes the half that the focal point
/// falls into. For example, in the layout H[1 V[2 3]] where container 2
/// is moved left, this results in H[V[2 1] 3].
///
/// Moving straight toward the nearest split divider when the split
/// partner is a single window swaps the two instead (Hyprland's
/// direction override), e.g. H[1 2] where 1 is moved right gives H[2 1].
///
/// Without a window in the given direction, the window takes that half of
/// the workspace (see `move_tiling_window_fallback`).
fn move_tiling_window(
  window_to_move: TilingWindow,
  direction: &Direction,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let rect = window_to_move.to_rect()?;
  let focal_point = match direction {
    Direction::Up => Point {
      x: rect.left + rect.width() / 2,
      y: rect.top - 1,
    },
    Direction::Down => Point {
      x: rect.left + rect.width() / 2,
      y: rect.bottom + 1,
    },
    Direction::Left => Point {
      x: rect.left - 1,
      y: rect.top + rect.height() / 2,
    },
    Direction::Right => Point {
      x: rect.right + 1,
      y: rect.top + rect.height() / 2,
    },
  };

  let workspace = window_to_move.workspace().context("No workspace.")?;
  let had_focus = window_to_move.has_focus(None);

  if let Some(partner) = partner_to_swap_with(&window_to_move, direction) {
    dwindle_split(
      &window_to_move,
      &partner,
      &focal_point,
      DwindlePlacement::Swap(direction),
      config,
    )?;
  } else {
    let Some(target) =
      window_in_direction(&window_to_move, &focal_point, direction)?
    else {
      return move_tiling_window_fallback(
        window_to_move,
        direction,
        state,
        config,
      );
    };

    dwindle_split(
      &window_to_move,
      &target,
      &focal_point,
      DwindlePlacement::Moved,
      config,
    )?;
  }

  if had_focus {
    set_focused_descendant(&window_to_move.clone().into(), None);
    state.emit_event(WmEvent::FocusedContainerMoved {
      focused_container: window_to_move.to_dto()?,
    });
  }

  state
    .pending_sync
    .queue_containers_to_redraw(workspace.tiling_children())
    .queue_cursor_jump();

  Ok(())
}

/// The window's split partner, if moving in the given direction swaps
/// the two (Hyprland's `movewindow` direction override): the split is
/// along the move axis, the window moves toward the divider, and the
/// partner is a single window rather than a split.
fn partner_to_swap_with(
  window: &TilingWindow,
  direction: &Direction,
) -> Option<TilingWindow> {
  let parent = window.direction_container()?;

  if parent.tiling_direction() != TilingDirection::from_direction(direction)
  {
    return None;
  }

  let children = parent.tiling_children().collect::<Vec<_>>();
  let [first, second] = children.as_slice() else {
    return None;
  };

  let (partner, window_is_first) = if first.id() == window.id() {
    (second, true)
  } else if second.id() == window.id() {
    (first, false)
  } else {
    return None;
  };

  let toward_partner = match direction {
    Direction::Up | Direction::Left => !window_is_first,
    Direction::Down | Direction::Right => window_is_first,
  };

  match partner {
    TilingContainer::TilingWindow(partner) if toward_partner => {
      Some(partner.clone())
    }
    _ => None,
  }
}

/// Where `dwindle_split` puts the window in the split target.
#[derive(Clone, Copy, Debug)]
pub enum DwindlePlacement<'a> {
  /// A new (or re-tiled) window: the half on the point's side of the
  /// target's middle (Hyprland's `force_split = 0`, also for a point
  /// outside the target); without a cursor the point is far right/below,
  /// so the second half, which builds Hyprland's spiral.
  New,
  /// `movewindow`: the half the focal point falls into (as in Hyprland,
  /// that's the half next to where the window came from).
  Moved,
  /// `movewindow` toward the window's split partner: the split is along
  /// the move axis and the window takes the far side, which swaps the
  /// two.
  Swap(&'a Direction),
}

/// Inserts `window` by splitting `target` (Hyprland's dwindle
/// `onWindowRemovedTiling` + `onWindowCreatedTiling`).
///
/// The window is removed first (its split partner takes its place), so
/// that the target is measured the way it'll be split. The target is
/// split along its longer side, and the window takes a half as described
/// by `placement`.
pub fn dwindle_split(
  window: &TilingWindow,
  target: &TilingWindow,
  point: &Point,
  placement: DwindlePlacement,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let workspace = target.workspace().context("No workspace.")?;
  let old_workspace = window.workspace();
  let window_to_move = window.clone();

  // The window may already be detached (e.g. a new window).
  if let Some(old_parent) = window.parent() {
    detach_container(window_to_move.clone().into())?;

    if let Some(old_parent) = old_parent.as_split().cloned() {
      if old_parent.child_count() == 1 && old_parent.parent().is_some() {
        flatten_split_container(old_parent)?;
      }
    }
  }

  // `movewindow` (Hyprland `movedTarget`): with the window out of the
  // tree, the node closest to the focal point is split — measured now that
  // its split partner has taken the window's place.
  let target = match placement {
    DwindlePlacement::Moved => {
      closest_tiling_window(&workspace, window, point).unwrap_or_else(|| target.clone())
    }
    _ => target.clone(),
  };
  let target = &target;

  // Decided on the target's node box (its share of the work area, before
  // gaps), as Hyprland does with the node it splits.
  let target_box = crate::traits::node_box(&target.clone().into())?;

  let (split_direction, is_first) = match placement {
    DwindlePlacement::Swap(direction) => (
      TilingDirection::from_direction(direction),
      matches!(direction, Direction::Up | Direction::Left),
    ),
    DwindlePlacement::New | DwindlePlacement::Moved => {
      // force_split = 0: the point's side of the box's middle, also when
      // the point is outside the box (a point at i32::MAX, i.e. no cursor,
      // takes the second half)
      #[allow(clippy::cast_lossless)]
      let is_first = crate::dwindle_math::new_is_first(
        target_box,
        (point.x as f64, point.y as f64),
      );

      let split_direction = if crate::dwindle_math::side_by_side(target_box) {
        TilingDirection::Horizontal
      } else {
        TilingDirection::Vertical
      };

      (split_direction, is_first)
    }
  };

  tracing::debug!(
    "dwindle_split: target_id={} node={:?} placement={:?} point={:?} split_dir={:?} is_first={}",
    target.id(),
    target_box,
    placement,
    point,
    split_direction,
    is_first
  );

  let target_parent = target
    .direction_container()
    .context("No direction container.")?;

  if target.tiling_siblings().count() == 0 {
    // Target fills its parent (i.e. it's alone on the workspace), so the
    // workspace itself is split.
    target_parent.set_tiling_direction(split_direction);

    attach_container(
      &window_to_move.clone().into(),
      &target_parent.clone().into(),
      Some(target.index() + usize::from(!is_first)),
    )?;
  } else {
    let split_container =
      SplitContainer::new(split_direction, config.value.gaps.clone());

    wrap_in_split_container(
      &split_container,
      &target_parent.clone().into(),
      &[target.clone().into()],
    )?;

    attach_container(
      &window_to_move.clone().into(),
      &split_container.into(),
      Some(usize::from(!is_first)),
    )?;
  }

  flatten_child_split_containers(&workspace.clone().into())?;

  if let Some(old_workspace) = old_workspace {
    if old_workspace.id() != workspace.id() {
      flatten_child_split_containers(&old_workspace.into())?;
    }
  }

  Ok(())
}

/// The tiling window of the workspace whose node box is closest to the
/// point (Hyprland's `getClosestNode`), other than `exclude`.
fn closest_tiling_window(
  workspace: &Workspace,
  exclude: &TilingWindow,
  point: &Point,
) -> Option<TilingWindow> {
  let candidates = workspace
    .descendants()
    .filter_map(|container| match container.as_tiling_container() {
      Ok(TilingContainer::TilingWindow(other)) if other.id() != exclude.id() => {
        crate::traits::node_box(&other.clone().into())
          .ok()
          .map(|node| (other, node))
      }
      _ => None,
    })
    .collect::<Vec<_>>();
  let boxes = candidates.iter().map(|(_, node)| *node).collect::<Vec<_>>();
  #[allow(clippy::cast_lossless)]
  crate::dwindle_math::closest((point.x as f64, point.y as f64), &boxes)
    .map(|index| candidates[index].0.clone())
}

/// Gets the tiling window on the same workspace at the focal point.
///
/// The focal point may land in the gap between windows (e.g. moving the
/// left one of `H[1 V[2 3]]` right, where the focal point is at the
/// height of the gap between 2 and 3). The nearest window lying in the
/// given direction is used then; Hyprland finds one there through the
/// windows' enlarged input areas.
fn window_in_direction(
  window: &TilingWindow,
  focal_point: &Point,
  direction: &Direction,
) -> anyhow::Result<Option<TilingWindow>> {
  let workspace = window.workspace().context("No workspace.")?;
  let mut nearest: Option<(i64, TilingWindow)> = None;

  for other in workspace.descendants() {
    let Ok(TilingContainer::TilingWindow(other)) =
      other.as_tiling_container()
    else {
      continue;
    };

    if other.id() == window.id() {
      continue;
    }

    let r = other.to_rect()?;

    if r.contains_point(focal_point) {
      return Ok(Some(other));
    }

    // Distance along the move (the window must lie beyond the focal
    // point) and across it (0 when the window spans the focal point).
    let (along, across) = match direction {
      Direction::Left => (
        focal_point.x - r.right,
        (r.top - focal_point.y).max(focal_point.y - r.bottom),
      ),
      Direction::Right => (
        r.left - focal_point.x,
        (r.top - focal_point.y).max(focal_point.y - r.bottom),
      ),
      Direction::Up => (
        focal_point.y - r.bottom,
        (r.left - focal_point.x).max(focal_point.x - r.right),
      ),
      Direction::Down => (
        r.top - focal_point.y,
        (r.left - focal_point.x).max(focal_point.x - r.right),
      ),
    };

    // Only across the gap between windows, not a window off to the side.
    if along < 0 || across > GAP_TOLERANCE {
      continue;
    }

    let along = i64::from(along);
    let across = i64::from(across.max(0));
    let distance = along * along + across * across;

    if nearest.as_ref().is_none_or(|(d, _)| distance < *d) {
      nearest = Some((distance, other));
    }
  }

  Ok(nearest.map(|(_, window)| window))
}

/// Moves a tiling window that has no window in the given direction, i.e.
/// it's at the workspace's edge on that side.
///
/// If the window already is the workspace's half on that side (or alone
/// on the workspace), it goes to the workspace of the monitor in that
/// direction, as upstream. Otherwise it takes that half of the workspace
/// and the rest of the layout keeps its shape in the other half.
fn move_tiling_window_fallback(
  window_to_move: TilingWindow,
  direction: &Direction,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let parent = window_to_move
    .direction_container()
    .context("No direction container.")?;

  let has_matching_tiling_direction = parent.tiling_direction()
    == TilingDirection::from_direction(direction);

  // Nothing beyond this edge on the workspace: the window stays put. It
  // used to cross to the neighbouring monitor; changing monitor is the
  // monitor shortcuts' job.
  if parent.is_workspace()
    && (has_matching_tiling_direction
      || window_to_move.tiling_siblings().count() == 0)
  {
    return Ok(());
  }

  let workspace = window_to_move.workspace().context("No workspace.")?;
  move_to_workspace_edge(window_to_move, &workspace, direction, state, config)
}

/// Moves a tiling window to the workspace's edge in the given direction,
/// where it takes half of the workspace; the rest of the layout keeps its
/// shape in the other half (the root of the binary dwindle tree is split
/// again). For example, in V[1 H[2 3]] where container 3 is moved right,
/// this results in H[V[1 2] 3], and a 2x2 grid H[V[1 2] V[3 4]] where 3
/// is moved right results in H[H[V[1 2] 4] 3].
fn move_to_workspace_edge(
  window_to_move: TilingWindow,
  workspace: &Workspace,
  direction: &Direction,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let had_focus = window_to_move.has_focus(None);

  // Its split partner takes its place.
  detach_container(window_to_move.clone().into())?;
  flatten_child_split_containers(&workspace.clone().into())?;

  let rest = workspace.tiling_children().collect::<Vec<_>>();

  if rest.len() > 1 {
    let split_container = SplitContainer::new(
      workspace.tiling_direction(),
      config.value.gaps.clone(),
    );

    wrap_in_split_container(
      &split_container,
      &workspace.clone().into(),
      &rest,
    )?;
  }

  workspace.set_tiling_direction(TilingDirection::from_direction(direction));

  let target_index = match direction {
    Direction::Left | Direction::Up => 0,
    _ => workspace.child_count(),
  };

  attach_container(
    &window_to_move.clone().into(),
    &workspace.clone().into(),
    Some(target_index),
  )?;

  flatten_child_split_containers(&workspace.clone().into())?;

  if had_focus {
    set_focused_descendant(&window_to_move.clone().into(), None);
    state.emit_event(WmEvent::FocusedContainerMoved {
      focused_container: window_to_move.to_dto()?,
    });
  }

  state
    .pending_sync
    .queue_containers_to_redraw(workspace.tiling_children())
    .queue_cursor_jump();

  Ok(())
}

fn move_floating_window(
  window_to_move: NonTilingWindow,
  direction: &Direction,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let new_position =
    new_floating_position(&window_to_move, direction, state)?;

  if let Some((position_rect, target_monitor)) = new_position {
    let monitor = window_to_move.monitor().context("No monitor.")?;

    // Mark window as needing DPI adjustment if it crosses monitors. The
    // handler for `PlatformEvent::LocationChanged` will update the
    // window's workspace if it goes out of bounds of its current
    // workspace.
    if monitor.id() != target_monitor.id()
      && monitor.has_dpi_difference(&target_monitor.into())?
    {
      window_to_move.set_has_pending_dpi_adjustment(true);
    }

    window_to_move.set_floating_placement(position_rect);
    state.pending_sync.queue_container_to_redraw(window_to_move);
  }

  Ok(())
}

/// Returns a tuple of the new floating position and the target monitor.
fn new_floating_position(
  window_to_move: &NonTilingWindow,
  direction: &Direction,
  state: &mut WmState,
) -> anyhow::Result<Option<(Rect, Monitor)>> {
  let monitor = window_to_move.monitor().context("No monitor.")?;
  let monitor_rect = monitor.native_properties().working_area;
  let window_pos = window_to_move.native_properties().frame;

  let is_on_monitor_edge = match direction {
    Direction::Up => window_pos.top == monitor_rect.top,
    Direction::Down => window_pos.bottom == monitor_rect.bottom,
    Direction::Left => window_pos.left == monitor_rect.left,
    Direction::Right => window_pos.right == monitor_rect.right,
  };

  // Window is on the edge of the monitor and should be moved to a
  // different monitor in the given direction.
  if is_on_monitor_edge {
    let next_monitor = state.monitor_in_direction(&monitor, direction)?;

    if let Some(next_monitor) = next_monitor {
      let monitor_rect = next_monitor.native().working_area()?.clone();

      let position = snap_to_monitor_edge(
        &window_pos,
        &monitor_rect,
        &direction.inverse(),
      )
      .clamp(&monitor_rect);

      return Ok(Some((position, next_monitor)));
    }

    return Ok(None);
  }

  let (monitor_length, window_length) = match direction {
    Direction::Up | Direction::Down => {
      (monitor_rect.height(), window_pos.height())
    }
    _ => (monitor_rect.width(), window_pos.width()),
  };

  let length_delta = monitor_length - window_length;

  // Calculate the distance the window should move based on the ratio of
  // the window's length to the monitor's length.
  #[allow(clippy::cast_precision_loss)]
  let move_distance = match window_length as f32 / monitor_length as f32 {
    x if (0.0..0.2).contains(&x) => length_delta / 5,
    x if (0.2..0.4).contains(&x) => length_delta / 4,
    x if (0.4..0.6).contains(&x) => length_delta / 3,
    _ => length_delta / 2,
  };

  // Snap the window to the current monitor's edge if it's within 15px of
  // it after the move.
  let should_snap_to_edge = match direction {
    Direction::Up => {
      window_pos.top - move_distance - SNAP_DISTANCE < monitor_rect.top
    }
    Direction::Down => {
      window_pos.bottom + move_distance + SNAP_DISTANCE
        > monitor_rect.bottom
    }
    Direction::Left => {
      window_pos.left - move_distance - SNAP_DISTANCE < monitor_rect.left
    }
    Direction::Right => {
      window_pos.right + move_distance + SNAP_DISTANCE > monitor_rect.right
    }
  };

  if should_snap_to_edge {
    let position =
      snap_to_monitor_edge(&window_pos, &monitor_rect, direction);

    return Ok(Some((position, monitor)));
  }

  // Snap the window to the current monitor's inverse edge if it's in
  // between two monitors or outside the bounds of the current monitor.
  let should_snap_to_inverse_edge = match direction {
    Direction::Up => window_pos.bottom > monitor_rect.bottom,
    Direction::Down => window_pos.top < monitor_rect.top,
    Direction::Left => window_pos.right > monitor_rect.right,
    Direction::Right => window_pos.left < monitor_rect.left,
  };

  let position = if should_snap_to_inverse_edge {
    snap_to_monitor_edge(&window_pos, &monitor_rect, &direction.inverse())
  } else {
    window_pos.translate_in_direction(direction, move_distance)
  };

  Ok(Some((position, monitor)))
}

fn snap_to_monitor_edge(
  window_pos: &Rect,
  monitor_rect: &Rect,
  edge: &Direction,
) -> Rect {
  let (x, y) = match edge {
    Direction::Up => (window_pos.x(), monitor_rect.top),
    Direction::Down => {
      (window_pos.x(), monitor_rect.bottom - window_pos.height())
    }
    Direction::Left => (monitor_rect.left, window_pos.y()),
    Direction::Right => {
      (monitor_rect.right - window_pos.width(), window_pos.y())
    }
  };

  window_pos.translate_to_coordinates(x, y)
}
