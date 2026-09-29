//! Accent tokens shared with every web widget; evaluated only when preferences change.
use std::{collections::HashMap, sync::OnceLock};
use super::{gfx::Rgba, view::{Theme, DARK, LIGHT}};

type Tokens = HashMap<String, HashMap<String, (String, f64)>>;
fn rgb(hex: &str) -> Option<[u8; 3]> {
  if hex.len() != 7 || !hex.starts_with('#') { return None; }
  let n = u32::from_str_radix(&hex[1..], 16).ok()?;
  Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}
fn mix(base: [u8; 3], seed: [u8; 3], amount: f64) -> [u8; 3] {
  std::array::from_fn(|i| (base[i] as f64 * (1.0 - amount) + seed[i] as f64 * amount).round() as u8)
}
fn luminance(c: [u8; 3]) -> f64 {
  c.into_iter().zip([0.2126, 0.7152, 0.0722]).map(|(v, weight)| {
    let v = v as f64 / 255.0;
    weight * if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
  }).sum()
}
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
  let (a, b) = (luminance(a), luminance(b));
  (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
pub fn theme(seed: &str, light: bool) -> Theme {
  static TOKENS: OnceLock<Tokens> = OnceLock::new();
  let tokens = TOKENS.get_or_init(|| serde_json::from_str(include_str!("../../../../../ui/theme-tokens.json")).expect("bundled theme tokens"));
  let seed = rgb(seed).unwrap_or([182, 157, 248]);
  let colors: HashMap<_, _> = tokens[if light { "light" } else { "dark" }].iter()
    .map(|(key, (base, amount))| (key.as_str(), mix(rgb(base).unwrap(), seed, *amount))).collect();
  let color = |key: &str| { let c = colors[key]; Rgba(c[0], c[1], c[2], 1.0) };
  let mut t = if light { LIGHT } else { DARK };
  let primary = colors["m3primary"];
  let mut p = primary;
  for i in 1..=100 {
    if contrast(p, colors["colLayer0"]) >= 4.5 { break; }
    p = mix(primary, if light { [0; 3] } else { [255; 3] }, i as f64 / 100.0);
  }
  t.primary = Rgba(p[0], p[1], p[2], 1.0);
  t.on_primary = Rgba::hex(if luminance(p) > 0.179 { 0 } else { 0xffffff });
  t.primary_container = color("m3primaryContainer");
  t.on_primary_container = color("m3onPrimaryContainer");
  t.sec_container = color("m3secondaryContainer");
  t.on_sec_container = color("m3onSecondaryContainer");
  t.layer0 = color("colLayer0"); t.layer1 = color("colLayer1");
  t.layer1_hover = color("colLayer1Hover");
  t.on_layer0 = color("m3onSurface"); t.on_layer1 = color("m3onSurface");
  t.on_surface_variant = color("m3onSurfaceVariant");
  t.subtext = color("m3outline");
  t.surface_container = color("colBackgroundSurfaceContainer");
  t.surface_container_high = color("colSurfaceContainerHigh");
  t.outline_variant = color("m3outlineVariant");
  t.border = color("m3outlineVariant"); t.border.3 = if light { 0.35 } else { 0.6 };
  t.occupied = t.sec_container; t.occupied.3 = if light { 0.9 } else { 0.6 };
  t
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn accent_is_readable_in_both_modes() {
    for seed in ["#b69df8", "#000000", "#ffffff", "#ff0000", "#00ff00", "#0000ff", "invalid"] {
      for light in [false, true] {
        let t = theme(seed, light);
        let c = |v: Rgba| [v.0, v.1, v.2];
        assert!(contrast(c(t.primary), c(t.layer0)) >= 4.5);
        assert!(contrast(c(t.primary), c(t.on_primary)) >= 4.5);
      }
    }
  }
}
