use anyhow::Context;

use super::flatten_split_container;
use crate::{
  models::{Container, TilingContainer},
  traits::{CommonGetters, TilingSizeGetters, MIN_TILING_SIZE},
};

/// Removes a container from the tree.
///
/// If the container is a tiling container, the siblings will be resized to
/// fill the freed up space. Will flatten empty parent split containers.
#[allow(clippy::needless_pass_by_value)]
pub fn detach_container(child_to_remove: Container) -> anyhow::Result<()> {
  // Flatten the parent split container if it'll be empty after removing
  // the child.
  if let Some(split_parent) = child_to_remove
    .parent()
    .and_then(|parent| parent.as_split().cloned())
  {
    if split_parent.child_count() == 1 {
      flatten_split_container(split_parent)?;
    }
  }

  let parent = child_to_remove.parent().context("No parent.")?;

  // Position among the tiling children, for finding its split partner.
  let tiling_index = parent
    .tiling_children()
    .position(|c| c.id() == child_to_remove.id());

  parent
    .borrow_children_mut()
    .retain(|c| c.id() != child_to_remove.id());

  parent
    .borrow_child_focus_order_mut()
    .retain(|id| *id != child_to_remove.id());

  *child_to_remove.borrow_parent_mut() = None;

  // Resize the siblings if it is a tiling container.
  if let Ok(child_to_remove) = child_to_remove.as_tiling_container() {
    let tiling_siblings = parent.tiling_children().collect::<Vec<_>>();

    // Logical Lunge: like Hyprland's dwindle layout, the freed space goes
    // to the removed container's split partner.
    if let Some(partner) = tiling_index.and_then(|index| {
      split_partner(&tiling_siblings, index, child_to_remove.tiling_size())
    }) {
      partner.set_tiling_size(
        partner.tiling_size() + child_to_remove.tiling_size(),
      );

      return Ok(());
    }

    // TODO: Share logic with `resize_tiling_container`.
    let available_size =
      tiling_siblings.iter().fold(0.0, |sum, container| {
        sum + container.tiling_size() - MIN_TILING_SIZE
      });

    // Adjust size of the siblings based on the freed up space.
    for sibling in &tiling_siblings {
      let resize_factor =
        (sibling.tiling_size() - MIN_TILING_SIZE) / available_size;

      let size_delta = resize_factor * child_to_remove.tiling_size();
      sibling.set_tiling_size(sibling.tiling_size() + size_delta);
    }
  }

  Ok(())
}

/// Finds the sibling that takes over the space of a removed container, as
/// in Hyprland's dwindle layout where the removed window's split partner
/// fills their split.
///
/// New splits are 50/50, so in a flattened split the partner is an
/// adjacent sibling with the same size. Spreading the space over all
/// siblings instead skewed the layout a bit more with every move (e.g.
/// 20/80 rows). Returns `None` when all siblings have that size (equal
/// rows/columns, where an even spread is right) or when no adjacent
/// sibling has it (e.g. after a manual resize).
fn split_partner(
  siblings: &[TilingContainer],
  removed_index: usize,
  removed_size: f32,
) -> Option<TilingContainer> {
  const TOLERANCE: f32 = 0.01;

  let same_size = |sibling: &&TilingContainer| {
    (sibling.tiling_size() - removed_size).abs() < TOLERANCE
  };

  if siblings.iter().all(|sibling| same_size(&sibling)) {
    return None;
  }

  let previous = removed_index
    .checked_sub(1)
    .and_then(|index| siblings.get(index));
  let next = siblings.get(removed_index);

  previous
    .filter(same_size)
    .or_else(|| next.filter(same_size))
    .cloned()
}
