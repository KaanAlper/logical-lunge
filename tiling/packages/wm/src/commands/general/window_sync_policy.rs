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

/// An app in its own fullscreen covers its whole monitor (to the pixel;
/// Chromium's background frame lacks the last row) and has dropped its frame
/// (caption and sizing border: Chromium, Electron and games do; a frameless
/// app's normal window keeps its sizing border).
pub(crate) fn is_app_fullscreen(frame: &Rect, monitor: &Rect, framed: bool) -> bool {
  !framed
    && frame.left <= monitor.left + 1
    && frame.top <= monitor.top + 1
    && frame.right >= monitor.right - 1
    && frame.bottom >= monitor.bottom - 1
}

/// Whether a window's live frame is its fullscreen target, give or take the
/// pixel the OS (or the app's own fullscreen code) can be off by. A
/// fullscreen window that already is there is never moved again by a
/// redraw: workspace switches show and hide it, and a resize there (with
/// `SWP_FRAMECHANGED`) made games and players rebuild their output, so the
/// picture shrank and grew at every switch.
pub(crate) fn at_fullscreen_target(frame: &Rect, target: &Rect) -> bool {
  (frame.left - target.left).abs() <= 1
    && (frame.top - target.top).abs() <= 1
    && (frame.right - target.right).abs() <= 1
    && (frame.bottom - target.bottom).abs() <= 1
}

/// ii's `misc:on_focus_under_fullscreen = 2`: another window focused on a
/// workspace that has a fullscreen window takes that window out of its
/// fullscreen, and the layout is normal again. One explicit rule instead
/// of a fullscreen window and the newly focused one taking turns on top.
pub(crate) fn leaves_fullscreen_for_focus(
  candidate: &WindowState,
  candidate_is_focused: bool,
  same_workspace: bool,
  focused: &WindowState,
) -> bool {
  matches!(candidate, WindowState::Fullscreen(_))
    && !candidate_is_focused
    && same_workspace
    && matches!(focused, WindowState::Tiling | WindowState::Floating(_))
}

/// What a tiling window that grew over its whole workspace by itself gets.
/// As in ii (Hyprland): an app's own fullscreen is real fullscreen, and the
/// spoof key (`toggle-fullscreen-spoof`) keeps it in its tile instead.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SelfFullscreen {
  /// The app's own fullscreen becomes the window manager's fullscreen
  Fullscreen,
  /// Spoofed: the app stays in its own fullscreen inside its tile
  KeepInTile,
  /// Not fullscreen (a framed window grew past its tile): back into it
  ReturnToTile,
}

pub(crate) fn self_fullscreen(
  spoofed: bool,
  frame: &Rect,
  monitor: &Rect,
  framed: bool,
) -> SelfFullscreen {
  if !is_app_fullscreen(frame, monitor, framed) {
    SelfFullscreen::ReturnToTile
  } else if spoofed {
    SelfFullscreen::KeepInTile
  } else {
    SelfFullscreen::Fullscreen
  }
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
    // The window manager's maximize is a window placed over the workspace's
    // area (see `NonTilingWindow::to_rect`), never Windows' own maximize,
    // which covers the bar: a natively maximized window is restored first.
    WindowState::Fullscreen(_) => maximized,
    WindowState::Minimized => false,
    _ => minimized || maximized,
  }
}

pub(super) fn needs_geometry_sync(
  target: &WindowState,
  minimized: bool,
  maximized: bool,
  pending_dpi: bool,
  at_target: bool,
) -> bool {
  match target {
    WindowState::Minimized => false,
    _ => {
      should_restore(target, minimized, maximized)
        || pending_dpi
        || !at_target
    }
  }
}

/// Hyprland's one-mode-at-a-time toggles: `Some` overrides the usual
/// toggle (back to the previous state). Super+F (`toggle-fullscreen`) on a
/// maximized window makes it fullscreen at once; every other toggle of a
/// fullscreen or maximized window (Super+D on a fullscreen one included)
/// leaves it, as the usual toggle does.
pub(crate) fn toggled_fullscreen_mode(current: &WindowState, target: &WindowState) -> Option<WindowState> {
  match (current, target) {
    (WindowState::Fullscreen(now), WindowState::Fullscreen(want)) if now.maximized && !want.maximized => {
      Some(target.clone())
    }
    _ => None,
  }
}

/// What a move of a window maximized by the window manager (Super+D)
/// means.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MaximizedMove {
  /// At the workspace's area: our own placement landing
  AtTarget,
  /// The app went into its own fullscreen (F on a video, F11, a game): real
  /// fullscreen over the monitor, whatever mode the window was in
  AppFullscreen,
  /// Somewhere else: the window leaves the maximize (restored by its app)
  Elsewhere,
}

pub(crate) fn maximized_move(frame: &Rect, target: &Rect, monitor: &Rect, framed: bool) -> MaximizedMove {
  if at_fullscreen_target(frame, target) {
    MaximizedMove::AtTarget
  } else if is_app_fullscreen(frame, monitor, framed) {
    MaximizedMove::AppFullscreen
  } else {
    MaximizedMove::Elsewhere
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

  #[test]
  fn a_fullscreen_window_at_its_monitor_is_left_alone() {
    let monitor = Rect::from_ltrb(0, 0, 1920, 1080);
    assert!(at_fullscreen_target(&monitor, &monitor));
    // a game a pixel past its monitor, or a row short of it
    assert!(at_fullscreen_target(&Rect::from_ltrb(-1, -1, 1921, 1081), &monitor));
    assert!(at_fullscreen_target(&Rect::from_ltrb(0, 0, 1920, 1079), &monitor));
    // still somewhere else: moved
    assert!(!at_fullscreen_target(&Rect::from_ltrb(0, 45, 1920, 1080), &monitor));
    assert!(!at_fullscreen_target(&Rect::from_ltrb(1920, 0, 3840, 1080), &monitor));
  }

  #[test]
  fn focusing_another_window_takes_the_fullscreen_one_down() {
    let full = fullscreen(false);
    let floating = WindowState::Floating(FloatingStateConfig::default());
    assert!(leaves_fullscreen_for_focus(&full, false, true, &WindowState::Tiling));
    assert!(leaves_fullscreen_for_focus(&full, false, true, &floating));
    // the fullscreen window itself focused, another workspace, or a
    // fullscreen window taking focus: nothing changes
    assert!(!leaves_fullscreen_for_focus(&full, true, true, &WindowState::Tiling));
    assert!(!leaves_fullscreen_for_focus(&full, false, false, &WindowState::Tiling));
    assert!(!leaves_fullscreen_for_focus(&full, false, true, &full));
    assert!(!leaves_fullscreen_for_focus(&WindowState::Tiling, false, true, &WindowState::Tiling));
  }

  #[test]
  fn an_apps_own_fullscreen_is_real_unless_spoofed() {
    let monitor = Rect::from_ltrb(0, 0, 1920, 1080);
    let shrunk = Rect::from_ltrb(0, 0, 1920, 1079); // Chromium, focus elsewhere
    let workspace_sized = Rect::from_ltrb(0, 45, 1920, 1080); // below the bar
    let elsewhere = Rect::from_ltrb(1920, 0, 3840, 1080); // the next monitor
    for (frame, framed, spoofed, want) in [
      (&monitor, false, false, SelfFullscreen::Fullscreen),
      (&shrunk, false, false, SelfFullscreen::Fullscreen),
      (&monitor, false, true, SelfFullscreen::KeepInTile),
      (&shrunk, false, true, SelfFullscreen::KeepInTile),
      // a framed window that restored a monitor-sized frame is not fullscreen
      (&monitor, true, false, SelfFullscreen::ReturnToTile),
      (&monitor, true, true, SelfFullscreen::ReturnToTile),
      (&workspace_sized, false, false, SelfFullscreen::ReturnToTile),
      (&elsewhere, false, false, SelfFullscreen::ReturnToTile),
    ] {
      assert_eq!(self_fullscreen(spoofed, frame, &monitor, framed), want, "{frame:?} framed={framed} spoofed={spoofed}");
    }
  }

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
    // our maximize: placed over the workspace's area, not Windows' maximize
    assert!(!needs_geometry_sync(&fullscreen(true), false, false, false, true));
  }

  #[test]
  fn maximize_restore_and_dpi_changes_still_require_geometry_sync() {
    // Windows' own maximize (it covers the bar) is restored into ours
    assert!(needs_geometry_sync(&fullscreen(true), false, true, false, true));
    assert!(should_restore(&fullscreen(true), false, true));
    assert!(needs_geometry_sync(&fullscreen(true), false, false, true, true));
    assert!(needs_geometry_sync(&fullscreen(true), false, false, false, false));
    assert!(needs_geometry_sync(&WindowState::Tiling, false, true, false, true));
    assert!(needs_geometry_sync(&WindowState::Tiling, true, false, false, true));
    assert!(needs_geometry_sync(&fullscreen(false), false, true, false, true));
  }

  #[test]
  fn unchanged_normal_windows_and_minimized_targets_need_no_geometry_work()
  {
    for target in [
      WindowState::Tiling,
      fullscreen(false),
      fullscreen(true),
      WindowState::Floating(FloatingStateConfig::default()),
      WindowState::Minimized,
    ] {
      assert!(!needs_geometry_sync(&target, false, false, false, true));
    }
    assert!(!should_restore(&WindowState::Minimized, true, true));
  }

  #[test]
  fn fullscreen_and_maximize_are_one_mode_at_a_time() {
    let full = fullscreen(false);
    let maxi = fullscreen(true);
    // Super+F on a maximized window: fullscreen at once
    assert_eq!(toggled_fullscreen_mode(&maxi, &full), Some(full.clone()));
    // Super+D on a fullscreen window, Super+F on a fullscreen one, Super+D
    // on a maximized one, and any toggle from a tile: the usual toggle
    assert_eq!(toggled_fullscreen_mode(&full, &maxi), None);
    assert_eq!(toggled_fullscreen_mode(&full, &full), None);
    assert_eq!(toggled_fullscreen_mode(&maxi, &maxi), None);
    assert_eq!(toggled_fullscreen_mode(&WindowState::Tiling, &full), None);
  }

  #[test]
  fn an_app_fullscreen_from_our_maximize_is_real_fullscreen() {
    let monitor = Rect::from_ltrb(0, 0, 1920, 1080);
    let area = Rect::from_ltrb(5, 45, 1915, 1075); // under the bar, inside the gaps
    assert_eq!(maximized_move(&area, &area, &monitor, true), MaximizedMove::AtTarget);
    // F on a video / F11: the app covers the monitor without its frame
    assert_eq!(maximized_move(&monitor, &area, &monitor, false), MaximizedMove::AppFullscreen);
    // a framed window at the monitor's size is no app fullscreen
    assert_eq!(maximized_move(&monitor, &area, &monitor, true), MaximizedMove::Elsewhere);
    // restored by its app (title bar double click): it leaves the maximize
    assert_eq!(maximized_move(&Rect::from_ltrb(300, 200, 1300, 900), &area, &monitor, true), MaximizedMove::Elsewhere);
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
