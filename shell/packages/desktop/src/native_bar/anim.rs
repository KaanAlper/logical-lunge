//! Animations that run in the compositor. A CSS cubic-bezier easing is
//! sampled into cubic segments of an `IDCompositionAnimation`: after
//! `Commit` the compositor plays it at the display's refresh rate and the bar
//! thread does no work per frame.
//!
//! The compositor does not report where an animation currently is, so every
//! animated value keeps its own tween: a new target starts from the value
//! the running animation has reached (no jumps when retargeted mid-way).

use std::time::Instant;

use windows::{
  core::Result,
  Win32::Graphics::DirectComposition::{IDCompositionAnimation, IDCompositionDesktopDevice},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve(pub f32, pub f32, pub f32, pub f32);

/// styles.css `--outSine` (ii OutSine): workspace indicator.
pub const OUT_SINE: Curve = Curve(0.61, 1.0, 0.88, 1.0);
/// `popDown` / `osdIn`: popups sliding out of the bar.
pub const POP_IN: Curve = Curve(0.05, 0.7, 0.1, 1.0);
/// `popUp`: popups going back.
pub const POP_OUT: Curve = Curve(0.3, 0.0, 0.8, 0.15);

impl Curve {
  fn bezier(a: f32, b: f32, s: f32) -> f32 {
    // cubic bezier with P0 = 0, P3 = 1
    let u = 1.0 - s;
    3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
  }

  fn bezier_d(a: f32, b: f32, s: f32) -> f32 {
    let u = 1.0 - s;
    3.0 * u * u * a + 6.0 * u * s * (b - a) + 3.0 * s * s * (1.0 - b)
  }

  /// Eased progress at time fraction `t` (0..1).
  pub fn at(&self, t: f32) -> f32 {
    if t <= 0.0 {
      return 0.0;
    }
    if t >= 1.0 {
      return 1.0;
    }
    // solve x(s) = t: Newton, then bisection if it wanders
    let mut s = t;
    for _ in 0..8 {
      let x = Self::bezier(self.0, self.2, s) - t;
      if x.abs() < 1e-6 {
        return Self::bezier(self.1, self.3, s);
      }
      let d = Self::bezier_d(self.0, self.2, s);
      if d.abs() < 1e-6 {
        break;
      }
      s -= x / d;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    s = t;
    for _ in 0..40 {
      let x = Self::bezier(self.0, self.2, s);
      if (x - t).abs() < 1e-6 {
        break;
      }
      if x < t {
        lo = s;
      } else {
        hi = s;
      }
      s = (lo + hi) / 2.0;
    }
    Self::bezier(self.1, self.3, s)
  }

  fn slope(&self, t: f32) -> f32 {
    let h = 1e-3;
    let (a, b) = ((t - h).max(0.0), (t + h).min(1.0));
    (self.at(b) - self.at(a)) / (b - a)
  }
}

const SEGMENTS: usize = 12;

/// `from` -> `to` over `ms` with `curve`, as compositor cubic segments
/// (Hermite fit of the eased curve on each segment).
pub fn build(
  dcomp: &IDCompositionDesktopDevice,
  from: f32,
  to: f32,
  ms: f32,
  curve: Curve,
) -> Result<IDCompositionAnimation> {
  let anim = unsafe { dcomp.CreateAnimation()? };
  let dur = (ms / 1000.0).max(0.001);
  let delta = to - from;
  for (t0, a, b, c, d) in segments(curve, dur) {
    unsafe {
      anim.AddCubic(t0 as f64, from + delta * a, delta * b, delta * c, delta * d)?;
    }
  }
  unsafe { anim.End(dur as f64, to)? };
  Ok(anim)
}

/// (start seconds, a, b, c, d) with value(τ) = a + bτ + cτ² + dτ³ of the
/// eased progress, τ in seconds from the segment start.
fn segments(curve: Curve, dur: f32) -> Vec<(f32, f32, f32, f32, f32)> {
  (0..SEGMENTS)
    .map(|i| {
      let t0 = i as f32 / SEGMENTS as f32;
      let t1 = (i + 1) as f32 / SEGMENTS as f32;
      let (p0, p1) = (curve.at(t0), curve.at(t1));
      let span = t1 - t0;
      let (m0, m1) = (curve.slope(t0) * span, curve.slope(t1) * span);
      // Hermite on u in 0..1
      let c_u = 3.0 * (p1 - p0) - 2.0 * m0 - m1;
      let d_u = 2.0 * (p0 - p1) + m0 + m1;
      let seg = dur * span;
      (t0 * dur, p0, m0 / seg, c_u / (seg * seg), d_u / (seg * seg * seg))
    })
    .collect()
}

/// One animated value and where it is going.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
  pub from: f32,
  pub to: f32,
  pub start: Instant,
  pub ms: f32,
  pub curve: Curve,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Animated {
  tween: Option<Tween>,
  value: f32,
}

impl Animated {
  pub fn new(value: f32) -> Self {
    Self { tween: None, value }
  }

  /// Where the value is right now (mid-animation included).
  pub fn current(&self) -> f32 {
    match self.tween {
      Some(t) => {
        let f = (t.start.elapsed().as_secs_f32() * 1000.0 / t.ms).clamp(0.0, 1.0);
        t.from + (t.to - t.from) * t.curve.at(f)
      }
      None => self.value,
    }
  }

  pub fn target(&self) -> f32 {
    self.tween.map_or(self.value, |t| t.to)
  }

  /// Starts moving to `to` from wherever it is; None when already going there.
  pub fn to(&mut self, dcomp: &IDCompositionDesktopDevice, to: f32, ms: f32, curve: Curve) -> Result<Option<IDCompositionAnimation>> {
    if (self.target() - to).abs() < 0.01 {
      return Ok(None);
    }
    let from = self.current();
    self.tween = Some(Tween { from, to, start: Instant::now(), ms, curve });
    self.value = to;
    build(dcomp, from, to, ms, curve).map(Some)
  }

  /// Jumps (no animation).
  pub fn set(&mut self, v: f32) {
    self.tween = None;
    self.value = v;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn curve_ends_and_shape() {
    for c in [OUT_SINE, POP_IN, POP_OUT] {
      assert!(c.at(0.0).abs() < 1e-4);
      assert!((c.at(1.0) - 1.0).abs() < 1e-4);
    }
    // ease-out: well ahead of linear at the middle
    assert!(OUT_SINE.at(0.5) > 0.6);
    // ease-in: behind linear
    assert!(POP_OUT.at(0.5) < 0.4);
  }

  #[test]
  fn segments_follow_the_curve() {
    let dur = 0.3;
    for c in [OUT_SINE, POP_IN, POP_OUT] {
      let segs = segments(c, dur);
      for k in 0..=300 {
        let t = k as f32 / 300.0 * dur;
        let (t0, a, b, cc, d) = *segs.iter().rev().find(|s| s.0 <= t + 1e-6).unwrap();
        let tau = t - t0;
        let v = a + b * tau + cc * tau * tau + d * tau * tau * tau;
        let want = c.at(t / dur);
        assert!((v - want).abs() < 0.01, "{:?} at {}: {} vs {}", c, t, v, want);
      }
    }
  }
}
