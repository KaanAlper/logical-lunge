use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;

use crate::{platform_impl, Dispatcher, WindowEvent};

/// Logical Lunge: window events received from the system, per kind
/// ([`WindowEvent::kind_index`]), counted where they arrive (the hook), so
/// a storm shows even while the window manager's thread is busy.
static RECEIVED: [AtomicU64; WindowEvent::KIND_NAMES.len()] =
  [const { AtomicU64::new(0) }; WindowEvent::KIND_NAMES.len()];

/// Counts an event that arrived (see [`window_events_received`]).
pub(crate) fn note_received(event: &WindowEvent) {
  RECEIVED[event.kind_index()].fetch_add(1, Ordering::Relaxed);
}

/// Logical Lunge: how many window events of each kind have arrived since
/// the start ([`WindowEvent::KIND_NAMES`] order).
#[must_use]
pub fn window_events_received() -> [u64; WindowEvent::KIND_NAMES.len()] {
  std::array::from_fn(|i| RECEIVED[i].load(Ordering::Relaxed))
}

/// A listener for system-wide window events.
pub struct WindowListener {
  event_rx: mpsc::UnboundedReceiver<WindowEvent>,

  /// Inner platform-specific window listener.
  inner: platform_impl::WindowListener,
}

impl WindowListener {
  /// Creates a new window listener.
  pub fn new(dispatcher: &Dispatcher) -> crate::Result<Self> {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let inner = platform_impl::WindowListener::new(event_tx, dispatcher)?;

    Ok(Self { event_rx, inner })
  }

  /// Returns the next window event from the listener.
  ///
  /// This will block until a window event is available.
  pub async fn next_event(&mut self) -> Option<WindowEvent> {
    self.event_rx.recv().await
  }

  /// Logical Lunge: the next event if one is already waiting (for taking
  /// a burst at once, see the window manager's event coalescing).
  pub fn try_next_event(&mut self) -> Option<WindowEvent> {
    self.event_rx.try_recv().ok()
  }

  /// Terminates the window listener.
  pub fn terminate(&mut self) {
    self.inner.terminate();
  }
}
