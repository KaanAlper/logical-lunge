//! Embeds the application manifest. A layered window can only be a child
//! window (ours sits under the desktop icons as a child of Explorer's
//! desktop window) in a program that declares Windows 8 or later; without
//! the manifest Windows never shows it.

fn main() {
  let manifest = std::path::Path::new(
    &std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default(),
  )
  .join("lunge-wallpaper.manifest");
  println!("cargo:rerun-if-changed={}", manifest.display());
  let windows =
    std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
  let msvc =
    std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
  if windows && msvc {
    // the linker embeds it (no resource compiler needed)
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
      "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
      manifest.display()
    );
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
  }
}
