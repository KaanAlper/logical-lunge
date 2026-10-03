//! The shell's in-process event bus: typed events between its parts.
//!
//! A part publishes what happened and does not need to know who reacts to
//! it (the Super menu sends a notification card without holding the bar,
//! the right panel's keyboard tile opens the on-screen keyboard). Events
//! from the core (its `ll:*` stream) are read by the bar itself
//! (`native_bar::core_api`); this bus carries the shell's own.

use std::sync::{Arc, RwLock};

/// What one part of the shell tells the others.
#[derive(Clone, Debug)]
pub enum Event {
  /// A notification card (its JSON: kind, title, body, icon, ...).
  Toast(serde_json::Value),
  /// Open the right panel at a page ("keys", "walls", "screensaver", "bug").
  SidebarOpenPage(String),
  /// Show or hide the on-screen keyboard.
  OskToggle,
}

type Subscriber = Arc<dyn Fn(&Event) + Send + Sync>;

static SUBSCRIBERS: RwLock<Vec<Subscriber>> = RwLock::new(Vec::new());

/// Calls `f` for every event published from now on (from the publishing
/// thread: a subscriber hands the event to its own thread if it must).
pub fn subscribe(f: impl Fn(&Event) + Send + Sync + 'static) {
  SUBSCRIBERS
    .write()
    .unwrap_or_else(|e| e.into_inner())
    .push(Arc::new(f));
}

/// Hands `event` to every subscriber.
pub fn publish(event: Event) {
  // a copy of the list: a subscriber may publish or subscribe in turn
  let subscribers = SUBSCRIBERS
    .read()
    .unwrap_or_else(|e| e.into_inner())
    .clone();
  for s in subscribers {
    s(&event);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicUsize, Ordering};

  #[test]
  fn every_subscriber_gets_each_event() {
    static PAGES: AtomicUsize = AtomicUsize::new(0);
    static KEYBOARD: AtomicUsize = AtomicUsize::new(0);
    subscribe(|e| {
      if let Event::SidebarOpenPage(p) = e {
        if p == "bus-test" {
          PAGES.fetch_add(1, Ordering::SeqCst);
        }
      }
    });
    subscribe(|e| {
      if matches!(e, Event::OskToggle) {
        KEYBOARD.fetch_add(1, Ordering::SeqCst);
      }
    });
    publish(Event::SidebarOpenPage("bus-test".into()));
    publish(Event::OskToggle);
    assert_eq!(PAGES.load(Ordering::SeqCst), 1);
    assert!(KEYBOARD.load(Ordering::SeqCst) >= 1);
  }
}
