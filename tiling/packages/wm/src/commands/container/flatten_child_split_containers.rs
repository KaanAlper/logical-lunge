use super::flatten_split_container;
use crate::{
  models::Container,
  traits::{CommonGetters, TilingDirectionGetters},
};

/// Flattens any redundant split containers at the top-level of the given
/// parent container.
///
/// For example:
/// ```ignore,compile_fail
/// H[1 H[V[2, 3]]] -> H[1, 2, 3]
/// H[1 H[2, 3]] -> H[1, 2, 3]
/// H[V[1]] -> V[1]
/// ```
pub fn flatten_child_split_containers(
  parent: &Container,
) -> anyhow::Result<()> {
  if let Ok(parent) = parent.as_direction_container() {
    // Get children that are either tiling windows or split containers.
    let tiling_children = parent
      .children()
      .into_iter()
      .filter(|child| child.is_tiling_window() || child.is_split())
      .collect::<Vec<_>>();

    if tiling_children.len() == 1 {
      // Handle case where the parent is a split container and has a
      // single split container child.
      if let Some(split_child) = tiling_children[0].as_split() {
        flatten_split_container(split_child.clone())?;
        parent.set_tiling_direction(parent.tiling_direction().inverse());
      }
    } else {
      let split_children = tiling_children
        .into_iter()
        .filter_map(|child| child.as_split().cloned())
        .collect::<Vec<_>>();

      for split_child in split_children.iter().filter(|split_child| {
        split_child.tiling_direction() == parent.tiling_direction()
      }) {
        // Additionally flatten redundant top-level split containers in
        // the child.
        if split_child.child_count() == 1 {
          if let Some(split_grandchild) =
            split_child.children()[0].as_split()
          {
            flatten_split_container(split_grandchild.clone())?;
          }
        }

        flatten_split_container(split_child.clone())?;
      }
    }
  }

  Ok(())
}

/// Flattens every redundant split container below the given container:
/// split containers with a single child, and split containers with the
/// same tiling direction as their parent.
///
/// Deepest containers are handled first, so that e.g. `H[V[V[1]] 2]`
/// becomes `H[1 2]`.
pub fn normalize_split_containers(root: &Container) -> anyhow::Result<()> {
  let splits = root
    .descendants()
    .filter_map(|container| container.as_split().cloned())
    .collect::<Vec<_>>();

  for split in splits.into_iter().rev() {
    let Some(parent) = split.parent() else {
      continue;
    };

    let same_direction = parent
      .as_direction_container()
      .is_ok_and(|parent| parent.tiling_direction() == split.tiling_direction());

    if split.child_count() == 1 || same_direction {
      flatten_split_container(split)?;
    }
  }

  // A workspace left with a single split child takes over its direction.
  flatten_child_split_containers(root)
}
