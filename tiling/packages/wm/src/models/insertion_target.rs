use wm_common::TilingDirection;

use crate::models::Container;

#[derive(Debug, Clone)]
pub struct InsertionTarget {
  pub target_parent: Container,
  pub target_index: usize,
  pub prev_tiling_size: f32,
  pub prev_sibling_count: usize,
  /// Logical Lunge: the tile next to it when it left the layout, and on
  /// which side it was: its place when the split it was in is gone (a
  /// two-tile split collapses as soon as one of them leaves it).
  pub neighbour: Option<Container>,
  pub direction: Option<TilingDirection>,
  pub was_first: bool,
}
