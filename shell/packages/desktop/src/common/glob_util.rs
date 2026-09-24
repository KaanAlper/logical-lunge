use std::path::Path;

use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};

/// Creates a `GlobSet` from a collection of glob patterns.
fn create_glob_set(patterns: &[String]) -> Result<GlobSet> {
  let mut builder = GlobSetBuilder::new();

  for pattern in patterns {
    builder.add(Glob::new(pattern)?);
  }

  Ok(builder.build()?)
}

/// Checks if a path matches any of the provided glob patterns.
///
/// Returns `true` if the path matches any of the patterns, otherwise
/// returns `false`.
pub fn is_match(path: &Path, patterns: &[String]) -> Result<bool> {
  if patterns.is_empty() {
    return Ok(false);
  }

  let glob_set = create_glob_set(patterns)?;
  Ok(glob_set.is_match(path))
}
