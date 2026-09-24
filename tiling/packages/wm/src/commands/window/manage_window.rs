use anyhow::Context;
use tracing::info;
use wm_common::{try_warn, WindowRuleEvent, WindowState, WmEvent};
use wm_platform::{NativeWindow, Point, RectDelta};

use crate::{
  commands::{
    container::{
      attach_container, detach_container, normalize_split_containers,
      set_focused_descendant,
    },
    window::{dwindle_split, run_window_rules},
  },
  models::{
    Container, Monitor, NativeWindowProperties, NonTilingWindow,
    TilingContainer, TilingWindow, WindowContainer,
  },
  traits::{CommonGetters, PositionGetters, WindowGetters},
  user_config::UserConfig,
  wm_state::WmState,
};

pub fn manage_window(
  native_window: NativeWindow,
  target_parent: Option<Container>,
  state: &mut WmState,
  config: &mut UserConfig,
) -> anyhow::Result<()> {
  let Some(native_properties) =
    check_is_manageable(&native_window).unwrap_or(None)
  else {
    return Ok(());
  };

  // Without a target parent the window was just opened (place it under the
  // cursor); with one, it's being managed on startup (build a spiral).
  let use_cursor = target_parent.is_none();

  // Create the window instance. This may fail if the window handle has
  // already been destroyed.
  let window = try_warn!(create_window(
    native_window,
    native_properties,
    target_parent,
    state,
    config
  ));

  if let WindowContainer::TilingWindow(tiling_window) = &window {
    dwindle_place(tiling_window, use_cursor, state, config)?;

    if let Some(workspace) = tiling_window.workspace() {
      normalize_split_containers(&workspace.into())?;
    }
  }

  // Set the newly added window as focus descendant. This means the window
  // rules will be run as if the window is focused.
  set_focused_descendant(&window.clone().into(), None);

  // Window might be detached if `ignore` command has been invoked.
  let updated_window = run_window_rules(
    window.clone(),
    &WindowRuleEvent::Manage,
    state,
    config,
  )?;

  if let Some(window) = updated_window {
    info!("New window managed: {window}");

    state.emit_event(WmEvent::WindowManaged {
      managed_window: window.to_dto()?,
    });

    // OS focus should be set to the newly added window in case it's not
    // already focused.
    state.pending_sync.queue_focus_change();

    // Normally, a `PlatformEvent::WindowFocused` event is what triggers
    // focus effects and workspace reordering to be applied. However, when
    // a window is first launched, this event can come before the
    // window is managed, and so we need to force an update here.
    state.pending_sync.queue_focused_effect_update();
    state.pending_sync.queue_workspace_to_reorder(
      window.workspace().context("No workspace.")?,
    );

    // Like Hyprland, never show a new window before it's in its final
    // place: cloak it and move it there synchronously. The redraw below
    // uncloaks it (or keeps it cloaked on a hidden workspace).
    #[cfg(target_os = "windows")]
    if window.state() == WindowState::Tiling
      && config.value.general.hide_method == wm_common::HideMethod::Cloak
    {
      use wm_platform::{
        NativeWindowWindowsExt, WindowZOrder, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOSENDCHANGING,
      };

      let rect = window
        .to_rect()?
        .apply_delta(&window.total_border_delta()?, None);

      if window.native().set_cloaked(true).is_ok() {
        let _ = window.native().set_window_pos(
          &WindowZOrder::Normal,
          &rect,
          SWP_NOACTIVATE
            | SWP_NOCOPYBITS
            | SWP_NOSENDCHANGING
            | SWP_FRAMECHANGED,
        );
      }
    }

    // Sibling containers need to be redrawn if the window is tiling.
    state.pending_sync.queue_container_to_redraw(
      if window.state() == WindowState::Tiling {
        window.parent().context("No parent.")?
      } else {
        window.into()
      },
    );
  }

  Ok(())
}

/// Checks if a window is manageable and retrieves its native properties.
///
/// Returns `Ok(Some(properties))` if the window is manageable and its
/// properties were retrieved successfully.
fn check_is_manageable(
  native_window: &NativeWindow,
) -> anyhow::Result<Option<NativeWindowProperties>> {
  if !native_window.is_visible()? {
    return Ok(None);
  }

  #[cfg(target_os = "macos")]
  {
    use wm_platform::NativeWindowExtMacOs;

    let is_standard_window = native_window.role()? == "AXWindow"
      && native_window.subrole()? == "AXStandardWindow";

    if !is_standard_window {
      return Ok(None);
    }
  }

  // Ensure window has a valid process name, title, etc.
  let native_properties = NativeWindowProperties::try_from(native_window)?;

  #[cfg(target_os = "windows")]
  {
    use wm_platform::{
      NativeWindowWindowsExt, WS_CAPTION, WS_CHILD, WS_EX_NOACTIVATE,
      WS_EX_TOOLWINDOW,
    };

    // TODO: Temporary fix for managing Flow Launcher until a force manage
    // command is added.
    let is_flow_launcher = native_properties.process_name
      == "Flow.Launcher"
      && native_properties.title == "Flow.Launcher";

    if !is_flow_launcher {
      // Ensure window is top-level (i.e. not a child window). Ignore
      // windows that cannot be focused or if they're unavailable in
      // task switcher (alt+tab menu).
      if native_window.has_window_style(WS_CHILD)
        || native_window
          .has_window_style_ex(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW)
      {
        return Ok(None);
      }

      // Some applications spawn top-level windows for menus that
      // should be ignored. This includes the autocomplete popup in
      // Notepad++ and title bar menu in Keepass. Although not
      // foolproof, these can typically be identified by having an
      // owner window and no title bar.
      if native_window.has_owner_window()
        && !native_window.has_window_style(WS_CAPTION)
      {
        return Ok(None);
      }
    }
  }

  Ok(Some(native_properties))
}

fn create_window(
  native_window: NativeWindow,
  native_properties: NativeWindowProperties,
  target_parent: Option<Container>,
  state: &mut WmState,
  config: &UserConfig,
) -> anyhow::Result<WindowContainer> {
  let nearest_monitor = state
    .nearest_monitor(&native_window)
    .context("No nearest monitor.")?;

  let nearest_workspace = nearest_monitor
    .displayed_workspace()
    .context("No nearest workspace.")?;

  let gaps_config = config.value.gaps.clone();
  let window_state =
    window_state_to_create(&native_properties, &nearest_monitor, config)?;

  // Attach the new window as the first child of the target parent (if
  // provided), otherwise, add as a sibling of the focused container.
  let (target_parent, target_index) = match target_parent {
    Some(parent) => (parent, 0),
    None => insertion_target(&window_state, state)?,
  };

  let target_workspace =
    target_parent.workspace().context("No target workspace.")?;

  let prefers_centered = config
    .value
    .window_behavior
    .state_defaults
    .floating
    .centered;

  // Calculate where window should be placed when floating is enabled. Use
  // the original width/height of the window and optionally position it in
  // the center of the workspace.
  let is_same_workspace = nearest_workspace.id() == target_workspace.id();
  let floating_placement = {
    let placement = if !is_same_workspace || prefers_centered {
      native_properties
        .frame
        .translate_to_center(&target_workspace.to_rect()?)
    } else {
      native_properties.frame.clone()
    };

    // Clamp the window size to be within the workspace's outer gaps. 10px
    // is arbitrary - helps differentiate from tiling windows.
    let max_workspace_rect = target_workspace.max_workspace_rect()?;
    placement.clamp_size(
      max_workspace_rect.width() - 10,
      max_workspace_rect.height() - 10,
    )
  };

  // Window has no border delta unless it's later changed via the
  // `adjust_borders` command.
  let border_delta = RectDelta::zero();

  let window_container: WindowContainer = match window_state {
    WindowState::Tiling => TilingWindow::new(
      None,
      native_window,
      native_properties,
      None,
      border_delta,
      floating_placement,
      false,
      gaps_config,
      Vec::new(),
      None,
    )
    .into(),
    _ => NonTilingWindow::new(
      None,
      native_window,
      native_properties,
      window_state,
      None,
      border_delta,
      None,
      floating_placement,
      !prefers_centered,
      Vec::new(),
      None,
    )
    .into(),
  };

  attach_container(
    &window_container.clone().into(),
    &target_parent,
    Some(target_index),
  )?;

  // The OS might spawn the window on a different monitor to the target
  // parent, so adjustments might need to be made because of DPI.
  if nearest_monitor
    .has_dpi_difference(&window_container.clone().into())?
  {
    window_container.set_has_pending_dpi_adjustment(true);
  }

  Ok(window_container)
}

/// Gets the initial state for a window based on its native state.
///
/// Note that maximized windows are initialized as tiling.
fn window_state_to_create(
  native_properties: &NativeWindowProperties,
  nearest_monitor: &Monitor,
  config: &UserConfig,
) -> anyhow::Result<WindowState> {
  if native_properties.is_minimized {
    return Ok(WindowState::Minimized);
  }

  let nearest_workspace = nearest_monitor
    .displayed_workspace()
    .context("No workspace.")?;

  // Only initialize as fullscreen if the window *exceeds* the workspace
  // bounds (due to the 1px inset).
  //
  // For example, with 0px outer gaps and a window that covers the entire
  // workspace, it would still not be initialized as fullscreen. The window
  // needs to be within the workspace's outer gaps by at least 1px on each
  // side.
  // Only borderless windows that can't be resized (e.g. games) are
  // initialized as fullscreen. A resizable app that restores a
  // screen-sized window from its last session (e.g. a browser) is tiled.
  if !native_properties.is_maximized
    && !native_properties.is_resizable
    && native_properties
      .frame
      .inset(1)
      .contains_rect(&nearest_workspace.max_workspace_rect()?)
  {
    return Ok(WindowState::Fullscreen(
      config
        .value
        .window_behavior
        .state_defaults
        .fullscreen
        .clone(),
    ));
  }

  // Initialize windows that can't be resized as floating.
  if !native_properties.is_resizable {
    return Ok(WindowState::Floating(
      config.value.window_behavior.state_defaults.floating.clone(),
    ));
  }

  Ok(WindowState::default_from_config(&config.value))
}

/// Gets where to insert a new window in the container tree.
///
/// Rules:
/// - For non-tiling windows: Always append to the workspace.
/// - For tiling windows:
///   1. Try to insert after the focused tiling window if one exists.
///   2. If a non-tiling window is focused, try to insert after the first
///      tiling window found.
///   3. If no tiling windows exist, append to the workspace.
///
/// Returns tuple of (parent container, insertion index).
fn insertion_target(
  window_state: &WindowState,
  state: &WmState,
) -> anyhow::Result<(Container, usize)> {
  let focused_container =
    state.focused_container().context("No focused container.")?;

  let focused_workspace =
    focused_container.workspace().context("No workspace.")?;

  // For tiling windows, try to find a suitable tiling window to insert
  // next to.
  if *window_state == WindowState::Tiling {
    let sibling = match focused_container {
      Container::TilingWindow(_) => Some(focused_container),
      _ => focused_workspace
        .descendant_focus_order()
        .find(Container::is_tiling_window),
    };

    if let Some(sibling) = sibling {
      return Ok((
        sibling.parent().context("No parent.")?,
        sibling.index() + 1,
      ));
    }
  }

  // Default to appending to workspace.
  Ok((
    focused_workspace.clone().into(),
    focused_workspace.child_count(),
  ))
}

/// Places a new tiling window like Hyprland's dwindle layout
/// (`onWindowCreatedTiling` with `force_split = 0`).
///
/// The window under the cursor (or else the previously focused tiling
/// window) is split along its longer side, and the new window takes the
/// half the cursor is over. Without `use_cursor`, the last focused window
/// is split and the new window takes the second half, which builds
/// Hyprland's spiral when windows are managed one after another.
pub fn dwindle_place(
  window: &TilingWindow,
  use_cursor: bool,
  state: &WmState,
  config: &UserConfig,
) -> anyhow::Result<()> {
  let workspace = window.workspace().context("No workspace.")?;
  detach_container(window.clone().into())?;

  let others = workspace
    .descendants()
    .filter_map(|container| match container.as_tiling_container() {
      Ok(TilingContainer::TilingWindow(other)) => Some(other),
      _ => None,
    })
    .collect::<Vec<_>>();

  // The cursor position is unavailable while the lock screen or the
  // screen saver is the input desktop (`GetCursorPos` fails with access
  // denied). The window is placed as on startup then, instead of failing
  // after it has already been detached (which lost the window and made
  // the WM unable to start while the screen was locked).
  let cursor = if use_cursor {
    state.dispatcher.cursor_position().ok()
  } else {
    None
  };

  let under_cursor = cursor.as_ref().and_then(|cursor| {
    others.iter().find(|other| {
      other
        .to_rect()
        .is_ok_and(|rect| rect.contains_point(cursor))
    })
  });

  let target = match under_cursor {
    Some(target) => Some(target.clone()),
    None => workspace
      .descendant_focus_order()
      .find_map(|container| match container.as_tiling_container() {
        Ok(TilingContainer::TilingWindow(other)) => Some(other),
        _ => None,
      }),
  };

  match target {
    Some(target) => {
      // Without a cursor: outside the target, so the second half is taken.
      let point = cursor.unwrap_or(Point {
        x: i32::MAX,
        y: i32::MAX,
      });

      dwindle_split(window, &target, &point, None, config)
    }
    // First tiling window on the workspace: it fills the workspace.
    None => attach_container(
      &window.clone().into(),
      &workspace.clone().into(),
      Some(workspace.child_count()),
    ),
  }
}
