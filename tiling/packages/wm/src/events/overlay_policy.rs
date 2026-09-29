//! Application-independent policy for passive visual overlays.
//! Names, titles and browser brands are deliberately not classification inputs.
#[derive(Default, Clone, Copy)]
pub(super) struct OverlayFacts {
  pub related_to_tile: bool,
  pub covers_monitor: bool,
  pub layered: bool,
  pub click_through: bool,
  pub no_activate: bool,
  pub caption: bool,
  pub taskbar_entry: bool,
  pub child: bool,
}

pub(super) fn is_transition_overlay(f: OverlayFacts) -> bool {
  f.related_to_tile && f.covers_monitor && f.layered
    && f.click_through && f.no_activate
    && !f.caption && !f.taskbar_entry && !f.child
}

#[cfg(test)]
mod tests {
  use super::*;
  fn fade() -> OverlayFacts {
    OverlayFacts { related_to_tile: true, covers_monitor: true, layered: true,
      click_through: true, no_activate: true, ..Default::default() }
  }
  #[test]
  fn passive_effect_is_scoped_without_application_identity() {
    assert!(is_transition_overlay(fade()));
  }
  #[test]
  fn unrelated_and_interactive_windows_are_never_constrained() {
    for other in [
      OverlayFacts { related_to_tile: false, ..fade() }, // system/screenshot overlay
      OverlayFacts { covers_monitor: false, ..fade() }, // tooltip or local popup
      OverlayFacts { no_activate: false, ..fade() }, // interactive dialog
      OverlayFacts { click_through: false, ..fade() }, // interactive full-screen UI
      OverlayFacts { taskbar_entry: true, ..fade() }, // separate app window
      OverlayFacts { caption: true, ..fade() },
      OverlayFacts { child: true, ..fade() },
      OverlayFacts { layered: false, ..fade() },
    ] { assert!(!is_transition_overlay(other)); }
  }
}
