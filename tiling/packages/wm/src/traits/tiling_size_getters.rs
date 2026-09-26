use std::cell::Ref;

use ambassador::delegatable_trait;
use anyhow::Context;
use wm_common::{GapsConfig, TilingDirection};

use super::{CommonGetters, TilingDirectionGetters};
use crate::models::{DirectionContainer, TilingContainer};

pub const MIN_TILING_SIZE: f32 = 0.01;

#[delegatable_trait]
pub trait TilingSizeGetters: CommonGetters {
  fn tiling_size(&self) -> f32;

  fn set_tiling_size(&self, tiling_size: f32);

  fn gaps_config(&self) -> Ref<'_, GapsConfig>;

  fn set_gaps_config(&self, gaps_config: GapsConfig);

  /// Gets the horizontal and vertical gaps between windows in pixels.
  fn inner_gaps(&self) -> anyhow::Result<(i32, i32)> {
    let monitor = self.monitor().context("No monitor.")?;
    let monitor_rect = monitor.native_properties().bounds;
    let gaps_config = self.gaps_config();

    let scale_factor = if gaps_config.scale_with_dpi {
      monitor.native_properties().scale_factor
    } else {
      1.
    };

    Ok((
      gaps_config
        .inner_gap
        .to_px(monitor_rect.height(), Some(scale_factor)),
      gaps_config
        .inner_gap
        .to_px(monitor_rect.width(), Some(scale_factor)),
    ))
  }

  /// Gets the container to resize when resizing a tiling window.
  ///
  /// Logical Lunge: like Hyprland's dwindle layout, that's the window or
  /// the nearest ancestor that sits in a split along the resize axis
  /// (next to its split partner). Between the window and that container
  /// there are only splits across the axis, so both have the same length
  /// along it. Upstream only looked at the parent and grandparent, which
  /// assumed that nested splits alternate direction; in the binary
  /// dwindle tree a split can have its parent's direction (e.g. in
  /// `V[4 V[H[2 3] 1]]`, a width resize of 1 resized the inner `V` inside
  /// the outer `V`, i.e. its height). `None` when no split runs along the
  /// axis (nothing to resize, as in Hyprland).
  fn container_to_resize(
    &self,
    is_width_resize: bool,
  ) -> anyhow::Result<Option<TilingContainer>> {
    let axis = if is_width_resize {
      TilingDirection::Horizontal
    } else {
      TilingDirection::Vertical
    };

    let mut current: TilingContainer = self.as_tiling_container()?;

    loop {
      let parent = current
        .parent()
        .context("No parent.")?
        .as_direction_container()?;

      if parent.tiling_direction() == axis
        && current.tiling_siblings().count() > 0
      {
        return Ok(Some(current));
      }

      match parent {
        DirectionContainer::Split(split) => current = split.into(),
        DirectionContainer::Workspace(_) => return Ok(None),
      }
    }
  }
}

/// Implements the `TilingSizeGetters` trait for a given struct.
///
/// Expects that the struct has a wrapping `RefCell` containing a struct
/// with a `tiling_size` field.
#[macro_export]
macro_rules! impl_tiling_size_getters {
  ($struct_name:ident) => {
    impl TilingSizeGetters for $struct_name {
      fn tiling_size(&self) -> f32 {
        self.0.borrow().tiling_size
      }

      fn set_tiling_size(&self, tiling_size: f32) {
        self.0.borrow_mut().tiling_size = tiling_size;
      }

      fn gaps_config(&self) -> Ref<'_, GapsConfig> {
        Ref::map(self.0.borrow(), |inner| &inner.gaps_config)
      }

      fn set_gaps_config(&self, gaps_config: GapsConfig) {
        self.0.borrow_mut().gaps_config = gaps_config;
      }
    }
  };
}
