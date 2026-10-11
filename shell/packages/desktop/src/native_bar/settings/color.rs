//! The colour picker's arithmetic: `#rrggbb` <-> RGB <-> HSV (hue in
//! degrees, saturation and value 0..1). No Windows here: it is tested on
//! its own.

/// A colour as the picker holds it: the hue stays where it was when the
/// colour is grey (no hue) or black (no saturation either).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
  pub h: f32,
  pub s: f32,
  pub v: f32,
}

/// `#rrggbb` or `rrggbb` (any case); None for anything else.
pub fn parse_hex(text: &str) -> Option<(u8, u8, u8)> {
  let h = text.trim();
  let h = h.strip_prefix('#').unwrap_or(h);
  if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
    return None;
  }
  let v = u32::from_str_radix(h, 16).ok()?;
  Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// `#rrggbb`, lower case (what the core stores).
pub fn to_hex((r, g, b): (u8, u8, u8)) -> String {
  format!("#{r:02x}{g:02x}{b:02x}")
}

pub fn hsv_to_rgb(c: Hsv) -> (u8, u8, u8) {
  let h = c.h.rem_euclid(360.0) / 60.0;
  let s = c.s.clamp(0.0, 1.0);
  let v = c.v.clamp(0.0, 1.0);
  let chroma = v * s;
  let x = chroma * (1.0 - (h % 2.0 - 1.0).abs());
  let (r, g, b) = match h as u32 {
    0 => (chroma, x, 0.0),
    1 => (x, chroma, 0.0),
    2 => (0.0, chroma, x),
    3 => (0.0, x, chroma),
    4 => (x, 0.0, chroma),
    _ => (chroma, 0.0, x),
  };
  let m = v - chroma;
  let to = |k: f32| ((k + m) * 255.0).round().clamp(0.0, 255.0) as u8;
  (to(r), to(g), to(b))
}

/// `hue` is kept when the colour has none (a grey).
pub fn rgb_to_hsv((r, g, b): (u8, u8, u8), hue: f32) -> Hsv {
  let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
  let max = r.max(g).max(b);
  let min = r.min(g).min(b);
  let d = max - min;
  let h = if d <= 0.0 {
    hue
  } else if max == r {
    60.0 * ((g - b) / d).rem_euclid(6.0)
  } else if max == g {
    60.0 * ((b - r) / d + 2.0)
  } else {
    60.0 * ((r - g) / d + 4.0)
  };
  let s = if max <= 0.0 { 0.0 } else { d / max };
  Hsv { h, s, v: max }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hex_is_six_digits_with_or_without_the_hash() {
    assert_eq!(parse_hex("#b69df8"), Some((0xb6, 0x9d, 0xf8)));
    assert_eq!(parse_hex("B69DF8"), Some((0xb6, 0x9d, 0xf8)));
    assert_eq!(parse_hex(" #00ff00 "), Some((0, 255, 0)));
    for bad in ["", "#", "#abc", "#b69df", "#b69df8a", "#gggggg", "b69df8#"] {
      assert_eq!(parse_hex(bad), None, "{bad}");
    }
    assert_eq!(to_hex((0xb6, 0x9d, 0xf8)), "#b69df8");
    assert_eq!(to_hex((0, 0, 0)), "#000000");
  }

  #[test]
  fn primaries_and_greys() {
    assert_eq!(hsv_to_rgb(Hsv { h: 0.0, s: 1.0, v: 1.0 }), (255, 0, 0));
    assert_eq!(hsv_to_rgb(Hsv { h: 120.0, s: 1.0, v: 1.0 }), (0, 255, 0));
    assert_eq!(hsv_to_rgb(Hsv { h: 240.0, s: 1.0, v: 1.0 }), (0, 0, 255));
    assert_eq!(hsv_to_rgb(Hsv { h: 360.0, s: 1.0, v: 1.0 }), (255, 0, 0));
    assert_eq!(hsv_to_rgb(Hsv { h: 200.0, s: 0.0, v: 0.5 }), (128, 128, 128));
    let red = rgb_to_hsv((255, 0, 0), 0.0);
    assert_eq!((red.h, red.s, red.v), (0.0, 1.0, 1.0));
    // a grey has no hue: the one the picker had stays
    let grey = rgb_to_hsv((80, 80, 80), 210.0);
    assert_eq!((grey.h, grey.s), (210.0, 0.0));
    assert_eq!(rgb_to_hsv((0, 0, 0), 33.0).h, 33.0);
  }

  #[test]
  fn every_colour_survives_hsv_and_back() {
    for r in (0..=255u32).step_by(3) {
      for g in (0..=255u32).step_by(5) {
        for b in (0..=255u32).step_by(7) {
          let c = (r as u8, g as u8, b as u8);
          let hsv = rgb_to_hsv(c, 0.0);
          assert!((0.0..360.0).contains(&hsv.h), "{c:?} -> {hsv:?}");
          assert_eq!(hsv_to_rgb(hsv), c, "{c:?} -> {hsv:?}");
          assert_eq!(parse_hex(&to_hex(hsv_to_rgb(hsv))), Some(c));
        }
      }
    }
  }
}
