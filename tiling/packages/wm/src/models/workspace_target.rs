use wm_platform::Direction;

pub enum WorkspaceTarget {
  Name(String),
  Recent,
  NextActive,
  PreviousActive,
  NextActiveInMonitor,
  PreviousActiveInMonitor,
  Next,
  Previous,
  /// r+N / r-N: Next / Previous taken N steps (signed).
  Relative(isize),
  #[allow(dead_code)]
  Direction(Direction),
}
