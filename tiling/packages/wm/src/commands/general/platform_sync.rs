use anyhow::Context;
#[cfg(target_os = "windows")]
use wm_common::WindowEffectConfig;
use wm_common::{
  CursorJumpTrigger, DisplayState, HideCorner, HideMethod, UniqueExt,
  WindowState, WmEvent,
};
#[cfg(target_os = "windows")]
use wm_platform::NativeWindowWindowsExt;
#[cfg(target_os = "windows")]
use wm_platform::{CornerStyle, OpacityValue};
#[cfg(target_os = "windows")]
use super::window_sync_policy;
use wm_platform::{Rect, WindowZOrder};

use crate::{
  commands::container::normalize_split_containers,
  models::{Container, WindowContainer},
  traits::{
    CommonGetters, PositionGetters, TilingDirectionGetters, WindowGetters,
  },
  user_config::UserConfig,
  wm_state::WmState,
};

/// Logical Lunge: the managed window that is in front and fullscreen (not
/// maximized) -- a game or a video.
fn fullscreen_in_front(state: &WmState) -> Option<WindowContainer> {
  let foreground = state.dispatcher.focused_window().ok()?;
  let window = state.window_from_native(&foreground)?;
  match window.state() {
    WindowState::Fullscreen(config) if !config.maximized => Some(window),
    _ => None,
  }
}

/// Logical Lunge: a fullscreen window that already is the foreground window
/// is above everything it should be: its z-order is left alone.
fn in_front_and_fullscreen(window: &WindowContainer, state: &WmState) -> bool {
  fullscreen_in_front(state).is_some_and(|front| front.id() == window.id())
}

/// Whether a pending focus change leaves a fullscreen window in front
/// alone: only when the system alone asked for it.
fn keeps_fullscreen_focus(system_only: bool, fullscreen_in_front: bool) -> bool {
  system_only && fullscreen_in_front
}

/// Whether any split below `root` has a single child or the same tiling
/// direction as its parent (see `normalize_split_containers`).
fn has_redundant_splits(root: &Container) -> bool {
  root.descendants().any(|container| {
    let Some(split) = container.as_split() else {
      return false;
    };

    split.child_count() == 1
      || split.parent().is_some_and(|parent| {
        parent.as_direction_container().is_ok_and(|parent| {
          parent.tiling_direction() == split.tiling_direction()
        })
      })
  })
}

pub fn platform_sync(
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  // Hyprland's pin: pinned windows are on the displayed workspace of their
  // monitor before anything is drawn (see `carry_pinned_windows`).
  crate::commands::window::carry_pinned_windows(state)?;

  // Logical Lunge: the tree never keeps a split with a single child or with
  // its parent's direction (dwindle has neither). Some path still left e.g.
  // `H[1]` inside a column, and later moves then acted on unexpected
  // neighbours. Enforced once per sync, whatever produced it.
  for workspace in state.workspaces() {
    let root: Container = workspace.clone().into();

    if has_redundant_splits(&root) {
      normalize_split_containers(&root)?;
      state
        .pending_sync
        .queue_containers_to_redraw(workspace.tiling_children());
    }
  }

  // Logical Lunge: a focus change only the system asked for (a window
  // appeared, closed or minimized) while a fullscreen window is in front
  // (a game) leaves the game alone: it stays fullscreen and focused, the
  // new window waits behind it. Only the user takes focus from it.
  let mut keep_game = false;
  if let Some(game) = fullscreen_in_front(state) {
    if keeps_fullscreen_focus(state.pending_sync.is_system_focus_only(), true) {
      crate::commands::container::set_focused_descendant(&game.into(), None);
      keep_game = true;
    }
  }

  // A window focused on a workspace with a fullscreen one: that one leaves
  // its fullscreen (ii's on_focus_under_fullscreen = 2) before the redraw.
  #[cfg(target_os = "windows")]
  if state.pending_sync.needs_focus_update() && !keep_game {
    crate::commands::window::leave_fullscreen_for_focus(state, config)?;
  }

  let focused_container =
    state.focused_container().context("No focused container.")?;

  // Logical Lunge: floating windows get a shadow. Set before the borders are
  // made below, so a new floating window's border has it from the start.
  #[cfg(target_os = "windows")]
  wm_borders::set_floating(
    state
      .windows()
      .iter()
      .filter(|window| matches!(window.state(), WindowState::Floating(_)))
      .map(|window| native_handle(window))
      .collect(),
  );
  // Only managed windows get a border.
  #[cfg(target_os = "windows")]
  wm_borders::set_managed(
    state
      .windows()
      .iter()
      .map(|window| native_handle(window))
      .collect(),
  );

  if state.pending_sync.needs_focus_update() && !keep_game {
    sync_focus(&focused_container, state)?;
  }

  if !state.pending_sync.containers_to_redraw().is_empty()
    || !state.pending_sync.workspaces_to_reorder().is_empty()
  {
    redraw_containers(&focused_container, state, config)?;
  }

  // After any sync (a click on a tile often only changes focus): no
  // floating window stays under a tile. Nothing to do without floating
  // windows on the shown workspaces.
  #[cfg(target_os = "windows")]
  if let Err(err) = keep_floating_above_tiling(state) {
    tracing::warn!("Failed to keep floating windows on top: {}", err);
  }

  if state.pending_sync.needs_cursor_jump()
    && !keep_game
    && config.value.general.cursor_jump.enabled
  {
    jump_cursor(focused_container.clone(), state, config)?;
  }

  if state.pending_sync.needs_focused_effect_update()
    || state.pending_sync.needs_all_effects_update()
  {
    // Keep reference to the previous window that had focus effects
    // applied.
    let prev_effects_window = state.prev_effects_window.clone();

    if let Ok(window) = focused_container.as_window_container() {
      apply_window_effects(&window, true, config);
      state.prev_effects_window = Some(window.clone());
    } else {
      state.prev_effects_window = None;
    }

    // Get windows that should have the unfocused border applied to them.
    // For the sake of performance, we only update the border of the
    // previously focused window. If the `reset_window_effects` flag is
    // passed, the unfocused border is applied to all unfocused windows.
    let unfocused_windows =
      if state.pending_sync.needs_all_effects_update() {
        state.windows()
      } else {
        prev_effects_window.into_iter().collect()
      }
      .into_iter()
      .filter(|window| window.id() != focused_container.id());

    for window in unfocused_windows {
      apply_window_effects(&window, false, config);
    }
  }

  state.pending_sync.clear();

  // Logical Lunge: remember where every window is (for a restart).
  let mut memory = std::mem::take(&mut state.layout_memory);
  memory.save(state);
  state.layout_memory = memory;

  Ok(())
}

fn sync_focus(
  focused_container: &Container,
  state: &mut WmState,
) -> anyhow::Result<()> {
  let native_window = focused_container.as_window_container().ok();

  // Sets focus to the appropriate target:
  // - If the container is a window, focuses that window.
  // - If the container is a workspace, "resets" focus by focusing the
  //   desktop window.
  //
  // In either case, a `PlatformEvent::WindowFocused` event is subsequently
  // triggered.
  let result = if let Some(window) = native_window {
    tracing::info!("Setting focus to window: {window}");
    window.native().focus()
  } else {
    tracing::info!("Setting focus to the desktop window.");
    state.dispatcher.reset_focus()
  };

  if let Err(err) = result {
    tracing::warn!("Failed to set focus: {}", err);
  }

  state.emit_event(WmEvent::FocusChanged {
    focused_container: focused_container.to_dto()?,
  });

  Ok(())
}

/// Finds windows that should be brought to the top of their workspace's
/// z-order.
///
/// Windows are brought to front if they match the focused window's state
/// (floating/tiling) and any of these conditions are met:
///  * Focus has changed to a different window.
///  * Focused window's state has changed (e.g. tiling -> floating).
///  * Focused window has moved to a different workspace.
fn windows_to_bring_to_front(
  focused_container: &Container,
  state: &WmState,
) -> anyhow::Result<Vec<WindowContainer>> {
  let focused_workspace =
    focused_container.workspace().context("No workspace.")?;

  // Add focused workspace if there's been a focus change.
  let workspaces_to_reorder = state
    .pending_sync
    .workspaces_to_reorder()
    .iter()
    .chain(
      state
        .pending_sync
        .needs_focus_update()
        .then_some(&focused_workspace),
    )
    .unique_by(|workspace| workspace.id());

  // Bring forward windows that match the focused state. Only do this for
  // tiling/floating windows.
  let windows_to_bring_to_front = workspaces_to_reorder
    .flat_map(|workspace| {
      let focused_descendant = workspace
        .descendant_focus_order()
        .next()
        .and_then(|container| container.as_window_container().ok());

      match focused_descendant {
        Some(focused_descendant) => workspace
          .descendants()
          .filter_map(|descendant| descendant.as_window_container().ok())
          .filter(|window| {
            let is_floating_or_tiling = matches!(
              window.state(),
              WindowState::Floating(_) | WindowState::Tiling
            );

            // A fullscreen window that is not always on top (an app's own
            // fullscreen) goes on top of the bar when focused and down when
            // another window is (see the z-order below)
            matches!(window.state(), WindowState::Fullscreen(fullscreen) if !fullscreen.shown_on_top)
              || (is_floating_or_tiling
                && (window.state().is_same_state(&focused_descendant.state())
                  // Logical Lunge: like Hyprland, floating windows stay
                  // above tiling ones. Focusing a tiling window used to
                  // bring only the tiling windows forward, hiding floating
                  // ones (e.g. a game launcher) behind them.
                  || floats_over_tiling(window, &focused_descendant)))
          })
          .collect(),
        None => vec![],
      }
    })
    .collect::<Vec<_>>();

  Ok(windows_to_bring_to_front)
}

/// Logical Lunge: a floating window (dialog, launcher, update screen) is
/// never left under a tiling window it overlaps, whatever raised the tiling
/// window (the app, a click, a restart). Checked after every redraw; a
/// floating window found under one is raised again, without taking focus.
#[cfg(target_os = "windows")]
pub fn keep_floating_above_tiling(state: &WmState) -> anyhow::Result<()> {
  let mut floating = Vec::new();
  let mut tiling = Vec::new();

  for workspace in state.workspaces().iter().filter(|w| w.is_displayed()) {
    for window in workspace
      .descendants()
      .filter_map(|descendant| descendant.as_window_container().ok())
    {
      match window.state() {
        WindowState::Floating(_) => floating.push(window),
        WindowState::Tiling => tiling.push(window),
        _ => {}
      }
    }
  }

  if floating.is_empty() || tiling.is_empty() {
    return Ok(());
  }

  // Top to bottom on Windows (`EnumWindows` order).
  let order = state
    .dispatcher
    .visible_windows()?
    .iter()
    .map(|window| window.hwnd().0)
    .collect::<Vec<_>>();

  let rank = |window: &WindowContainer| {
    let handle = window.native().hwnd().0;
    order.iter().position(|h| *h == handle)
  };

  for float in &floating {
    let (Some(float_rank), Ok(float_rect)) = (rank(float), float.to_rect())
    else {
      continue;
    };

    let covered = tiling.iter().any(|tile| {
      rank(tile).is_some_and(|tile_rank| tile_rank < float_rank)
        && tile
          .to_rect()
          .is_ok_and(|rect| rect.intersection_area(&float_rect) > 0)
    });

    if !covered {
      continue;
    }

    if float.native().is_controllable() {
      tracing::info!("Raising floating window above tiling: {float}");
      if let Err(err) = float.native().set_z_order(&WindowZOrder::Normal) {
        tracing::warn!("Failed to raise floating window: {}", err);
      }
    } else {
      // A window of an app run as administrator (Task Manager) can't be
      // raised by a WM that isn't: the tiles above it go right under it.
      for tile in tiling.iter().filter(|tile| {
        rank(tile).is_some_and(|tile_rank| tile_rank < float_rank)
          && tile
            .to_rect()
            .is_ok_and(|rect| rect.intersection_area(&float_rect) > 0)
      }) {
        if let Err(err) = tile
          .native()
          .set_z_order(&WindowZOrder::AfterWindow(float.native().id()))
        {
          tracing::warn!("Failed to lower tiling window: {}", err);
        }
      }
    }
  }

  Ok(())
}

/// Whether `window` is a floating window to keep above the tiling windows
/// of its workspace, whose focused window is `focused_descendant`.
fn floats_over_tiling(
  window: &WindowContainer,
  focused_descendant: &WindowContainer,
) -> bool {
  matches!(window.state(), WindowState::Floating(_))
    && matches!(focused_descendant.state(), WindowState::Tiling)
}

/// The focused window of the workspace that `window` is on.
fn workspace_focused_window(
  window: &WindowContainer,
) -> Option<WindowContainer> {
  window
    .workspace()?
    .descendant_focus_order()
    .next()
    .and_then(|container| container.as_window_container().ok())
}

#[allow(clippy::too_many_lines)]
fn redraw_containers(
  focused_container: &Container,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let windows_to_redraw = state.windows_to_redraw();
  let windows_to_bring_to_front =
    windows_to_bring_to_front(focused_container, state)?;

  let windows_to_update = {
    let mut windows = windows_to_redraw
      .iter()
      .chain(&windows_to_bring_to_front)
      .unique_by(|window| window.id())
      .collect::<Vec<_>>();

    let descendant_focus_order = state
      .root_container
      .descendant_focus_order()
      .collect::<Vec<_>>();

    // Sort the windows to update by their focus order. The most recently
    // focused window will be updated first.
    // TODO: To reduce flicker, redraw windows that will be shown first,
    // then redraw the ones to be hidden last.
    // Floating windows kept above tiling ones go first: windows are
    // updated in reverse, so they're raised after the focused tiling
    // window and end up on top of it.
    windows.sort_by_key(|window| {
      let over_tiling = windows_to_bring_to_front.contains(window)
        && workspace_focused_window(window)
          .is_some_and(|focused| floats_over_tiling(window, &focused));

      (
        !over_tiling,
        descendant_focus_order
          .iter()
          .position(|order| order.id() == window.id()),
      )
    });

    windows
  };

  // Get monitors by their optimal hide corner.
  let monitors_by_hide_corner = state.monitors_by_hide_corner();

  for window in windows_to_update.iter().rev() {
    let should_bring_to_front = windows_to_bring_to_front.contains(window);

    let workspace =
      window.workspace().context("Window has no workspace.")?;

    let monitor = window.monitor().context("No monitor.")?;
    let hide_corner = monitors_by_hide_corner
      .iter()
      .find(|(m, _)| m.id() == monitor.id())
      .map(|(_, hide_corner)| hide_corner)
      .context("Monitor not found in hide corner map.")?;

    // Whether the window should be shown above all other windows.
    // A workspace whose focused window is fullscreen (a game, a video) keeps
    // that window in front: no window on it is made HWND_TOPMOST, and the
    // fullscreen window itself never is either (it stays at the top of the
    // normal band, above the bar, which is not topmost). A topmost game
    // lost independent flip and stayed topmost if the WM died.
    let fullscreen_in_front = workspace_focused_window(window)
      .is_some_and(|focused| matches!(focused.state(), WindowState::Fullscreen(_)));
    // the open special workspace is drawn over the monitor's workspace
    // (and over its dim, see `sync_backdrop`)
    let z_order = if workspace.is_special() {
      WindowZOrder::TopMost
    } else {
      match window.state() {
        WindowState::Floating(config)
          if config.shown_on_top && !fullscreen_in_front =>
        {
          WindowZOrder::TopMost
        }
        // An app's own fullscreen: in front of the bar and every other normal
        // window while it is its workspace's focused window (the top of the
        // non-topmost band -- the bar is not topmost), never HWND_TOPMOST: a
        // game must keep its own z-order (independent flip) and must not be
        // left topmost if the WM stops. The window focused next comes in
        // front of it (as with Alt+Tab), and it comes back when focused.
        WindowState::Fullscreen(_) => WindowZOrder::Normal,
        // a floating window under a focused fullscreen one: below it
        WindowState::Floating(config) if config.shown_on_top => WindowZOrder::Normal,
        // Raised to the top, after the focused tiling window (see above).
        WindowState::Floating(_)
          if should_bring_to_front
            && workspace_focused_window(window).is_some_and(|focused| {
              floats_over_tiling(window, &focused)
            }) =>
        {
          WindowZOrder::Normal
        }
        _ if should_bring_to_front => {
          let focused_descendant = workspace
            .descendant_focus_order()
            .next()
            .and_then(|container| container.as_window_container().ok());

          if let Some(focused_descendant) = focused_descendant {
            if window.id() == focused_descendant.id() {
              WindowZOrder::Normal
            } else {
              WindowZOrder::AfterWindow(focused_descendant.native().id())
            }
          } else {
            WindowZOrder::Normal
          }
        }
        _ => WindowZOrder::Normal,
    }
    };

    // Set the z-order of the window.
    //
    // NOTE: macOS doesn't have a robust public API for setting the z-order
    // of a window. See `NativeWindow::raise` for more details.
    #[cfg(target_os = "windows")]
    if should_bring_to_front
      && !windows_to_redraw.contains(window)
      && window.native().is_controllable()
      && !in_front_and_fullscreen(window, state)
    {
      tracing::info!("Updating window z-order: {window}");

      if let Err(err) = window.native().set_z_order(&z_order) {
        tracing::warn!("Failed to set window z-order: {}", err);
      }
    }

    // Skip updating the window's position if it only required a z-order
    // change.
    if !windows_to_redraw.contains(window) {
      continue;
    }

    // Transition display state depending on whether window will be
    // shown or hidden.
    window.set_display_state(
      match (window.display_state(), workspace.is_displayed()) {
        (DisplayState::Hidden | DisplayState::Hiding, true) => {
          DisplayState::Showing
        }
        (DisplayState::Shown | DisplayState::Showing, false) => {
          DisplayState::Hiding
        }
        _ => window.display_state(),
      },
    );

    let is_visible = matches!(
      window.display_state(),
      DisplayState::Showing | DisplayState::Shown
    );

    #[cfg(target_os = "windows")]
    let (sync_tiled_fullscreen, notify_background_frame) = {
      let tiled = window.state() == WindowState::Tiling;
      let handle = window.native().hwnd().0;
      let marked = state.fake_fullscreen.contains(&handle);
      // Consume once, even if hidden, paused, dragged, or superseded by a
      // newer native position. A later layout must not reuse this signal.
      let observed = state.background_fullscreen_frames.remove(&handle);
      let live = observed.as_ref().and_then(|_| window.native().frame_with_shadows().ok());
      let background_escape = observed.is_some() && observed == live;
      // Cached geometry can lag an asynchronous move. Check the live frame
      // before choosing whether a corrective move may wait on the app.
      let escaped = if tiled && marked && is_visible {
        let bounds = workspace.max_workspace_rect()?;
        background_escape || window.native().frame().map_or(true, |frame| {
          frame.apply_delta(&window.border_delta().inverse(), None)
            .inset(1).contains_rect(&bounds)
        })
      } else {
        false
      };
      // Never wait on an app that stopped answering (a game compiling its
      // shaders): the WM's single thread would wait with it.
      let synchronous = window_sync_policy::synchronous_tile_correction(tiled, marked, escaped, is_visible)
        && !window.native().is_hung();
      let foreground = state.dispatcher.focused_window()
        .map_or(true, |focused| focused.id() == window.native().id());
      let notify = !state.is_paused && window.active_drag().is_none()
        && window_sync_policy::should_notify_background_fullscreen(
          observed.as_ref(), live.as_ref(), synchronous, foreground,
        );
      (synchronous, notify)
    };
    #[cfg(not(target_os = "windows"))]
    let sync_tiled_fullscreen = false;
    let reposition_result = reposition_window(window, *hide_corner, &z_order, is_visible, sync_tiled_fullscreen, config);
    #[cfg(target_os = "windows")]
    if reposition_result.is_ok() && notify_background_frame {
      // The NOSENDCHANGING correction above must finish first. Only then
      // let the background app recalculate its client area without sizing.
      let tile = window.to_rect()?.apply_delta(&window.total_border_delta()?, None);
      match window.native().notify_background_frame_changed(&tile) {
        Ok(true) => tracing::debug!("Notified background fullscreen client size: {window}"),
        Ok(false) => {},
        Err(err) => tracing::warn!("Failed to notify background fullscreen frame: {}", err),
      }
    }
    if let Err(err) = reposition_result {
      tracing::warn!("Failed to set window position: {}", err);
    }

    #[cfg(target_os = "windows")]
    if config.value.general.hide_method == HideMethod::Cloak {
      sync_companions(window, state);
    }

    // `prev_state` is toggle history, not a pending native transition.
    // Cache successful marks so workspace redraws don't repeat COM calls.
    #[cfg(target_os = "windows")]
    {
      let handle = window.native().hwnd().0;
      if let Some(fullscreen) = window_sync_policy::fullscreen_mark(
        window.prev_state().as_ref(),
        &window.state(),
        state.fullscreen_marks.get(&handle).copied(),
      ) {
        match window.native().mark_fullscreen(fullscreen) {
          Ok(()) => {
            state.fullscreen_marks.insert(handle, fullscreen);
          }
          Err(err) => {
            tracing::warn!("Failed to mark window as fullscreen: {}", err);
          }
        }
      }
    }

    // Skip setting taskbar visibility if the window is hidden (has no
    // effect). Since cloaked windows are normally always visible in the
    // taskbar, we only need to set visibility if `show_all_in_taskbar` is
    // `false`.
    #[cfg(target_os = "windows")]
    if config.value.general.hide_method == HideMethod::Cloak
      && !config.value.general.show_all_in_taskbar
      && matches!(
        window.display_state(),
        DisplayState::Showing | DisplayState::Hiding
      )
    {
      if let Err(err) = window.native().set_taskbar_visibility(is_visible)
      {
        tracing::warn!("Failed to set taskbar visibility: {}", err);
      }
    }
  }

  Ok(())
}

/// Logical Lunge: cloaking hides one window. Windows its app's own
/// processes put over it (an embedded browser's input window) stayed up and
/// kept catching the clicks on its area, on every workspace. They go when it
/// starts hiding and come back when it starts showing; windows the WM
/// manages are left to it.
#[cfg(target_os = "windows")]
fn sync_companions(window: &WindowContainer, state: &mut WmState) {
  let key = window.native().hwnd().0;
  match window.display_state() {
    DisplayState::Hiding if !state.hidden_companions.contains_key(&key) => {
      let managed = state.windows();
      let hidden: Vec<_> = window
        .native()
        .companions()
        .into_iter()
        .filter(|c| !managed.iter().any(|w| w.native().id() == c.id()))
        .filter(|c| c.hide().is_ok())
        .collect();
      if !hidden.is_empty() {
        state.hidden_companions.insert(key, hidden);
      }
    }
    DisplayState::Showing => {
      for companion in state.hidden_companions.remove(&key).unwrap_or_default() {
        let _ = companion.show();
      }
    }
    _ => {}
  }
}

fn reposition_window(
  window: &WindowContainer,
  hide_corner: HideCorner,
  // LINT: `z_order` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  z_order: &WindowZOrder,
  is_visible: bool,
  // Only an actual escape to fullscreen needs a synchronous correction.
  // The fake-fullscreen marker also survives normal layout/exit moves.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  sync_tiled_fullscreen: bool,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let rect = window
    .to_rect()?
    .apply_delta(&window.total_border_delta()?, None);

  // For `HideMethod::PlaceInCorner`, we need to reposition hidden windows
  // to the corner of the monitor.
  if config.value.general.hide_method == HideMethod::PlaceInCorner
    && !is_visible
  {
    const VISIBLE_SLIVER: i32 = 1;

    let monitor_rect = window
      .monitor()
      .context("No monitor.")?
      .native_properties()
      .working_area;

    let frame = window.native_properties().frame;

    let position_y = monitor_rect.bottom - VISIBLE_SLIVER;
    let position_x = match hide_corner {
      HideCorner::BottomLeft => {
        monitor_rect.left + VISIBLE_SLIVER - frame.width()
      }
      HideCorner::BottomRight => monitor_rect.right - VISIBLE_SLIVER,
    };

    // Even though the window size is unchanged, `NativeWindow::set_frame`
    // is used instead of `NativeWindow::reposition` because the latter
    // resulted in occasional incorrect positionings on macOS.
    window.native().set_frame(&Rect::from_xy(
      position_x,
      position_y,
      frame.width(),
      frame.height(),
    ))?;

    return Ok(());
  }

  if window.active_drag().is_some() {
    window.native().resize(rect.width(), rect.height())?;
  } else {
    #[cfg(target_os = "macos")]
    window.native().set_frame(&rect)?;

    #[cfg(target_os = "windows")]
    {
      use wm_platform::{
        SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOCOPYBITS, SWP_NOSENDCHANGING, WS_EX_TOPMOST,
      };

      // Logical Lunge: a tile's visible frame, so that a window that stays
      // bigger (a minimum size) is cut to it instead of covering its
      // neighbours (see `NativeWindowWindowsExt::set_slot`).
      {
        use wm_platform::NativeWindowWindowsExt;
        let slot = if window.state() == WindowState::Tiling && is_visible {
          Some(window.to_rect()?)
        } else {
          None
        };
        if let Err(err) = window.native().set_slot(slot.as_ref()) {
          tracing::warn!("Failed to set window slot: {}", err);
        }
      }

      // Restore window if it's minimized/maximized and shouldn't be. This
      // is needed to be able to move and resize it.
      let target_state = window.state();
      let is_minimized = window.native().is_minimized()?;
      let is_maximized = window.native().is_maximized()?;
      let should_restore = window_sync_policy::should_restore(
        &target_state, is_minimized, is_maximized,
      );
      // A fullscreen window already at its monitor stays exactly as it is
      // while its workspace is shown and hidden (cloaked, never resized).
      let live_frame = window.native().frame_with_shadows().ok();
      let at_target = match &target_state {
        WindowState::Fullscreen(fullscreen) if !fullscreen.maximized => live_frame
          .as_ref()
          .is_some_and(|frame| window_sync_policy::at_fullscreen_target(frame, &rect)),
        _ => live_frame.as_ref() == Some(&rect),
      };
      let needs_geometry_sync = window_sync_policy::needs_geometry_sync(
        &target_state,
        is_minimized,
        is_maximized,
        window.has_pending_dpi_adjustment(),
        at_target,
      );

      if should_restore {
        // Restoring to position has the same effect as `ShowWindow` with
        // `SW_RESTORE`, but doesn't cause a flicker.
        window.native().restore(Some(&rect))?;
      }

      let mut swp_flags = SWP_NOACTIVATE
        | SWP_NOCOPYBITS
        | SWP_NOSENDCHANGING;
      if !sync_tiled_fullscreen {
        swp_flags |= SWP_ASYNCWINDOWPOS;
      }

      match &window.state() {
        WindowState::Minimized => {
          if !is_minimized {
            window.native().minimize()?;
          }
        }
        // The window manager's maximize is placed like any other window,
        // over its workspace's area (see `NonTilingWindow::to_rect`).
        _ => {
          // Skip `SetWindowPos` when the window is already at its target
          // rect. Switching workspaces redraws every window being shown or
          // hidden, and `SWP_FRAMECHANGED` forces each app to recalculate
          // and repaint its frame and contents even if nothing moved,
          // which loads DWM during workspace animations.
          if !needs_geometry_sync {
            // Only the z-order may still need updating (e.g. a window
            // that is no longer shown on top).
            let is_topmost =
              window.native().has_window_style_ex(WS_EX_TOPMOST);

            if is_visible
              && (*z_order != WindowZOrder::Normal || is_topmost)
            {
              window.native().set_z_order(z_order)?;
            }
          } else {
            swp_flags |= SWP_FRAMECHANGED;

            window.native().set_window_pos(z_order, &rect, swp_flags)?;

            // When there's a mismatch between the DPI of the monitor and
            // the window, the window might be sized incorrectly after the
            // first move. If we set the position twice, inconsistencies
            // after the first move are resolved.
            if window.has_pending_dpi_adjustment() {
              window.native().set_window_pos(z_order, &rect, swp_flags)?;
            }
          }
        }
      }

      // Logical Lunge: hide the border in the same step as the window.
      if !is_visible {
        wm_borders::hide(native_handle(window));
      }

      // Set visibility based on the hide method.
      if config.value.general.hide_method == HideMethod::Cloak {
        window.native().set_cloaked(!is_visible)?;
      } else if is_visible {
        window.native().show()?;
      } else {
        window.native().hide()?;
      }

      // Logical Lunge: move (or show) the border in the same step as the
      // window, at the window's visible frame (its rect without the
      // invisible resize borders).
      // Fullscreen windows (e.g. a browser playing a video fullscreen) get
      // no border, like in Hyprland; a border shown while the window was
      // tiled is hidden, otherwise it stayed at the old position.
      let has_frame = match window.state() {
        WindowState::Tiling | WindowState::Floating(_) => true,
        WindowState::Fullscreen(_) | WindowState::Minimized => false,
      };

      if is_visible && !has_frame {
        wm_borders::hide(native_handle(window));
      }

      if is_visible && has_frame {
        let frame = window
          .to_rect()?
          .apply_delta(&window.border_delta(), None);

        wm_borders::place(
          native_handle(window),
          frame.left,
          frame.top,
          frame.right,
          frame.bottom,
        );
      }
    }
  }

  Ok(())
}

/// Gets the window handle of a window as passed to the border engine.
#[cfg(target_os = "windows")]
#[allow(clippy::cast_possible_wrap, clippy::unnecessary_cast)]
fn native_handle(window: &WindowContainer) -> isize {
  window.native().id().0 as isize
}

fn jump_cursor(
  focused_container: Container,
  state: &WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let cursor_jump = &config.value.general.cursor_jump;

  let jump_target = match cursor_jump.trigger {
    CursorJumpTrigger::WindowFocus => Some(focused_container),
    CursorJumpTrigger::MonitorFocus => {
      let target_monitor =
        focused_container.monitor().context("No monitor.")?;

      let cursor_monitor = state
        .dispatcher
        .cursor_position()
        .ok()
        .and_then(|pos| state.monitor_at_point(&pos));

      // Jump to the target monitor if the cursor is not already on it.
      cursor_monitor
        .filter(|monitor| monitor.id() != target_monitor.id())
        .map(|_| target_monitor.into())
    }
  };

  if let Some(jump_target) = jump_target {
    let center = jump_target.to_rect()?.center_point();

    if let Err(err) = state.dispatcher.set_cursor_position(&center) {
      tracing::warn!("Failed to set cursor position: {}", err);
    }
  }

  Ok(())
}

fn apply_window_effects(
  // LINT: `window` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  window: &WindowContainer,
  is_focused: bool,
  config: &UserConfig,
) {
  let window_effects = &config.value.window_effects;

  // LINT: `effect_config` is only used on Windows.
  #[cfg_attr(not(target_os = "windows"), allow(unused_variables))]
  let effect_config = if is_focused {
    &window_effects.focused_window
  } else {
    &window_effects.other_windows
  };

  // Skip if both focused + non-focused window effects are disabled.
  #[cfg(target_os = "windows")]
  if window_effects.focused_window.border.enabled
    || window_effects.other_windows.border.enabled
  {
    apply_border_effect(window, effect_config);
  } else {
    // Logical Lunge draws the borders itself: Windows 11's own 1 px frame
    // line (on windows whose corners Windows draws) would sit next to ours.
    // DWMWA_COLOR_NONE removes it; on Windows 10 the call fails harmlessly.
    _ = window.native().set_border_color(None);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.hide_title_bar.enabled
    || window_effects.other_windows.hide_title_bar.enabled
  {
    apply_hide_title_bar_effect(window, effect_config);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.corner_style.enabled
    || window_effects.other_windows.corner_style.enabled
  {
    apply_corner_effect(window, effect_config);
  }

  #[cfg(target_os = "windows")]
  if window_effects.focused_window.transparency.enabled
    || window_effects.other_windows.transparency.enabled
  {
    apply_transparency_effect(window, effect_config);
  }
}

#[cfg(target_os = "windows")]
fn apply_border_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let border_color = if effect_config.border.enabled {
    Some(&effect_config.border.color)
  } else {
    None
  };

  _ = window.native().set_border_color(border_color);

  let native = window.native().clone();
  let border_color = border_color.cloned();

  // Re-apply border color after a short delay to better handle
  // windows that change it themselves.
  tokio::task::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    _ = native.set_border_color(border_color.as_ref());
  });
}

#[cfg(target_os = "windows")]
fn apply_hide_title_bar_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  _ = window
    .native()
    .set_title_bar_visibility(!effect_config.hide_title_bar.enabled);
}

#[cfg(target_os = "windows")]
fn apply_corner_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let corner_style = if effect_config.corner_style.enabled {
    &effect_config.corner_style.style
  } else {
    &CornerStyle::Default
  };

  _ = window.native().set_corner_style(corner_style);
}

#[cfg(target_os = "windows")]
fn apply_transparency_effect(
  window: &WindowContainer,
  effect_config: &WindowEffectConfig,
) {
  let transparency = if effect_config.transparency.enabled {
    &effect_config.transparency.opacity
  } else {
    // Reset the transparency to default.
    &OpacityValue::from_alpha(u8::MAX)
  };

  _ = window.native().set_transparency(transparency);
}

#[cfg(test)]
mod game_focus_tests {
  use super::keeps_fullscreen_focus;

  #[test]
  fn system_focus_leaves_a_fullscreen_window_alone() {
    assert!(keeps_fullscreen_focus(true, true));
  }

  #[test]
  fn user_focus_takes_it_from_a_fullscreen_window() {
    assert!(!keeps_fullscreen_focus(false, true));
  }

  #[test]
  fn nothing_is_kept_without_a_fullscreen_window() {
    assert!(!keeps_fullscreen_focus(true, false));
    assert!(!keeps_fullscreen_focus(false, false));
  }
}
