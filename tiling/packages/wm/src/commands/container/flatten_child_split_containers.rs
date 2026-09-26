use super::{
  detach_container, flatten_split_container, wrap_in_split_container,
};
use crate::{
  models::{Container, SplitContainer, TilingContainer, Workspace},
  traits::{CommonGetters, TilingDirectionGetters, TilingSizeGetters},
};

/// Tidies the layout of the workspace that the given container is in.
///
/// Logical Lunge: the layout is a binary tree like Hyprland's dwindle
/// layout (see `collapse_split_containers`). Upstream merged split
/// containers with their parent's direction here (`H[1 H[2 3]]` became
/// `H[1 2 3]`), which lost which window is whose split partner.
pub fn flatten_child_split_containers(
  parent: &Container,
) -> anyhow::Result<()> {
  collapse_split_containers(parent)
}

/// Tidies the layout of every workspace in or around the given container
/// (see `collapse_split_containers`).
pub fn normalize_split_containers(root: &Container) -> anyhow::Result<()> {
  collapse_split_containers(root)
}

/// Keeps the layout a binary tree like Hyprland's dwindle layout: every
/// split has exactly two children and its own ratio, so
/// `H[1 H[2 3]]` (1 | 2 and 3 split in half) is not the same layout as
/// `H[1 2 3]`: closing 3 gives 2 the whole right half.
///
/// - A split container with a single child is removed and the child takes
///   its place and size (the removed window's split partner fills their
///   split, as in Hyprland).
/// - A workspace with a single split child takes over its direction and
///   children.
/// - A container with more than two tiling children (e.g. a tree from an
///   older version) is made binary without changing the layout on
///   screen: `H[1 2 3]` becomes `H[1 H[2 3]]`.
///
/// Splits with their parent's direction are kept.
pub fn collapse_split_containers(
  container: &Container,
) -> anyhow::Result<()> {
  let workspaces = match container.workspace() {
    Some(workspace) => vec![workspace],
    None => container
      .self_and_descendants()
      .filter_map(|container| container.as_workspace().cloned())
      .collect(),
  };

  for workspace in workspaces {
    // Every pass makes one change and a tree of n windows needs at most
    // about 2n of them; the limit only guards against a malformed tree.
    for _ in 0..256 {
      if !collapse_once(&workspace)? {
        break;
      }
    }
  }

  Ok(())
}

/// Makes one change towards a binary layout, if one is needed.
fn collapse_once(workspace: &Workspace) -> anyhow::Result<bool> {
  let root: Container = workspace.clone().into();

  // Deepest first, so that e.g. `H[V[V[1]] 2]` becomes `H[1 2]`.
  let splits = root
    .descendants()
    .filter_map(|container| container.as_split().cloned())
    .collect::<Vec<_>>();

  for split in splits.into_iter().rev() {
    if split.parent().is_none() {
      continue;
    }

    match split.tiling_children().count() {
      0 => {
        detach_container(split.into())?;
        return Ok(true);
      }
      1 => {
        flatten_split_container(split)?;
        return Ok(true);
      }
      _ => {}
    }
  }

  let tiling_children = workspace.tiling_children().collect::<Vec<_>>();

  if let [TilingContainer::Split(split)] = tiling_children.as_slice() {
    let tiling_direction = split.tiling_direction();
    flatten_split_container(split.clone())?;
    workspace.set_tiling_direction(tiling_direction);
    return Ok(true);
  }

  for container in root.self_and_descendants() {
    let Ok(direction_container) = container.as_direction_container() else {
      continue;
    };

    let tiling_children = container.tiling_children().collect::<Vec<_>>();

    if tiling_children.len() > 2 {
      let split = SplitContainer::new(
        direction_container.tiling_direction(),
        tiling_children[0].gaps_config().clone(),
      );

      wrap_in_split_container(&split, &container, &tiling_children[1..])?;
      return Ok(true);
    }
  }

  Ok(false)
}
