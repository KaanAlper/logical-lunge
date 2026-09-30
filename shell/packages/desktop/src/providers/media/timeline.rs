/// Browsers may briefly publish a 0/0 timeline while a seek is in flight.
/// A real seek to the start retains the video's duration.
pub(crate) fn accept_snapshot(previous_end_seconds: u64, position_ticks: i64, end_ticks: i64) -> bool {
  previous_end_seconds == 0 || position_ticks > 0 || end_ticks > 0
}

#[cfg(test)]
mod tests {
  use super::accept_snapshot;

  #[test]
  fn transient_empty_event_does_not_erase_known_video() {
    assert!(!accept_snapshot(900, 0, 0));
  }

  #[test]
  fn explicit_seek_to_zero_keeps_known_duration() {
    assert!(accept_snapshot(900, 0, 9_008_210_000));
  }

  #[test]
  fn fresh_session_and_positive_position_are_accepted() {
    assert!(accept_snapshot(0, 0, 0));
    assert!(accept_snapshot(900, 980_002_865, 9_008_210_000));
    assert!(accept_snapshot(900, 980_002_865, 0));
  }
}
