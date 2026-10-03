//! The interface scale (prefs.json "uiScale", percent; Settings > Görünüm):
//! every native window draws at its monitor's DPI scale times this factor.
//! Layout stays in DIPs, so the bar, panels, menus, cards and widgets grow
//! or shrink together; windows that would not fit their monitor at the new
//! size are clamped to it (`fit`) and scroll or shrink their content.

use std::sync::atomic::{AtomicU32, Ordering};

/// The choices the settings offer (the core accepts only these).
pub const STEPS: [u32; 6] = [85, 90, 100, 110, 125, 150];

static PERCENT: AtomicU32 = AtomicU32::new(100);

pub fn percent() -> u32 {
  PERCENT.load(Ordering::Relaxed)
}

/// A step from the preferences (anything else is 100 %).
pub fn from_pref(v: Option<u64>) -> u32 {
  v.and_then(|p| u32::try_from(p).ok()).filter(|p| STEPS.contains(p)).unwrap_or(100)
}

/// Sets the factor; true when it changed (the windows are made again).
pub fn set_percent(p: u32) -> bool {
  let p = if STEPS.contains(&p) { p } else { 100 };
  PERCENT.swap(p, Ordering::Relaxed) != p
}

/// A monitor's DPI (96 = 100 %) to the scale native windows draw at.
pub fn of_dpi(dpi: u32) -> f32 {
  scale_for(dpi, percent())
}

pub fn scale_for(dpi: u32, percent: u32) -> f32 {
  dpi.max(1) as f32 / 96.0 * percent as f32 / 100.0
}

/// The largest DIP length up to `want` that fits `avail_px` pixels at
/// `scale` (a window wider or taller than its monitor at a large scale).
pub fn fit(want: f32, avail_px: i32, scale: f32) -> f32 {
  want.min((avail_px.max(0) as f32 / scale.max(0.01)).floor())
}

/// The scale a window `dip` wide (or tall) is drawn at so it fits
/// `avail_px` pixels: `want`, or smaller when the whole window would not fit
/// (it shrinks as one, like the settings window).
pub fn fit_scale(want: f32, dip: f32, avail_px: i32) -> f32 {
  want.min(avail_px.max(1) as f32 / dip.max(1.0))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn scale_is_dpi_times_the_setting() {
    assert_eq!(scale_for(96, 100), 1.0);
    assert_eq!(scale_for(144, 100), 1.5);
    assert_eq!(scale_for(96, 125), 1.25);
    assert!((scale_for(120, 150) - 1.875).abs() < 1e-6);
  }

  #[test]
  fn only_offered_steps_are_taken() {
    assert_eq!(from_pref(Some(125)), 125);
    assert_eq!(from_pref(Some(101)), 100);
    assert_eq!(from_pref(None), 100);
    assert_eq!(from_pref(Some(u64::MAX)), 100);
  }

  #[test]
  fn fit_keeps_windows_on_their_monitor() {
    // 1000 DIP wanted, 1366 px monitor at 150 %: 910 DIP fit
    assert_eq!(fit(1000.0, 1366, 1.5), 910.0);
    // fits already: unchanged
    assert_eq!(fit(600.0, 1920, 1.0), 600.0);
    // a broken monitor size never gives a negative length
    assert_eq!(fit(600.0, -5, 1.0), 0.0);
  }

  #[test]
  fn a_window_too_wide_shrinks_as_one() {
    // a 900 DIP keyboard at 150 % on a 1280 px monitor: 1280 / 900
    assert!((fit_scale(1.5, 900.0, 1280) - 1280.0 / 900.0).abs() < 1e-6);
    // it fits: the wanted scale
    assert_eq!(fit_scale(1.25, 900.0, 1920), 1.25);
  }
}
