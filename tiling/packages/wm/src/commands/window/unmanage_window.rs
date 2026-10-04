use anyhow::Context;
use wm_common::{WindowState, WmEvent};

use crate::{
  commands::container::{
    detach_container, flatten_child_split_containers, normalize_split_containers,
    set_focused_descendant,
  },
  models::WindowContainer,
  traits::{CommonGetters, PositionGetters, WindowGetters},
  wm_state::WmState,
};

#[allow(clippy::needless_pass_by_value)]
pub fn unmanage_window(
  window: WindowContainer,
  state: &mut WmState,
) -> anyhow::Result<()> {
  #[cfg(target_os = "windows")]
  {
    use wm_platform::NativeWindowWindowsExt;
    let handle = window.native().hwnd().0;
    if state.fullscreen_marks.remove(&handle) == Some(true)
      && window.native().is_valid()
    {
      if let Err(err) = window.native().mark_fullscreen(false) {
        tracing::warn!("Failed to clear fullscreen mark on unmanage: {}", err);
      }
    }
    state.fake_fullscreen.remove(&handle);
    state.self_resizes.remove(&handle);
  }

  // Create iterator of parent, grandparent, and great-grandparent.
  let ancestors = window.ancestors().take(3).collect::<Vec<_>>();

  // Get container to switch focus to after the window has been removed.
  let focus_target = state.focus_target_after_removal(&window.clone());

  // Logical Lunge: as in Hyprland, focus leaving a closed tiling window goes
  // to the window that takes over its space, i.e. the one at its center once
  // the gap is closed. The focus history is the fallback.
  let workspace = window.workspace();
  let freed_center = focus_target
    .as_ref()
    .filter(|_| window.state() == WindowState::Tiling)
    .and_then(|_| window.to_rect().ok())
    .map(|rect| rect.center_point());

  detach_container(window.clone().into())?;

  // After detaching the container, flatten any redundant split containers.
  // For example, in the layout V[1 H[2]] where container 1 is detached to
  // become V[H[2]], this will then need to be flattened to V[2].
  for ancestor in ancestors.iter().rev() {
    flatten_child_split_containers(ancestor)?;
  }

  // Deeper leftovers too (e.g. a split left with a single child), which
  // otherwise make later moves produce extra columns.
  if let Some(workspace) = ancestors.iter().find_map(|a| a.as_workspace().cloned()) {
    normalize_split_containers(&workspace.clone().into())?;
    state
      .pending_sync
      .queue_containers_to_redraw(workspace.tiling_children());
  }

  state.emit_event(WmEvent::WindowUnmanaged {
    unmanaged_id: window.id(),
    #[allow(clippy::cast_possible_wrap, clippy::unnecessary_cast)]
    unmanaged_handle: window.native().id().0 as isize,
  });

  let focus_target = freed_center
    .zip(workspace)
    .and_then(|(center, workspace)| {
      workspace
        .descendants()
        .filter(|c| c.as_tiling_window().is_some())
        .find(|c| c.to_rect().is_ok_and(|rect| rect.contains_point(&center)))
    })
    .or(focus_target);

  // Reassign focus to suitable target.
  if let Some(focus_target) = focus_target {
    set_focused_descendant(&focus_target, None);
    state.pending_sync.queue_focus_change();
    state.unmanaged_or_minimized_timestamp =
      Some(std::time::Instant::now());
  }

  // Sibling containers need to be redrawn if the window was tiling.
  if window.state() == WindowState::Tiling {
    let ancestor_to_redraw = ancestors
      .into_iter()
      .find(|ancestor| !ancestor.is_detached())
      .context("No ancestor to redraw.")?;

    state
      .pending_sync
      .queue_containers_to_redraw(ancestor_to_redraw.tiling_children());
  }

  Ok(())
}
