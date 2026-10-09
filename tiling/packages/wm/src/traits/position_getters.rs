use ambassador::delegatable_trait;
use anyhow::Context;
use wm_common::TilingDirection;
use wm_platform::Rect;

use crate::{
  dwindle_math::{partition, Bx, Gaps},
  models::Container,
  traits::{CommonGetters, TilingDirectionGetters, TilingSizeGetters},
};

#[delegatable_trait]
pub trait PositionGetters {
  fn to_rect(&self) -> anyhow::Result<Rect>;
}

/// Logical Lunge: a rectangle as a dwindle box.
#[allow(clippy::cast_lossless)]
pub fn rect_box(rect: &Rect) -> Bx {
  Bx::new(
    rect.x() as f64,
    rect.y() as f64,
    rect.width() as f64,
    rect.height() as f64,
  )
}

/// Logical Lunge: the dwindle node box of a workspace or of a tiling
/// container in it (see `dwindle_math`): the workspace's work area cut by
/// the containers' shares along each split, with no gaps between boxes.
pub fn node_box(container: &Container) -> anyhow::Result<Bx> {
  if let Some(workspace) = container.as_workspace() {
    return Ok(rect_box(&workspace.to_rect()?));
  }

  let parent = container
    .parent()
    .and_then(|parent| parent.as_direction_container().ok())
    .context("Parent does not have a tiling direction.")?;

  let parent_box = node_box(&parent.clone().into())?;
  let children = parent.tiling_children().collect::<Vec<_>>();
  let index = children
    .iter()
    .position(|child| child.id() == container.id())
    .context("Container is not a tiling child of its parent.")?;
  let shares = children
    .iter()
    .map(|child| f64::from(child.tiling_size()))
    .collect::<Vec<_>>();

  let horizontal = parent.tiling_direction() == TilingDirection::Horizontal;
  Ok(partition(parent_box, horizontal, &shares)[index])
}

/// Logical Lunge: `gaps_in` of one side (half the space between two
/// windows) for a container.
#[allow(clippy::cast_lossless)]
pub fn side_gaps<T: TilingSizeGetters>(container: &T) -> anyhow::Result<Gaps> {
  let (horizontal_gap, vertical_gap) = container.inner_gaps()?;
  Ok(Gaps {
    h: horizontal_gap as f64 / 2.,
    v: vertical_gap as f64 / 2.,
  })
}

/// Implements the `PositionGetters` trait for tiling containers that can
/// be resized. This is used by `SplitContainer` and `TilingWindow`.
///
/// Logical Lunge: positioned like Hyprland's dwindle layout (see
/// `dwindle_math`). The container sits in its node box less `gaps_in` on
/// each side that does not touch the workspace's work area. Upstream cut
/// the parent's own rectangle after taking the gaps out of it, which put
/// uneven splits off by a few pixels per level and truncated heights
/// while widths were rounded.
#[macro_export]
macro_rules! impl_position_getters_as_resizable {
  // A tiled window: its frame is inset by the WM's border so that frame +
  // border fill the visible box (the border engine draws outside the
  // frame; Hyprland draws the border inside the window's cell).
  ($struct_name:ident, window) => {
    impl PositionGetters for $struct_name {
      fn to_rect(&self) -> anyhow::Result<Rect> {
        let workspace = self.workspace().context("No workspace.")?;
        let work = $crate::traits::rect_box(&workspace.to_rect()?);
        let node = $crate::traits::node_box(&self.clone().into())?;
        let gaps = $crate::traits::side_gaps(self)?;
        let visible = $crate::dwindle_math::window_box(node, work, gaps);

        let scale = self
          .monitor()
          .map_or(1., |monitor| monitor.native_properties().scale_factor);
        let (border_width, border_offset) = {
          let config = self.gaps_config();
          (config.window_border_width, config.window_border_offset)
        };
        let border =
          $crate::dwindle_math::border_px(border_width, border_offset, scale);
        let (x, y, width, height) =
          $crate::dwindle_math::inset(visible, border);

        Ok(Rect::from_xy(x, y, width, height))
      }
    }
  };
  ($struct_name:ident) => {
    impl PositionGetters for $struct_name {
      fn to_rect(&self) -> anyhow::Result<Rect> {
        let workspace = self.workspace().context("No workspace.")?;
        let work = $crate::traits::rect_box(&workspace.to_rect()?);
        let node = $crate::traits::node_box(&self.clone().into())?;
        let gaps = $crate::traits::side_gaps(self)?;
        let (x, y, width, height) =
          $crate::dwindle_math::window_box(node, work, gaps);

        Ok(Rect::from_xy(x, y, width, height))
      }
    }
  };
}
