use std::time::{Duration, Instant};

use uuid::Uuid;
use wm_common::{
  ActiveDrag, ActiveDragOperation, FloatingStateConfig, TilingDirection,
  WindowState,
};
use wm_platform::{Point, Rect};

use super::update_window_state;
use crate::{
  commands::container::{flatten_split_container, set_focused_descendant},
  events::handle_window_moved_or_resized_end,
  models::{Container, WindowContainer},
  traits::{
    node_box, CommonGetters, TilingDirectionGetters, TilingSizeGetters,
    WindowGetters,
  },
  user_config::UserConfig,
  wm_state::WmState,
};

/// Hyprland's mouse binds (ii: Super + left / middle button moves a
/// window, Super + right button resizes it). The core's input hook only
/// reports the press and the release; the window follows the pointer here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseDragKind {
  Move,
  Resize,
}

#[derive(Clone, Debug)]
pub struct MouseDrag {
  pub kind: MouseDragKind,
  window_id: Uuid,
  started: Instant,
  start_cursor: Point,
  last_cursor: Point,
  /// The window's visible frame and its outer rect (with the invisible
  /// resize borders) when the drag started.
  start_frame: Rect,
  start_outer: Rect,
  /// The corner nearest to the pointer at the start: the one a resize
  /// moves (Hyprland's `smart_resizing`).
  left: bool,
  top: bool,
  /// A tiled window being resized: its splits move, the layout follows.
  tiled_resize: bool,
}

/// A drag the core never ended (its release was lost) ends on its own.
const MAX_DRAG: Duration = Duration::from_secs(120);
/// Smallest frame a floating resize leaves.
const MIN_FLOATING_SIZE: i32 = 50;

/// Starts moving or resizing the window under the pointer. A tiled window
/// being moved leaves the layout (the other tiles close the gap) and is
/// dropped back into it under the pointer at the end; a tiled window being
/// resized moves the splits at the grabbed corner.
pub fn start_mouse_drag(
  kind: MouseDragKind,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  end_mouse_drag(state, config)?;

  let cursor = state.dispatcher.cursor_position()?;
  let Some(native) = state.dispatcher.window_from_point(&cursor)? else {
    return Ok(());
  };
  let Some(window) = state.window_from_native(&native) else {
    return Ok(());
  };

  if matches!(
    window.state(),
    WindowState::Fullscreen(_) | WindowState::Minimized
  ) {
    return Ok(());
  }

  // Hyprland focuses the window it grabs
  set_focused_descendant(&window.clone().into(), None);
  state.pending_sync.queue_focus_change();

  let frame = window.native().frame()?;
  #[cfg(target_os = "windows")]
  let outer = {
    use wm_platform::NativeWindowWindowsExt;
    window.native().frame_with_shadows()?
  };
  #[cfg(not(target_os = "windows"))]
  let outer = frame.clone();
  let center = frame.center_point();
  let tiled = window.state() == WindowState::Tiling;
  let tiled_resize = tiled && kind == MouseDragKind::Resize;

  if !tiled_resize {
    window.set_active_drag(Some(ActiveDrag {
      operation: Some(match kind {
        MouseDragKind::Move => ActiveDragOperation::Move,
        MouseDragKind::Resize => ActiveDragOperation::Resize,
      }),
      is_from_floating: !tiled,
      initial_position: frame.clone(),
    }));
  }

  if tiled && kind == MouseDragKind::Move {
    lift_out_of_layout(&window, &frame, state, config)?;
  }

  state.mouse_drag = Some(MouseDrag {
    kind,
    window_id: window.id(),
    started: Instant::now(),
    start_cursor: cursor.clone(),
    last_cursor: cursor.clone(),
    start_frame: frame,
    start_outer: outer,
    left: cursor.x < center.x,
    top: cursor.y < center.y,
    tiled_resize,
  });

  Ok(())
}

/// The window floats at its tile's place and size while it is carried.
fn lift_out_of_layout(
  window: &WindowContainer,
  frame: &Rect,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let parent = window.parent();
  window.set_floating_placement(frame.clone());

  let window = update_window_state(
    window.clone(),
    WindowState::Floating(FloatingStateConfig {
      centered: false,
      shown_on_top: true,
    }),
    state,
    config,
  )?;

  // it stays where it is (the pointer moves it); the other tiles are
  // redrawn to fill its place
  state
    .pending_sync
    .dequeue_container_from_redraw(window.clone());

  if let Some(split_parent) = parent.as_ref().and_then(|parent| parent.as_split()) {
    if split_parent.child_count() == 1 {
      flatten_split_container(split_parent.clone())?;
      state
        .pending_sync
        .queue_containers_to_redraw(window.tiling_siblings());
    }
  }

  Ok(())
}

/// Follows the pointer (called on a short interval while a drag is on).
/// Returns whether the layout changed and a sync is needed.
pub fn mouse_drag_tick(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<bool> {
  let Some(drag) = state.mouse_drag.clone() else {
    return Ok(false);
  };

  if drag.started.elapsed() > MAX_DRAG {
    end_mouse_drag(state, config)?;
    return Ok(true);
  }

  let cursor = state.dispatcher.cursor_position()?;
  if (cursor.x, cursor.y) == (drag.last_cursor.x, drag.last_cursor.y) {
    return Ok(false);
  }

  let Some(window) = state
    .windows()
    .into_iter()
    .find(|window| window.id() == drag.window_id)
  else {
    state.mouse_drag = None;
    return Ok(false);
  };

  if let Some(current) = state.mouse_drag.as_mut() {
    current.last_cursor = cursor.clone();
  }

  let dx = cursor.x - drag.start_cursor.x;
  let dy = cursor.y - drag.start_cursor.y;

  if drag.tiled_resize {
    let step_x = cursor.x - drag.last_cursor.x;
    let step_y = cursor.y - drag.last_cursor.y;
    resize_tiled(&window, step_x, step_y, drag.left, drag.top)?;

    if let Some(workspace) = window.workspace() {
      state
        .pending_sync
        .queue_containers_to_redraw(workspace.tiling_children());
    }

    return Ok(true);
  }

  let (frame, outer) = match drag.kind {
    MouseDragKind::Move => (
      translated(&drag.start_frame, dx, dy),
      translated(&drag.start_outer, dx, dy),
    ),
    MouseDragKind::Resize => {
      let frame =
        resized_corner(&drag.start_frame, dx, dy, drag.left, drag.top);
      // the invisible borders around the frame stay as they were
      let outer = Rect::from_ltrb(
        frame.left - (drag.start_frame.left - drag.start_outer.left),
        frame.top - (drag.start_frame.top - drag.start_outer.top),
        frame.right + (drag.start_outer.right - drag.start_frame.right),
        frame.bottom + (drag.start_outer.bottom - drag.start_frame.bottom),
      );
      (frame, outer)
    }
  };

  window.set_floating_placement(frame.clone());
  place_window(&window, &frame, &outer, drag.kind == MouseDragKind::Move);

  Ok(false)
}

#[cfg(target_os = "windows")]
fn place_window(window: &WindowContainer, frame: &Rect, outer: &Rect, move_only: bool) {
  use wm_platform::{
    NativeWindowWindowsExt, WindowZOrder, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
  };

  let mut flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
  if move_only {
    flags |= SWP_NOSIZE;
  }

  if let Err(err) =
    window
      .native()
      .set_window_pos(&WindowZOrder::Normal, outer, flags)
  {
    tracing::warn!("Failed to move dragged window: {}", err);
  }

  // the border moves in the same step
  wm_borders::place(
    window.native().hwnd().0,
    frame.left,
    frame.top,
    frame.right,
    frame.bottom,
  );
}

#[cfg(not(target_os = "windows"))]
fn place_window(_: &WindowContainer, _: &Rect, _: &Rect, _: bool) {}

/// Ends the drag: a carried tiled window is dropped into the layout under
/// the pointer, a floating one keeps its new place (on the monitor it was
/// dropped on).
pub fn end_mouse_drag(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let Some(drag) = state.mouse_drag.take() else {
    return Ok(());
  };

  let Some(window) = state
    .windows()
    .into_iter()
    .find(|window| window.id() == drag.window_id)
  else {
    return Ok(());
  };

  if drag.tiled_resize {
    state.pending_sync.queue_container_to_redraw(window);
    return Ok(());
  }

  if let Ok(frame) = window.native().frame() {
    window.update_native_properties(|properties| {
      properties.frame = frame;
    });
  }

  if let WindowContainer::NonTilingWindow(non_tiling) = &window {
    non_tiling.set_has_custom_floating_placement(true);
  }

  handle_window_moved_or_resized_end(&window, state, config)
}

fn translated(rect: &Rect, dx: i32, dy: i32) -> Rect {
  rect.translate_to_coordinates(rect.x() + dx, rect.y() + dy)
}

/// `rect` with its grabbed corner moved by the pointer's travel, never
/// smaller than `MIN_FLOATING_SIZE` (the opposite corner stays put).
pub(crate) fn resized_corner(rect: &Rect, dx: i32, dy: i32, left: bool, top: bool) -> Rect {
  let (mut l, mut t, mut r, mut b) = (rect.left, rect.top, rect.right, rect.bottom);

  if left {
    l = (l + dx).min(r - MIN_FLOATING_SIZE);
  } else {
    r = (r + dx).max(l + MIN_FLOATING_SIZE);
  }

  if top {
    t = (t + dy).min(b - MIN_FLOATING_SIZE);
  } else {
    b = (b + dy).max(t + MIN_FLOATING_SIZE);
  }

  Rect::from_ltrb(l, t, r, b)
}

/// Hyprland's dwindle `resizeActiveWindow` with `smart_resizing`: going up
/// from the window, the first split of each direction whose shared edge is
/// the grabbed one moves by the pointer's travel ("outer"); a split of the
/// same direction below it that the window is on the far side of
/// ("inner") is moved back, so the window's other edge stays put.
fn resize_tiled(
  window: &WindowContainer,
  dx: i32,
  dy: i32,
  left: bool,
  top: bool,
) -> anyhow::Result<()> {
  let mut h_outer: Option<Container> = None;
  let mut h_inner: Option<Container> = None;
  let mut v_outer: Option<Container> = None;
  let mut v_inner: Option<Container> = None;

  let mut current: Container = window.clone().into();

  while let Some(parent) = current.parent() {
    let Ok(direction_parent) = parent.as_direction_container() else {
      break;
    };

    let children = direction_parent.tiling_children().collect::<Vec<_>>();

    if children.len() == 2 {
      let is_first = children[0].id() == current.id();

      if direction_parent.tiling_direction() == TilingDirection::Vertical {
        if v_outer.is_none() && grabbed_side(top, is_first) {
          v_outer = Some(current.clone());
        } else if v_outer.is_none() && v_inner.is_none() {
          v_inner = Some(current.clone());
        }
      } else if h_outer.is_none() && grabbed_side(left, is_first) {
        h_outer = Some(current.clone());
      } else if h_outer.is_none() && h_inner.is_none() {
        h_inner = Some(current.clone());
      }
    }

    if h_outer.is_some() && v_outer.is_some() {
      break;
    }

    current = parent;
  }

  move_split(h_outer, h_inner, f64::from(dx), true)?;
  move_split(v_outer, v_inner, f64::from(dy), false)?;

  Ok(())
}

/// The grabbed edge (left / top when `near_start`) is the one this child
/// shares with its sibling: the second child's start edge, the first
/// child's end edge.
pub(crate) fn grabbed_side(near_start: bool, is_first: bool) -> bool {
  near_start != is_first
}

fn move_split(
  outer: Option<Container>,
  inner: Option<Container>,
  delta: f64,
  horizontal: bool,
) -> anyhow::Result<()> {
  let Some(outer) = outer else {
    return Ok(());
  };

  if delta == 0.0 {
    return Ok(());
  }

  let length = |container: &Container| -> anyhow::Result<f64> {
    let node = node_box(container)?;
    Ok(if horizontal { node.w } else { node.h })
  };

  let inner_original = match &inner {
    Some(inner) => Some(length(inner)?),
    None => None,
  };

  let Some(outer_parent) = outer.parent() else {
    return Ok(());
  };

  let outer_length = length(&outer_parent)?;
  set_first_share(&outer_parent, |first| moved_first_share(first, delta, outer_length));

  if let (Some(inner), Some(original)) = (inner, inner_original) {
    if let Some(inner_parent) = inner.parent() {
      let inner_length = length(&inner_parent)?;
      let is_first = inner_parent
        .tiling_children()
        .next()
        .is_some_and(|first| first.id() == inner.id());
      set_first_share(&inner_parent, |_| {
        inner_first_share(original, delta, inner_length, is_first)
      });
    }
  }

  Ok(())
}

/// Sets the first child's share of a two-child split (the pair keeps its
/// total size).
fn set_first_share(parent: &Container, share: impl FnOnce(f64) -> f64) {
  let children = parent.tiling_children().collect::<Vec<_>>();
  let [first, second] = children.as_slice() else {
    return;
  };

  let total = f64::from(first.tiling_size() + second.tiling_size());
  if total <= 0.0 {
    return;
  }

  let new_share = share(f64::from(first.tiling_size()) / total);

  #[allow(clippy::cast_possible_truncation)]
  {
    first.set_tiling_size((new_share * total) as f32);
    second.set_tiling_size(((1.0 - new_share) * total) as f32);
  }
}

/// The first child's new share after its split moved by `delta` px over a
/// split `length` px long, within Hyprland's ratio range (0.1..1.9, each
/// side 5%..95%).
pub(crate) fn moved_first_share(first: f64, delta: f64, length: f64) -> f64 {
  if length <= 0.0 {
    return first;
  }

  (first + delta / length).clamp(0.05, 0.95)
}

/// The inner split's first share keeping the window's far edge in place:
/// the window (`original` px long) grows by the travel on its grabbed side
/// within its parent, now `length` px long.
pub(crate) fn inner_first_share(original: f64, delta: f64, length: f64, window_first: bool) -> f64 {
  if length <= 0.0 {
    return 0.5;
  }

  let share = if window_first {
    (original - delta) / length
  } else {
    1.0 - (original + delta) / length
  };

  share.clamp(0.05, 0.95)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_grabbed_edge_is_the_one_shared_with_the_sibling() {
    // right edge grabbed: the window must be the first (left) child
    assert!(grabbed_side(false, true));
    assert!(!grabbed_side(false, false));
    // left edge grabbed: the window must be the second (right) child
    assert!(grabbed_side(true, false));
    assert!(!grabbed_side(true, true));
  }

  #[test]
  fn moving_a_split_follows_the_pointer_within_hyprlands_range() {
    assert!((moved_first_share(0.5, 100.0, 1000.0) - 0.6).abs() < 1e-9);
    assert!((moved_first_share(0.5, -100.0, 1000.0) - 0.4).abs() < 1e-9);
    assert!((moved_first_share(0.9, 200.0, 1000.0) - 0.95).abs() < 1e-9);
    assert!((moved_first_share(0.1, -200.0, 1000.0) - 0.05).abs() < 1e-9);
  }

  #[test]
  fn the_inner_split_keeps_the_far_edge() {
    // window second in a 1000 px split, 400 px wide, right edge dragged by
    // +50 after the outer split grew the parent to 1050: 450 px of 1050
    let share = inner_first_share(400.0, 50.0, 1050.0, false);
    assert!(((1.0 - share) * 1050.0 - 450.0).abs() < 1e-6);
    // window first, left edge dragged by -50: it grows to 450 px
    let share = inner_first_share(400.0, -50.0, 1050.0, true);
    assert!((share * 1050.0 - 450.0).abs() < 1e-6);
  }

  #[test]
  fn a_floating_resize_moves_only_the_grabbed_corner() {
    let rect = Rect::from_ltrb(100, 100, 500, 400);
    assert_eq!(resized_corner(&rect, 20, 30, false, false), Rect::from_ltrb(100, 100, 520, 430));
    assert_eq!(resized_corner(&rect, -20, -30, true, true), Rect::from_ltrb(80, 70, 500, 400));
    // never below the minimum: the opposite corner stays
    assert_eq!(resized_corner(&rect, 1000, 0, true, false), Rect::from_ltrb(450, 100, 500, 400));
  }
}
