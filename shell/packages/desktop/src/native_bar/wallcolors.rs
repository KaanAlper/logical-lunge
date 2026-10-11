//! Colours from the wallpaper (prefs.json "wallpaperColors", Settings >
//! Görünüm; off by default, when the accent colour stays the reference).
//!
//! illogical-impulse's pipeline: the wallpaper is scaled to 128 px, its
//! colours quantized and scored for the source colour (Material Color
//! Utilities, as matugen does), and a Material 3 scheme built from it:
//! tonal spot, or neutral for a source of low chroma (a grey or washed-out
//! picture). The shell's surfaces and the window borders take its colours.

use material_colors::{color::Rgb, dynamic_color::Variant, hct::Hct, image::extract_color, scheme::Scheme, theme::ThemeBuilder};

use super::{gfx::Rgba, view::Theme};

/// Below this chroma the source makes a neutral scheme (ii's rule).
const NEUTRAL_CHROMA: f64 = 40.0;

/// The current wallpaper's file (Windows' own setting); None for none or a
/// solid colour. Reads the disk: call it off the UI thread.
#[cfg(windows)]
fn wallpaper() -> Option<String> {
  use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
  let mut buf = [0u16; 1024];
  unsafe {
    SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, Some(buf.as_mut_ptr().cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).ok()?;
  }
  let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
  let path = String::from_utf16_lossy(&buf[..end]);
  (!path.is_empty() && std::path::Path::new(&path).exists()).then_some(path)
}

#[cfg(not(windows))]
fn wallpaper() -> Option<String> {
  None
}

/// Decoded pixels (premultiplied BGRA, rows of `w * 4` bytes).
struct Pixels {
  data: Vec<u8>,
}

/// The picture's first frame scaled to fit `max_w` x `max_h` (WIC).
#[cfg(windows)]
fn load(path: &str, max_w: u32, max_h: u32) -> Option<Pixels> {
  use windows::{
    core::{HSTRING, Interface},
    Win32::{
      Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory, WICBitmapDitherTypeNone,
        WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut, WICDecodeMetadataCacheOnDemand,
      },
      System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
    },
  };
  unsafe {
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let dec = wic
      .CreateDecoderFromFilename(&HSTRING::from(path), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand)
      .ok()?;
    let frame = dec.GetFrame(0).ok()?;
    let (mut w, mut h) = (0u32, 0u32);
    frame.GetSize(&mut w, &mut h).ok()?;
    let k = (max_w as f32 / w.max(1) as f32).min(max_h as f32 / h.max(1) as f32).min(1.0);
    let (tw, th) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
    let scaler = wic.CreateBitmapScaler().ok()?;
    scaler.Initialize(&frame, tw, th, WICBitmapInterpolationModeFant).ok()?;
    let conv = wic.CreateFormatConverter().ok()?;
    conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
    let src: IWICBitmapSource = conv.cast().ok()?;
    let mut data = vec![0u8; (tw * th * 4) as usize];
    src.CopyPixels(std::ptr::null(), tw * 4, &mut data).ok()?;
    Some(Pixels { data })
  }
}

#[cfg(not(windows))]
fn load(_path: &str, _max_w: u32, _max_h: u32) -> Option<Pixels> {
  None
}

/// The wallpaper's source colour; None when there is no picture to read.
/// Decodes and quantizes a 128 px copy: call it off the UI thread.
pub(super) fn seed() -> Option<[u8; 3]> {
  let path = wallpaper()?;
  let px = load(&path, 128, 128)?;
  // premultiplied BGRA; a wallpaper is opaque, so the colours are as they are
  let pixels: Vec<Rgb> = px.data.chunks_exact(4).filter(|p| p[3] > 0).map(|p| Rgb::new(p[2], p[1], p[0])).collect();
  if pixels.is_empty() {
    return None;
  }
  let c = extract_color(&pixels);
  Some([c.red, c.green, c.blue])
}

fn scheme(seed: [u8; 3], light: bool) -> Scheme {
  let source = Rgb::new(seed[0], seed[1], seed[2]);
  let variant = if Hct::new(source).get_chroma() < NEUTRAL_CHROMA { Variant::Neutral } else { Variant::TonalSpot };
  let theme = ThemeBuilder::with_source(source).variant(variant).build();
  if light {
    theme.schemes.light
  } else {
    theme.schemes.dark
  }
}

fn rgba(c: Rgb, a: f32) -> Rgba {
  Rgba(c.red, c.green, c.blue, a)
}

/// The shell's theme from the wallpaper's scheme (the same roles as the
/// built-in purple one, view::DARK / view::LIGHT).
pub(super) fn theme(seed: [u8; 3], light: bool) -> Theme {
  let s = scheme(seed, light);
  Theme {
    primary: rgba(s.primary, 1.0),
    on_primary: rgba(s.on_primary, 1.0),
    sec_container: rgba(s.secondary_container, 1.0),
    on_sec_container: rgba(s.on_secondary_container, 1.0),
    error: rgba(s.error, 1.0),
    layer0: rgba(s.surface, 1.0),
    layer1: rgba(if light { s.surface_container } else { s.surface_container_low }, 1.0),
    layer1_hover: rgba(s.surface_container_highest, 1.0),
    on_layer0: rgba(s.on_surface, 1.0),
    on_layer1: rgba(s.on_surface, 1.0),
    subtext: rgba(s.outline, 1.0),
    inactive: rgba(s.outline, 1.0),
    occupied: rgba(s.secondary_container, if light { 0.9 } else { 0.6 }),
    tip_bg: rgba(s.inverse_surface, 1.0),
    tip_fg: rgba(s.inverse_on_surface, 1.0),
    border: rgba(s.outline_variant, if light { 0.35 } else { 0.6 }),
    primary_container: rgba(s.primary_container, 1.0),
    on_primary_container: rgba(s.on_primary_container, 1.0),
    surface_container: rgba(s.surface_container, 1.0),
    surface_container_high: rgba(s.surface_container_high, 1.0),
    outline_variant: rgba(s.outline_variant, 1.0),
    on_surface_variant: rgba(s.on_surface_variant, 1.0),
  }
}

/// The window borders' colours (#rrggbbaa, active and inactive) for the
/// border style: ii's outline variant at 47 % and surface container low at
/// 20 %, or the accent style's primary at 80 % over its usual inactive grey.
pub(super) fn border(seed: [u8; 3], light: bool, ii: bool) -> (String, String) {
  let s = scheme(seed, light);
  let hex = |c: Rgb, a: u8| format!("#{:02x}{:02x}{:02x}{:02x}", c.red, c.green, c.blue, a);
  if ii {
    (hex(s.outline_variant, 0x77), hex(s.surface_container_low, 0x33))
  } else {
    (hex(s.primary, 0xcc), "#3a3a4099".to_string())
  }
}

/// The web panels' colours (ui/lib/theme.mjs) from the wallpaper's scheme:
/// the CSS variables of ui/theme-tokens.json and the text colours, for the
/// dark and the light theme, in the roles of illogical-impulse's
/// Appearance.qml (layer 0 the surface, layer 1 the low container, ...).
pub(super) fn web_palette(seed: [u8; 3]) -> serde_json::Value {
  let hex = |c: Rgb| format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue);
  // ii's ColorUtils.mix: `a` by `amount`, the rest `b`
  let mix = |a: Rgb, b: Rgb, amount: f64| {
    let m = |x: u8, y: u8| (x as f64 * amount + y as f64 * (1.0 - amount)).round() as u8;
    Rgb::new(m(a.red, b.red), m(a.green, b.green), m(a.blue, b.blue))
  };
  let mode = |light: bool| {
    let s = scheme(seed, light);
    let layer1 = if light { s.surface_container } else { s.surface_container_low };
    let layer1_hover = if light { s.surface_container_highest } else { s.surface_container_high };
    serde_json::json!({
      "m3primary": hex(s.primary),
      "m3onPrimary": hex(s.on_primary),
      "m3primaryContainer": hex(s.primary_container),
      "m3onPrimaryContainer": hex(s.on_primary_container),
      "m3secondaryContainer": hex(s.secondary_container),
      "m3onSecondaryContainer": hex(s.on_secondary_container),
      "m3onSurface": hex(s.on_surface),
      "m3onSurfaceVariant": hex(s.on_surface_variant),
      "m3outline": hex(s.outline),
      "m3outlineVariant": hex(s.outline_variant),
      "colLayer0": hex(s.surface),
      "colLayer0Border": hex(mix(s.outline_variant, s.surface, 0.45)),
      "colLayer1": hex(layer1),
      "colLayer1Hover": hex(layer1_hover),
      "barLayer1Hover": hex(s.surface_container_highest),
      "colLayer2": hex(s.surface_container_high),
      "colLayer2Hover": hex(mix(s.on_surface, s.surface_container_high, 0.06)),
      "colLayer3": hex(s.surface_container_highest),
      "colBackgroundSurfaceContainer": hex(s.surface_container),
      "colSurfaceContainerHigh": hex(s.surface_container_high),
      "colOnLayer0": hex(s.on_surface),
      "colOnLayer1": hex(s.on_surface),
      "colSubtext": hex(s.outline),
    })
  };
  serde_json::json!({ "dark": mode(false), "light": mode(true) })
}

/// The web panels' wallpaper colours in use (web_palette), with the source
/// colour they came from; None while the accent colour is in use.
static WEB: std::sync::Mutex<Option<([u8; 3], serde_json::Value)>> = std::sync::Mutex::new(None);

/// The web panels' colours for the source colour in use (None: the accent
/// colour). True when they changed: the panels are told (`ll:wall-palette`).
pub(super) fn web_publish(seed: Option<[u8; 3]>) -> bool {
  let mut web = WEB.lock().unwrap_or_else(|e| e.into_inner());
  if web.as_ref().map(|(s, _)| *s) == seed {
    return false;
  }
  *web = seed.map(|s| (s, web_palette(s)));
  true
}

/// The web panels' wallpaper colours (`wall_palette` command); None while
/// the accent colour is in use.
pub(crate) fn web_current() -> Option<serde_json::Value> {
  WEB.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, p)| p.clone())
}

#[cfg(test)]
mod tests {
  use super::{border, scheme, theme, web_palette};

  #[test]
  fn a_grey_wallpaper_makes_a_neutral_scheme() {
    let grey = scheme([0x80, 0x80, 0x84], false);
    let vivid = scheme([0x20, 0x60, 0xe0], false);
    let chroma = |c: material_colors::color::Rgb| material_colors::hct::Hct::new(c).get_chroma();
    assert!(chroma(grey.primary) < chroma(vivid.primary));
  }

  #[test]
  fn the_purple_reference_stays_close_to_the_built_in_theme() {
    let t = theme([0xb6, 0x9d, 0xf8], false);
    // the background is the near-black of the built-in dark theme
    assert!(t.layer0.0 < 0x30 && t.layer0.1 < 0x30 && t.layer0.2 < 0x30);
    let (active, inactive) = border([0xb6, 0x9d, 0xf8], false, true);
    assert!(active.ends_with("77") && inactive.ends_with("33") && active.len() == 9);
  }

  #[test]
  fn the_web_panels_get_every_theme_token_in_both_modes() {
    let tokens: serde_json::Value = serde_json::from_str(include_str!("../../../../../ui/theme-tokens.json")).unwrap();
    let p = web_palette([0xb6, 0x9d, 0xf8]);
    for mode in ["dark", "light"] {
      for key in tokens[mode].as_object().unwrap().keys() {
        let v = p[mode][key].as_str().unwrap_or_default();
        assert!(v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit()), "{mode} {key}: {v:?}");
      }
    }
    // the purple reference: ii's near-black and near-white backgrounds
    assert!(p["dark"]["colLayer0"].as_str().unwrap() < "#303030");
    assert!(p["light"]["colLayer0"].as_str().unwrap() > "#f0f0f0");
  }
}
