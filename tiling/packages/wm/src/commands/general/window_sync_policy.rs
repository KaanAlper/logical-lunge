//! Decisions kept separate from Win32 calls so redraws can be tested offline.
use wm_common::WindowState;
use wm_platform::Rect;

/// Chromium's background-fullscreen workaround uses the physical monitor
/// bounds, with exactly its last row removed (not the working area).
pub(crate) fn is_background_fullscreen_frame(frame: &Rect, monitor: &Rect) -> bool {
  monitor.width() > 0
    && monitor.height() > 1
    && frame.left == monitor.left
    && frame.top == monitor.top
    && frame.right == monitor.right
    && frame.bottom == monitor.bottom - 1
}

pub(super) fn should_notify_background_fullscreen(
  observed: Option<&Rect>,
  live: Option<&Rect>,
  synchronous: bool,
  foreground: bool,
) -> bool {
  synchronous && !foreground && observed.is_some() && observed == live
}

pub(super) fn synchronous_tile_correction(
  tiled: bool,
  marked: bool,
  escaped: bool,
  visible: bool,
) -> bool {
  tiled && marked && escaped && visible
}

pub(super) fn should_restore(
  target: &WindowState,
  minimized: bool,
  maximized: bool,
) -> bool {
  match target {
    WindowState::Fullscreen(fullscreen) => {
      !fullscreen.maximized && maximized
    }
    WindowState::Minimized => false,
    _ => minimized || maximized,
  }
}

pub(super) fn needs_geometry_sync(
  target: &WindowState,
  minimized: bool,
  maximized: bool,
  has_maximize_box: bool,
  pending_dpi: bool,
  at_target: bool,
) -> bool {
  match target {
    WindowState::Minimized => false,
    WindowState::Fullscreen(fullscreen)
      if fullscreen.maximized && has_maximize_box =>
    {
      !maximized || pending_dpi || !at_target
    }
    _ => {
      should_restore(target, minimized, maximized)
        || pending_dpi
        || !at_target
    }
  }
}

pub(super) fn fullscreen_mark(
  previous: Option<&WindowState>,
  current: &WindowState,
  last_mark: Option<bool>,
) -> Option<bool> {
  let desired = matches!(current, WindowState::Fullscreen(fullscreen) if !fullscreen.maximized);
  match last_mark {
    Some(last) if last == desired => None,
    Some(_) => Some(desired),
    None if desired => Some(true),
    None if matches!(previous, Some(WindowState::Fullscreen(_))) => {
      Some(false)
    }
    None => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use wm_common::{FloatingStateConfig, FullscreenStateConfig};

  fn fullscreen(maximized: bool) -> WindowState {
    WindowState::Fullscreen(FullscreenStateConfig {
      maximized,
      ..Default::default()
    })
  }

  #[test]
  fn background_fullscreen_requires_exact_monitor_minus_one_pixel() {
    use wm_platform::Rect;
    for monitor in [Rect::from_xy(0, 0, 1920, 1080), Rect::from_xy(-2560, -200, 2560, 1440)] {
      let background = Rect::from_xy(monitor.x(), monitor.y(), monitor.width(), monitor.height() - 1);
      assert!(is_background_fullscreen_frame(&background, &monitor));
      assert!(!is_background_fullscreen_frame(&monitor, &monitor));
      assert!(!is_background_fullscreen_frame(&Rect::from_xy(monitor.x(), monitor.y() + 1, monitor.width(), monitor.height() - 1), &monitor));
      assert!(!is_background_fullscreen_frame(&Rect::from_xy(monitor.x(), monitor.y(), monitor.width() - 1, monitor.height() - 1), &monitor));
      assert!(!is_background_fullscreen_frame(&Rect::from_xy(964, 45, 951, 1030), &monitor));
    }
  }

  #[test]
  fn background_frame_notification_requires_current_observation_and_sync_move() {
    use wm_platform::Rect;
    let background = Rect::from_xy(0, 0, 1920, 1079);
    let active = Rect::from_xy(0, 0, 1920, 1080);
    let tile = Rect::from_xy(964, 45, 951, 1030);
    assert!(should_notify_background_fullscreen(Some(&background), Some(&background), true, false));
    assert!(!should_notify_background_fullscreen(None, Some(&background), true, false));
    assert!(!should_notify_background_fullscreen(Some(&background), None, true, false));
    assert!(!should_notify_background_fullscreen(Some(&background), Some(&active), true, false));
    assert!(!should_notify_background_fullscreen(Some(&background), Some(&tile), true, false));
    assert!(!should_notify_background_fullscreen(Some(&background), Some(&background), false, false));
    assert!(!should_notify_background_fullscreen(Some(&background), Some(&background), true, true));
  }

  #[test]
  fn acknowledged_fullscreen_does_not_block_later_layout_moves() {
    assert!(synchronous_tile_correction(true, true, true, true));
    assert!(!synchronous_tile_correction(true, true, false, true));
    assert!(!synchronous_tile_correction(true, true, true, false));
    assert!(!synchronous_tile_correction(false, true, true, true));
    assert!(!synchronous_tile_correction(true, false, true, true));
  }

  #[test]
  fn unchanged_maximized_window_needs_no_geometry_work_on_workspace_switch()
  {
    assert!(!needs_geometry_sync(
      &fullscreen(true),
      false,
      true,
      true,
      false,
      true
    ));
  }

  #[test]
  fn maximize_restore_and_dpi_changes_still_require_geometry_sync() {
    assert!(needs_geometry_sync(
      &fullscreen(true),
      false,
      false,
      true,
      false,
      true
    ));
    assert!(needs_geometry_sync(
      &fullscreen(true),
      false,
      true,
      true,
      true,
      true
    ));
    assert!(needs_geometry_sync(
      &fullscreen(true),
      false,
      true,
      true,
      false,
      false
    ));
    assert!(needs_geometry_sync(
      &WindowState::Tiling,
      false,
      true,
      true,
      false,
      true
    ));
    assert!(needs_geometry_sync(
      &WindowState::Tiling,
      true,
      false,
      true,
      false,
      true
    ));
    assert!(needs_geometry_sync(
      &fullscreen(false),
      false,
      true,
      true,
      false,
      true
    ));
  }

  #[test]
  fn unchanged_normal_windows_and_minimized_targets_need_no_geometry_work()
  {
    for target in [
      WindowState::Tiling,
      fullscreen(false),
      WindowState::Floating(FloatingStateConfig::default()),
      WindowState::Minimized,
    ] {
      assert!(!needs_geometry_sync(
        &target, false, false, true, false, true
      ));
    }
    assert!(!should_restore(&WindowState::Minimized, true, true));
  }

  #[test]
  fn fullscreen_redraw_marks_once_until_state_changes() {
    let current = fullscreen(false);
    assert_eq!(
      fullscreen_mark(Some(&WindowState::Tiling), &current, None),
      Some(true)
    );
    assert_eq!(
      fullscreen_mark(Some(&WindowState::Tiling), &current, Some(true)),
      None
    );
    assert_eq!(
      fullscreen_mark(Some(&current), &WindowState::Tiling, Some(true)),
      Some(false)
    );
    assert_eq!(
      fullscreen_mark(Some(&current), &WindowState::Tiling, Some(false)),
      None
    );
  }

  #[test]
  fn initially_fullscreen_window_is_marked_without_previous_state() {
    assert_eq!(fullscreen_mark(None, &fullscreen(false), None), Some(true));
    assert_eq!(fullscreen_mark(None, &WindowState::Tiling, None), None);
  }

  #[test]
  fn maximized_fullscreen_clears_real_fullscreen_mark() {
    let real_fullscreen = fullscreen(false);
    assert_eq!(
      fullscreen_mark(
        Some(&real_fullscreen),
        &fullscreen(true),
        Some(true)
      ),
      Some(false)
    );
    assert_eq!(
      fullscreen_mark(
        Some(&real_fullscreen),
        &fullscreen(true),
        Some(false)
      ),
      None
    );
  }

  #[test]
  fn failed_mark_is_retried_with_unchanged_success_cache() {
    assert_eq!(
      fullscreen_mark(Some(&WindowState::Tiling), &fullscreen(false), None),
      Some(true)
    );
    assert_eq!(
      fullscreen_mark(
        Some(&fullscreen(false)),
        &WindowState::Tiling,
        Some(true)
      ),
      Some(false)
    );
  }
}
