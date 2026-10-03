use std::str::FromStr;

use anyhow::bail;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A length on a monitor: a share of the monitor's size plus pixels, e.g.
/// `40px`, `100%`, or `100% - 40px` (a panel under a 40 px bar). The pixel
/// part follows the monitor's scale; the share does not need to.
#[derive(Debug, Clone, PartialEq)]
pub struct LengthValue {
  /// percent of the total
  pub percent: f32,
  /// pixels (scaled by `to_px_scaled`)
  pub px: f32,
}

impl LengthValue {
  pub fn to_px(&self, total_px: i32) -> i32 {
    self.to_px_scaled(total_px, 1.)
  }

  pub fn to_px_scaled(&self, total_px: i32, scale_factor: f32) -> i32 {
    // each part rounded down on its own, as a single `%` or `px` value was
    (self.percent / 100. * total_px as f32) as i32 + (scale_factor * self.px) as i32
  }
}

impl FromStr for LengthValue {
  type Err = anyhow::Error;

  /// Parses a sum of terms, each a number with `%` or `px` (no unit:
  /// pixels), joined by `+` or `-`: `100px`, `50.5%`, `100% - 40px`.
  /// Anything else in the text is an error.
  fn from_str(unparsed: &str) -> anyhow::Result<Self> {
    let err = || anyhow::anyhow!("Not a valid length value '{}'. Must be like '10px', '10%' or '100% - 10px'.", unparsed);
    let mut value = LengthValue { percent: 0., px: 0. };
    let mut rest = unparsed.trim();
    let mut sign = 1.;
    let mut first = true;
    loop {
      if !first {
        rest = rest.trim_start();
        sign = match rest.chars().next() {
          Some('+') => 1.,
          Some('-') => -1.,
          _ => return Err(err()),
        };
        rest = rest[1..].trim_start();
      }
      first = false;
      let end = rest
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && (c == '+' || c == '-'))))
        .map_or(rest.len(), |(i, _)| i);
      let amount = f32::from_str(&rest[..end]).map_err(|_| err())?;
      if !amount.is_finite() {
        bail!(err());
      }
      rest = &rest[end..];
      if let Some(r) = rest.strip_prefix('%') {
        value.percent += sign * amount;
        rest = r;
      } else {
        value.px += sign * amount;
        rest = rest.strip_prefix("px").unwrap_or(rest);
      }
      if rest.trim().is_empty() {
        return Ok(value);
      }
    }
  }
}

impl Serialize for LengthValue {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let s = match (self.percent != 0., self.px != 0.) {
      (true, false) => format!("{}%", self.percent),
      (true, true) => format!("{}% {} {}px", self.percent, if self.px < 0. { '-' } else { '+' }, self.px.abs()),
      _ => format!("{}px", self.px),
    };

    serializer.serialize_str(&s)
  }
}

impl<'de> Deserialize<'de> for LengthValue {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let s = String::deserialize(deserializer)?;
    LengthValue::from_str(&s).map_err(serde::de::Error::custom)
  }
}

impl Default for LengthValue {
  fn default() -> Self {
    Self { percent: 0., px: 0. }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn text(v: &LengthValue) -> String {
    serde_json::to_value(v).unwrap().as_str().unwrap().to_string()
  }

  #[test]
  fn single_values_read_and_size_as_before() {
    // every share and pixel amount the packs use, and generated ones
    for n in (0..=1000).step_by(7).map(|n| n as f32).chain([0.5, 12.25, 1920.]) {
      let px: LengthValue = format!("{n}px").parse().unwrap();
      let bare: LengthValue = format!("{n}").parse().unwrap();
      let pc: LengthValue = format!("{n}%").parse().unwrap();
      assert_eq!(px, bare);
      for total in [0, 1, 1080, 1920] {
        assert_eq!(px.to_px(total), n as i32);
        assert_eq!(px.to_px_scaled(total, 1.25), (1.25 * n) as i32);
        assert_eq!(pc.to_px(total), (n / 100. * total as f32) as i32);
        assert_eq!(pc.to_px_scaled(total, 1.25), (n / 100. * total as f32) as i32);
      }
      assert_eq!(text(&px), format!("{n}px"));
      // (0 % and 0 px are the same length, written 0px)
      if n != 0. {
        assert_eq!(text(&pc), format!("{n}%"));
      }
    }
  }

  #[test]
  fn sums_of_shares_and_pixels() {
    let below_bar: LengthValue = "100% - 40px".parse().unwrap();
    assert_eq!(below_bar, LengthValue { percent: 100., px: -40. });
    assert_eq!(below_bar.to_px_scaled(1080, 1.5), 1080 - 60);
    // generated: every sign and spacing of two and three terms reads back to itself
    for (a, b, c) in [(100., 40., 0.), (50., 12.5, 8.), (0.5, 1., 2.)] {
      for (sb, sc) in [('+', '+'), ('+', '-'), ('-', '+'), ('-', '-')] {
        for s in [format!("{a}% {sb} {b}px {sc} {c}px"), format!("{a}%{sb}{b}px{sc}{c}")] {
          let v: LengthValue = s.parse().unwrap();
          let px = if sb == '-' { -b } else { b } + if sc == '-' { -c } else { c };
          assert_eq!(v, LengthValue { percent: a, px }, "{s}");
          let again: LengthValue = text(&v).parse().unwrap();
          assert_eq!(again, v, "{s}");
        }
      }
    }
  }

  #[test]
  fn other_text_is_refused() {
    for bad in ["", "px", "%", "10em", "100%40px", "100% -", "100% - abc", "- 40px", "1.2.3px", "10px%", "NaN", "inf%"] {
      assert!(bad.parse::<LengthValue>().is_err(), "{bad:?} was accepted");
    }
  }
}
