use crate::{
  models::TilingContainer,
  traits::{CommonGetters, TilingSizeGetters, MIN_TILING_SIZE},
};

pub fn resize_tiling_container(
  container_to_resize: &TilingContainer,
  target_size: f32,
) {
  let tiling_siblings =
    container_to_resize.tiling_siblings().collect::<Vec<_>>();

  // Ignore cases where the container is the only child.
  if tiling_siblings.is_empty() {
    container_to_resize.set_tiling_size(1.);
    return;
  }

  // Prevent the container from being smaller than the minimum size, and
  // larger than the space available from sibling containers.
  // Logical Lunge: a binary dwindle split keeps Hyprland's ratio range
  // (0.1..1.9, i.e. each side 5%..95% of the split).
  #[allow(clippy::cast_precision_loss)]
  let clamped_target_size = if tiling_siblings.len() == 1 {
    target_size.clamp(0.05, 0.95)
  } else {
    target_size.clamp(
      MIN_TILING_SIZE,
      1. - (tiling_siblings.len() as f32 * MIN_TILING_SIZE),
    )
  };

  let size_delta = clamped_target_size - container_to_resize.tiling_size();
  container_to_resize.set_tiling_size(clamped_target_size);

  // Get available tiling size amongst siblings.
  let available_size =
    tiling_siblings.iter().fold(0.0, |sum, container| {
      sum + container.tiling_size() - MIN_TILING_SIZE
    });

  // Distribute the available tiling size amongst its siblings.
  for sibling in &tiling_siblings {
    // Get percentage of resize that affects this container. Siblings are
    // resized in proportion to their current size (i.e. larger containers
    // are shrunk more).
    let resize_factor =
      (sibling.tiling_size() - MIN_TILING_SIZE) / available_size;

    let size_delta = resize_factor * size_delta;

    sibling.set_tiling_size(sibling.tiling_size() - size_delta);
  }
}
