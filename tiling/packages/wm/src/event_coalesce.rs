//! Logical Lunge: window events are taken in bursts. A window that moves
//! or renames itself many times a second (an animation, an app fighting
//! its size, a title counting) queued an event each time, and each was
//! handled on its own with a full sync after it: under such a storm the
//! WM thread was always busy with stale moves and every IPC command
//! (workspace switches) waited behind them. Of the events queued at once,
//! only the last "look again" event of each window and kind is handled (the
//! handlers read the window's live state); every other event stays, in
//! order.

use std::{collections::HashMap, hash::Hash};

use wm_platform::WindowEvent;

/// The most events taken in one burst (the IPC gets its turn after it).
pub const MAX_BURST: usize = 512;

/// `first` and the events already waiting behind it (at most
/// `MAX_BURST`), coalesced (see the module documentation).
pub fn take_burst(
  first: WindowEvent,
  mut next: impl FnMut() -> Option<WindowEvent>,
) -> Vec<WindowEvent> {
  let mut events = vec![first];
  while events.len() < MAX_BURST {
    match next() {
      Some(event) => events.push(event),
      None => break,
    }
  }
  keep_last_per_key(events, WindowEvent::coalesce_key)
}

/// Keeps, of the items sharing a key, only the last one (in its place);
/// items without a key all stay. Order is otherwise unchanged.
pub fn keep_last_per_key<T, K: Eq + Hash>(
  items: Vec<T>,
  key: impl Fn(&T) -> Option<K>,
) -> Vec<T> {
  let keys: Vec<Option<K>> = items.iter().map(&key).collect();
  let mut last: HashMap<&K, usize> = HashMap::new();
  for (index, item_key) in keys.iter().enumerate() {
    if let Some(item_key) = item_key {
      last.insert(item_key, index);
    }
  }
  let keep: Vec<bool> = keys
    .iter()
    .enumerate()
    .map(|(index, item_key)| {
      item_key.as_ref().is_none_or(|k| last.get(k) == Some(&index))
    })
    .collect();
  items
    .into_iter()
    .zip(keep)
    .filter_map(|(item, kept)| kept.then_some(item))
    .collect()
}

#[cfg(test)]
mod tests {
  use super::keep_last_per_key;

  /// (key, payload): key 0 means "no key"
  fn key(item: &(u32, &str)) -> Option<u32> {
    (item.0 != 0).then_some(item.0)
  }

  #[test]
  fn a_storm_of_one_window_becomes_its_last_event() {
    let items = vec![(7, "a"), (7, "b"), (7, "c")];
    assert_eq!(keep_last_per_key(items, key), vec![(7, "c")]);
  }

  #[test]
  fn other_events_stay_in_order_around_the_kept_ones() {
    let items = vec![(1, "move a"), (0, "focus"), (2, "move b"), (1, "move a again"), (0, "hide")];
    assert_eq!(
      keep_last_per_key(items, key),
      vec![(0, "focus"), (2, "move b"), (1, "move a again"), (0, "hide")]
    );
  }

  #[test]
  fn nothing_to_coalesce_keeps_everything() {
    let items = vec![(0, "x"), (1, "y"), (2, "z")];
    assert_eq!(keep_last_per_key(items.clone(), key), items);
  }
}
