use crate::{
  models::Container,
  traits::{CommonGetters, TilingSizeGetters},
  wm_state::WmState,
};

/// Hyprland's `splitratio`: moves the split right above the focused tiled
/// window, whatever its direction. `delta` is in Hyprland's units (the
/// ratio runs 0.1–1.9 around 1.0): the first (left / top) child gets
/// `delta / 2` of the split, the second gives it up.
pub fn split_ratio(
  container: Container,
  delta: f32,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let Container::TilingWindow(window) = container else {
    return Ok(());
  };

  // the nearest ancestor with two tiled children is the split to move
  let mut child = window.as_tiling_container()?;
  let (first, second) = loop {
    let Some(parent) = child.parent() else { return Ok(()) };
    let children = parent.tiling_children().collect::<Vec<_>>();
    if children.len() >= 2 {
      let index = children.iter().position(|c| *c == child).unwrap_or(0);
      // the pair this window belongs to: itself and its next (or previous)
      // sibling; a binary tree has exactly these two
      let (a, b) = if index + 1 < children.len() {
        (index, index + 1)
      } else {
        (index - 1, index)
      };
      break (children[a].clone(), children[b].clone());
    }
    match parent.as_tiling_container() {
      Ok(up) => child = up,
      Err(_) => return Ok(()),
    }
  };

  let (a, b) =
    moved_split(first.tiling_size(), second.tiling_size(), delta);
  first.set_tiling_size(a);
  second.set_tiling_size(b);

  if let Some(workspace) = first.workspace() {
    state
      .pending_sync
      .queue_containers_to_redraw(workspace.tiling_children());
  }

  Ok(())
}

/// The two sizes after moving their split by `delta` (Hyprland units),
/// keeping their sum and each at least 10% of it.
fn moved_split(first: f32, second: f32, delta: f32) -> (f32, f32) {
  let total = first + second;
  if total <= 0. {
    return (first, second);
  }
  let share = (first / total + delta / 2.).clamp(0.1, 0.9);
  (share * total, (1. - share) * total)
}

#[cfg(test)]
mod tests {
  use super::moved_split;

  fn close(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
  }

  #[test]
  fn moves_by_half_the_delta_of_the_pair() {
    assert!(close(moved_split(0.5, 0.5, 0.1), (0.55, 0.45)));
    assert!(close(moved_split(0.5, 0.5, -0.1), (0.45, 0.55)));
    // a pair that is half of its parent keeps its sum
    assert!(close(moved_split(0.25, 0.25, 0.1), (0.275, 0.225)));
  }

  #[test]
  fn stays_within_limits() {
    assert!(close(moved_split(0.85, 0.15, 0.4), (0.9, 0.1)));
    assert!(close(moved_split(0.15, 0.85, -0.4), (0.1, 0.9)));
  }
}
