use wm_platform::LengthValue;

use super::set_window_size;
use crate::{
  models::WindowContainer,
  traits::{CommonGetters, PositionGetters, TilingSizeGetters},
  wm_state::WmState,
};

pub fn resize_window(
  window: &WindowContainer,
  width_delta: Option<LengthValue>,
  height_delta: Option<LengthValue>,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let window_rect = window.to_rect()?;

  let target_width = match width_delta {
    Some(delta) => {
      let parent_width = match window.as_tiling_container() {
        // Logical Lunge: a percentage is of the parent's node length
        // (dwindle model, see `dwindle_math`)
        #[allow(clippy::cast_possible_truncation)]
        Ok(tiling_window) => tiling_window
          .container_to_resize(true)?
          .and_then(|container| {
            let parent = container.parent()?;
            Some(crate::traits::node_box(&parent).ok()?.w.round() as i32)
          }),
        _ => window.parent().and_then(|parent| {
          parent.to_rect().ok().map(|rect| rect.width())
        }),
      };

      parent_width.map(|parent_width| {
        window_rect.width() + delta.to_px(parent_width, None)
      })
    }
    _ => None,
  };

  let target_height = match height_delta {
    Some(delta) => {
      let parent_height = match window.as_tiling_container() {
        // Logical Lunge: a percentage is of the parent's node length
        // (dwindle model, see `dwindle_math`)
        #[allow(clippy::cast_possible_truncation)]
        Ok(tiling_window) => tiling_window
          .container_to_resize(false)?
          .and_then(|container| {
            let parent = container.parent()?;
            Some(crate::traits::node_box(&parent).ok()?.h.round() as i32)
          }),
        _ => window.parent().and_then(|parent| {
          parent.to_rect().ok().map(|rect| rect.height())
        }),
      };

      parent_height.map(|parent_height| {
        window_rect.height() + delta.to_px(parent_height, None)
      })
    }
    _ => None,
  };

  set_window_size(
    window.clone(),
    target_width.map(LengthValue::from_px),
    target_height.map(LengthValue::from_px),
    state,
  )?;

  Ok(())
}
