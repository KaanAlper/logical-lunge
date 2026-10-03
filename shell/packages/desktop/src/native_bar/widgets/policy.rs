//! Pure decisions for retained widget surfaces and desktop stacking.
//! This module can also be tested with `rustc --test policy.rs` without a desktop.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role { Desktop, Widget, Editing, Other }

#[derive(Clone, Copy, Debug)]
pub(crate) struct Window {
  pub role: Role,
  pub visible: bool,
  pub cloaked: bool,
  pub topmost: bool,
}

/// Windows are supplied in front-to-back order.
pub(crate) fn layer_is_correct(stack: &[Window]) -> bool {
  if !stack.iter().any(|w| w.role == Role::Desktop) { return true; }
  let mut widgets_started = false;
  let mut desktop_seen = false;
  for w in stack {
    match w.role {
      Role::Desktop => desktop_seen = true,
      Role::Editing => {}, // A note/place intentionally takes the foreground.
      Role::Widget => {
        if desktop_seen || w.topmost { return false; }
        widgets_started = true;
      },
      Role::Other if w.visible && !w.cloaked && !w.topmost => {
        if widgets_started && !desktop_seen { return false; }
      },
      Role::Other => {},
    }
  }
  true
}

#[derive(Default)]
pub(crate) struct Repaint {
  last_key: Option<u64>,
}

impl Repaint {
  pub fn should_draw(&self, key: u64, force: bool) -> bool {
    force || self.last_key != Some(key)
  }

  pub fn presented(&mut self, key: u64) {
    self.last_key = Some(key);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn window(role: Role) -> Window {
    Window { role, visible: true, cloaked: false, topmost: false }
  }

  #[test]
  fn workspace_cloaking_and_window_close_do_not_move_correct_widgets() {
    let widget = window(Role::Widget);
    let desktop = window(Role::Desktop);
    let mut app = window(Role::Other);
    app.cloaked = true;
    assert!(layer_is_correct(&[widget, app, desktop]));
    app.cloaked = false;
    app.visible = false;
    assert!(layer_is_correct(&[widget, app, desktop]));
    assert!(layer_is_correct(&[widget, desktop]));
  }

  #[test]
  fn hidden_notification_or_error_window_does_not_restack_widgets() {
    let mut notification = window(Role::Other);
    notification.visible = false;
    assert!(layer_is_correct(&[window(Role::Widget), notification, window(Role::Desktop)]));
    notification.visible = true;
    notification.topmost = true;
    assert!(layer_is_correct(&[notification, window(Role::Widget), window(Role::Desktop)]));
  }

  #[test]
  fn every_widget_must_be_above_the_desktop() {
    assert!(!layer_is_correct(&[window(Role::Widget), window(Role::Desktop), window(Role::Widget)]));
    assert!(!layer_is_correct(&[window(Role::Desktop), window(Role::Widget)]));
  }

  #[test]
  fn visible_app_below_widgets_requires_repair() {
    assert!(!layer_is_correct(&[window(Role::Widget), window(Role::Other), window(Role::Desktop)]));
  }

  #[test]
  fn finished_editor_must_leave_the_topmost_band() {
    let mut widget = window(Role::Widget);
    widget.topmost = true;
    assert!(!layer_is_correct(&[widget, window(Role::Desktop)]));
    widget.topmost = false;
    assert!(layer_is_correct(&[widget, window(Role::Desktop)]));
  }

  #[test]
  fn multiple_widgets_and_raised_editor_keep_their_positions() {
    let mut editor = window(Role::Editing);
    editor.topmost = true;
    assert!(layer_is_correct(&[editor, window(Role::Other), window(Role::Widget), window(Role::Widget), window(Role::Desktop)]));
    assert!(layer_is_correct(&[editor, window(Role::Other), window(Role::Desktop)]));
  }

  #[test]
  fn missing_desktop_does_not_trigger_repair_on_every_provider_event() {
    assert!(layer_is_correct(&[window(Role::Other), window(Role::Widget)]));
  }

  #[test]
  fn failed_draw_or_commit_remains_dirty_for_the_next_refresh() {
    let mut state = Repaint::default();
    assert!(state.should_draw(10, false));
    // No successful draw/commit: the same provider data must be retried.
    assert!(state.should_draw(10, false));
    state.presented(10);
    assert!(!state.should_draw(10, false));
  }

  #[test]
  fn unrelated_events_keep_presented_surface_but_content_and_resize_redraw() {
    let mut state = Repaint::default();
    assert!(state.should_draw(0, false));
    state.presented(0);
    for _ in 0..100 { assert!(!state.should_draw(0, false)); }
    assert!(state.should_draw(1, false));
    state.presented(1);
    assert!(!state.should_draw(1, false));
    assert!(state.should_draw(1, true));
  }
}
