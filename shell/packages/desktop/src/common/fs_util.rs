use std::{fs, path::Path};

use anyhow::Context;
use serde::de::DeserializeOwned;

/// Reads a JSON file and parses it into the specified type.
///
/// Returns the parsed type `T` if successful.
pub fn read_and_parse_json<T: DeserializeOwned>(
  path: &Path,
) -> anyhow::Result<T> {
  let content = fs::read_to_string(path)
    .with_context(|| format!("Failed to read file: {}", path.display()))?;

  // Notepad and Windows PowerShell 5.1 (`Set-Content -Encoding UTF8`) write
  // a UTF-8 BOM, which serde_json rejects.
  serde_json::from_str(content.trim_start_matches('\u{feff}')).with_context(|| {
    format!("Failed to parse JSON from file: {}", path.display())
  })
}

/// Returns whether the path has the given extension.
pub fn has_extension(path: &Path, extension: &str) -> bool {
  path
    .file_name()
    .and_then(|name| name.to_str())
    .map(|name| name.ends_with(extension))
    .unwrap_or(false)
}
