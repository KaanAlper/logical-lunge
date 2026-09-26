//! The UI fonts, straight from `ui/fonts` (the WOFF2 files the web widgets
//! use), so the native bar looks the same.
//!
//! `fonts.css` splits each family into unicode subsets (latin, latin-ext,
//! cyrillic...), like Google Fonts does. DirectWrite does not merge the
//! coverage of several files of one family, so every file becomes its own
//! collection and a custom font fallback sends each unicode range to the file
//! that covers it -- the same thing `unicode-range` does in the browser. The
//! system fallback comes last.

use std::{collections::HashMap, path::Path};

use anyhow::Context;
use windows::{
  core::{Interface, HSTRING, PCWSTR},
  Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteFactory6, IDWriteFontCollection, IDWriteFontCollection2,
    IDWriteFontFallback, IDWriteFontSetBuilder1, IDWriteTextFormat, IDWriteTextFormat1,
    DWRITE_CONTAINER_TYPE_WOFF2, DWRITE_FONT_AXIS_TAG, DWRITE_FONT_AXIS_VALUE,
    DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_UNICODE_RANGE, DWRITE_WORD_WRAPPING_NO_WRAP,
  },
};

pub const TEXT_FAMILY: &str = "Google Sans Flex";
pub const ICON_FAMILY: &str = "Material Symbols Rounded";

struct Face {
  family: String,
  file: String,
  ranges: Vec<DWRITE_UNICODE_RANGE>,
}

pub struct Fonts {
  dwrite: IDWriteFactory6,
  text: IDWriteFontCollection2,
  icons: IDWriteFontCollection2,
  fallback: IDWriteFontFallback,
  formats: HashMap<(bool, u32, u32, bool), IDWriteTextFormat>,
}

/// Text style: size in DIPs, weight (Google Sans Flex is variable 300-700).
#[derive(Clone, Copy)]
pub struct TextStyle {
  pub size: f32,
  pub weight: f32,
}

impl Fonts {
  pub fn load(dwrite: &IDWriteFactory6, ui_dir: &Path) -> anyhow::Result<Self> {
    let css = std::fs::read_to_string(ui_dir.join("fonts.css"))
      .context("fonts.css")?;
    let faces = parse_css(&css);
    anyhow::ensure!(!faces.is_empty(), "no @font-face in fonts.css");

    unsafe {
      let loader = dwrite.CreateInMemoryFontFileLoader()?;
      dwrite.RegisterFontFileLoader(&loader)?;
      let factory: IDWriteFactory = dwrite.cast()?;

      // family -> [(collection, ranges)] in fonts.css order
      let mut by_family: Vec<(String, IDWriteFontCollection2, Vec<DWRITE_UNICODE_RANGE>)> =
        Vec::new();
      // Rubik has one file per weight and subset: files of one subset share a collection
      let mut grouped: HashMap<(String, String), IDWriteFontSetBuilder1> = HashMap::new();
      let mut order: Vec<(String, String, Vec<DWRITE_UNICODE_RANGE>)> = Vec::new();

      for face in &faces {
        let path = ui_dir.join(face.file.trim_start_matches("./"));
        let data = match std::fs::read(&path) {
          Ok(d) => d,
          Err(err) => {
            tracing::warn!("Native bar: font {:?}: {}", path, err);
            continue;
          }
        };
        let stream = dwrite.UnpackFontFile(
          DWRITE_CONTAINER_TYPE_WOFF2,
          data.as_ptr() as *const _,
          data.len() as u32,
        )?;
        let size = stream.GetFileSize()?;
        let mut ptr = std::ptr::null_mut();
        let mut ctx = std::ptr::null_mut();
        stream.ReadFileFragment(&mut ptr, 0, size, &mut ctx)?;
        let bytes =
          std::slice::from_raw_parts(ptr as *const u8, size as usize).to_vec();
        stream.ReleaseFileFragment(ctx);
        // no owner object: DirectWrite keeps its own copy of the data
        let file = loader.CreateInMemoryFontFileReference(
          &factory,
          bytes.as_ptr() as *const _,
          bytes.len() as u32,
          None,
        )?;

        // subset key: the file name without the weight part
        let subset = subset_of(&face.file);
        let key = (face.family.clone(), subset.clone());
        if !grouped.contains_key(&key) {
          let builder: IDWriteFontSetBuilder1 = dwrite.CreateFontSetBuilder()?.cast()?;
          grouped.insert(key.clone(), builder);
          order.push((face.family.clone(), subset, face.ranges.clone()));
        }
        grouped[&key].AddFontFile(&file)?;
      }

      for (family, subset, ranges) in order {
        let builder = &grouped[&(family.clone(), subset)];
        let set = builder.CreateFontSet()?;
        let collection =
          dwrite.CreateFontCollectionFromFontSet(&set, DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC)?;
        by_family.push((family, collection, ranges));
      }

      let find = |family: &str, latin: bool| {
        by_family
          .iter()
          .filter(|(f, _, _)| f == family)
          .find(|(_, _, r)| !latin || r.iter().any(|r| r.first <= 0x41 && r.last >= 0x7A))
          .or_else(|| by_family.iter().find(|(f, _, _)| f == family))
          .map(|(_, c, _)| c.clone())
      };
      let text = find(TEXT_FAMILY, true).context("Google Sans Flex missing")?;
      let icons = find(ICON_FAMILY, false).context("Material Symbols missing")?;

      let builder = dwrite.CreateFontFallbackBuilder()?;
      for (family, collection, ranges) in &by_family {
        if ranges.is_empty() {
          continue;
        }
        let name = HSTRING::from(family.as_str());
        let names = [name.as_ptr()];
        let base: IDWriteFontCollection = collection.cast()?;
        builder.AddMapping(ranges, &names, &base, PCWSTR::null(), PCWSTR::null(), 1.0)?;
      }
      builder.AddMappings(&dwrite.GetSystemFontFallback()?)?;
      let fallback = builder.CreateFontFallback()?;

      Ok(Self { dwrite: dwrite.clone(), text, icons, fallback, formats: HashMap::new() })
    }
  }

  /// Text format for UI text (single line, vertically centred, ellipsis).
  pub fn text(&mut self, style: TextStyle) -> anyhow::Result<IDWriteTextFormat> {
    let axes = [axis(b"wght", style.weight)];
    self.format(false, style.size, style.weight, false, &axes)
  }

  /// Text format for Material Symbols icons (`fill` = ii's filled variant).
  pub fn icon(&mut self, size: f32, fill: bool) -> anyhow::Result<IDWriteTextFormat> {
    let weight = if fill { 600.0 } else { 400.0 };
    let axes = [
      axis(b"FILL", if fill { 1.0 } else { 0.0 }),
      axis(b"wght", weight),
      axis(b"GRAD", 0.0),
      axis(b"opsz", 20.0),
    ];
    self.format(true, size, weight, fill, &axes)
  }

  fn format(
    &mut self,
    icon: bool,
    size: f32,
    weight: f32,
    fill: bool,
    axes: &[DWRITE_FONT_AXIS_VALUE],
  ) -> anyhow::Result<IDWriteTextFormat> {
    let key = (icon, (size * 10.0) as u32, weight as u32, fill);
    if let Some(f) = self.formats.get(&key) {
      return Ok(f.clone());
    }
    unsafe {
      let (family, collection) = if icon {
        (ICON_FAMILY, &self.icons)
      } else {
        (TEXT_FAMILY, &self.text)
      };
      let collection: IDWriteFontCollection = collection.cast()?;
      let f3 = self.dwrite.CreateTextFormat(
        &HSTRING::from(family),
        &collection,
        axes,
        size,
        &HSTRING::from("en-us"),
      )?;
      let f1: IDWriteTextFormat1 = f3.cast()?;
      f1.SetFontFallback(&self.fallback)?;
      f1.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
      f1.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
      f1.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
      if !icon {
        let sign = self.dwrite.CreateEllipsisTrimmingSign(&f1)?;
        let trimming = DWRITE_TRIMMING {
          granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
          delimiter: 0,
          delimiterCount: 0,
        };
        f1.SetTrimming(&trimming, &sign)?;
      }
      let f: IDWriteTextFormat = f1.cast()?;
      self.formats.insert(key, f.clone());
      Ok(f)
    }
  }
}

fn axis(tag: &[u8; 4], value: f32) -> DWRITE_FONT_AXIS_VALUE {
  DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG(u32::from_le_bytes(*tag)), value }
}

/// `google-sans-flex-300-700-latin-ext.woff2` -> `latin-ext`, `rubik-500-cyrillic.woff2` -> `cyrillic`
fn subset_of(file: &str) -> String {
  let name = file.rsplit('/').next().unwrap_or(file).trim_end_matches(".woff2");
  let parts: Vec<&str> = name.split('-').collect();
  match parts.iter().rposition(|p| p.chars().all(|c| c.is_ascii_digit())) {
    Some(i) if i + 1 < parts.len() => parts[i + 1..].join("-"),
    _ => name.to_string(),
  }
}

fn parse_css(css: &str) -> Vec<Face> {
  let mut faces = Vec::new();
  for block in css.split("@font-face").skip(1) {
    let body = match (block.find('{'), block.find('}')) {
      (Some(a), Some(b)) if b > a => &block[a + 1..b],
      _ => continue,
    };
    let prop = |name: &str| {
      body.split(';').find_map(|decl| {
        let (k, v) = decl.split_once(':')?;
        (k.trim() == name).then(|| v.trim().to_string())
      })
    };
    let family = match prop("font-family") {
      Some(f) => f.trim_matches(|c| c == '\'' || c == '"').to_string(),
      None => continue,
    };
    let file = match prop("src").and_then(|s| {
      let a = s.find("url(")? + 4;
      let b = s[a..].find(')')? + a;
      Some(s[a..b].trim_matches(|c| c == '\'' || c == '"').to_string())
    }) {
      Some(f) => f,
      None => continue,
    };
    let ranges = prop("unicode-range").map(|r| parse_ranges(&r)).unwrap_or_default();
    faces.push(Face { family, file, ranges });
  }
  faces
}

fn parse_ranges(s: &str) -> Vec<DWRITE_UNICODE_RANGE> {
  s.split(',')
    .filter_map(|part| {
      let p = part.trim().trim_start_matches("U+").trim_start_matches("u+");
      if p.is_empty() {
        return None;
      }
      let (a, b) = match p.split_once('-') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        // U+4?? wildcards
        None if p.contains('?') => (p.replace('?', "0"), p.replace('?', "F")),
        None => (p.to_string(), p.to_string()),
      };
      Some(DWRITE_UNICODE_RANGE {
        first: u32::from_str_radix(&a, 16).ok()?,
        last: u32::from_str_radix(&b, 16).ok()?,
      })
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn subsets() {
    assert_eq!(subset_of("./fonts/google-sans-flex-300-700-latin-ext.woff2"), "latin-ext");
    assert_eq!(subset_of("./fonts/rubik-500-cyrillic.woff2"), "cyrillic");
    assert_eq!(subset_of("./fonts/material-symbols-rounded-100-700-fallback.woff2"), "fallback");
  }

  #[test]
  fn ranges() {
    let r = parse_ranges("U+02C7, U+02D8-02D9, U+4??");
    assert_eq!(r.len(), 3);
    assert_eq!((r[1].first, r[1].last), (0x2D8, 0x2D9));
    assert_eq!((r[2].first, r[2].last), (0x400, 0x4FF));
  }
}
