use super::NativeWindow;
use crate::{
  platform_impl::WindowEventNotificationInner, Keybinding, MouseEventKind,
  Point, WindowId,
};

#[derive(Clone, Debug)]
pub enum PlatformEvent {
  Window(WindowEvent),
  Keybinding(KeybindingEvent),
  Mouse(MouseEvent),
  DisplaySettingsChanged,
}

#[derive(Clone, Debug)]
pub enum WindowEvent {
  /// Window was created; its initial geometry can be constrained before show.
  Created {
    window: NativeWindow,
    notification: WindowEventNotification,
  },

  /// Window gained focus.
  Focused {
    window: NativeWindow,
    notification: WindowEventNotification,
  },

  /// Window was hidden.
  Hidden {
    window: NativeWindow,
    notification: WindowEventNotification,
  },

  /// Size or position of window has changed.
  ///
  /// `is_interactive_start` and `is_interactive_end` indicate whether the
  /// move or resize was initiated via manual interaction with the
  /// window's drag handles.
  ///
  /// # Platform-specific
  ///
  /// - **Windows**: Corresponds to `EVENT_OBJECT_LOCATIONCHANGE`,
  ///   `EVENT_SYSTEM_MOVESIZESTART`, and `EVENT_SYSTEM_MOVESIZEEND`.
  /// - **macOS**: Corresponds to `AXWindowMoved` and `AXWindowResized`.
  ///   The `is_interactive_start` and `is_interactive_end` flags are
  ///   always `false`.
  MovedOrResized {
    window: NativeWindow,
    is_interactive_start: bool,
    is_interactive_end: bool,
    notification: WindowEventNotification,
  },

  /// Window was minimized.
  Minimized {
    window: NativeWindow,
    notification: WindowEventNotification,
  },

  /// Window was restored from minimized state.
  MinimizeEnded {
    window: NativeWindow,
    notification: WindowEventNotification,
  },

  /// Window became visible.
  Shown {
    window: NativeWindow,
    notification: WindowEventNotification,
  },

  /// Window title changed.
  TitleChanged {
    window: NativeWindow,
    notification: WindowEventNotification,
  },

  /// Window was destroyed.
  Destroyed {
    window_id: WindowId,
    notification: WindowEventNotification,
  },
}

impl WindowEvent {
  /// Logical Lunge: the event kinds' names, in [`Self::kind_index`] order
  /// (for the window manager's event rate log).
  pub const KIND_NAMES: [&'static str; 9] = [
    "created",
    "focused",
    "hidden",
    "moved",
    "minimized",
    "minimize_ended",
    "shown",
    "title",
    "destroyed",
  ];

  /// Logical Lunge: the event's kind as an index into [`Self::KIND_NAMES`].
  #[must_use]
  pub fn kind_index(&self) -> usize {
    match self {
      Self::Created { .. } => 0,
      Self::Focused { .. } => 1,
      Self::Hidden { .. } => 2,
      Self::MovedOrResized { .. } => 3,
      Self::Minimized { .. } => 4,
      Self::MinimizeEnded { .. } => 5,
      Self::Shown { .. } => 6,
      Self::TitleChanged { .. } => 7,
      Self::Destroyed { .. } => 8,
    }
  }

  /// Logical Lunge: events that only say "this window's position (or
  /// title) changed, look again" share a key per window and kind: the
  /// handlers read the window's live state, so of several queued at once
  /// the last one is enough. The start and end of a drag are events of
  /// their own and have none.
  #[must_use]
  pub fn coalesce_key(&self) -> Option<(usize, WindowId)> {
    match self {
      Self::MovedOrResized {
        window,
        is_interactive_start: false,
        is_interactive_end: false,
        ..
      }
      | Self::TitleChanged { window, .. } => {
        Some((self.kind_index(), window.id()))
      }
      _ => None,
    }
  }

  /// Get the window handle if available (not available for
  /// `WindowEvent::Destroyed`).
  #[must_use]
  pub fn window(&self) -> Option<&NativeWindow> {
    match self {
      Self::Created { window, .. }
      | Self::Focused { window, .. }
      | Self::Hidden { window, .. }
      | Self::MovedOrResized { window, .. }
      | Self::Minimized { window, .. }
      | Self::MinimizeEnded { window, .. }
      | Self::Shown { window, .. }
      | Self::TitleChanged { window, .. } => Some(window),
      Self::Destroyed { .. } => None,
    }
  }

  /// Returns the platform-specific window event notification.
  #[must_use]
  pub fn notification(&self) -> &WindowEventNotification {
    match self {
      Self::Created { notification, .. }
      | Self::Focused { notification, .. }
      | Self::Hidden { notification, .. }
      | Self::MovedOrResized { notification, .. }
      | Self::Minimized { notification, .. }
      | Self::MinimizeEnded { notification, .. }
      | Self::Shown { notification, .. }
      | Self::TitleChanged { notification, .. }
      | Self::Destroyed { notification, .. } => notification,
    }
  }
}

/// Platform-specific window event notification.
///
/// Some events are "synthetic" and do not have a corresponding
/// notification (represented by `None`).
///
/// Synthetic events can occur when:
/// * On macOS, `WindowEvent::Shown` is emitted for new visible windows
///   even if a different notification is received first.
#[derive(Clone, Debug)]
pub struct WindowEventNotification(
  pub Option<WindowEventNotificationInner>,
);

#[derive(Clone, Debug)]
pub struct KeybindingEvent(pub Keybinding);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MouseButton {
  Left,
  Right,
}

/// Tracks which mouse buttons are currently pressed.
#[derive(Clone, Copy, Debug, Default)]
pub struct PressedButtons {
  left: bool,
  right: bool,
}

impl PressedButtons {
  /// Returns whether the given button is currently pressed.
  #[must_use]
  pub fn contains(&self, button: &MouseButton) -> bool {
    match button {
      MouseButton::Left => self.left,
      MouseButton::Right => self.right,
    }
  }

  /// Updates button state based on a mouse event.
  pub(crate) fn update(&mut self, event: MouseEventKind) {
    match event {
      MouseEventKind::LeftButtonDown => self.left = true,
      MouseEventKind::LeftButtonUp => self.left = false,
      MouseEventKind::RightButtonDown => self.right = true,
      MouseEventKind::RightButtonUp => self.right = false,
      MouseEventKind::Move => {}
    }
  }
}

#[derive(Clone, Debug)]
pub enum MouseEvent {
  /// Mouse cursor moved.
  Move {
    position: Point,
    pressed_buttons: PressedButtons,
    /// Window under cursor.
    ///
    /// # Platform-specific
    ///
    /// - **macOS**: Sourced from the `CGEvent` field. Unreliable; often
    ///   `None`, with the real window ID appearing sporadically.
    /// - **Windows**: Always `None`.
    window_below_cursor: Option<WindowId>,
  },

  /// A mouse button was pressed.
  ButtonDown {
    position: Point,
    button: MouseButton,
    pressed_buttons: PressedButtons,
  },

  /// A mouse button was released.
  ButtonUp {
    position: Point,
    button: MouseButton,
    pressed_buttons: PressedButtons,
  },
}

impl MouseEvent {
  /// Returns the cursor position at the time of the event.
  ///
  /// `0,0` is the top-left corner of the primary monitor.
  #[must_use]
  pub fn position(&self) -> &Point {
    match self {
      Self::Move { position, .. }
      | Self::ButtonDown { position, .. }
      | Self::ButtonUp { position, .. } => position,
    }
  }

  /// Returns which mouse buttons were pressed at the time of the event.
  #[must_use]
  pub fn pressed_buttons(&self) -> &PressedButtons {
    match self {
      Self::Move {
        pressed_buttons, ..
      }
      | Self::ButtonDown {
        pressed_buttons, ..
      }
      | Self::ButtonUp {
        pressed_buttons, ..
      } => pressed_buttons,
    }
  }
}
