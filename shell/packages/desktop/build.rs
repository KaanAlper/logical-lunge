//! Embeds the shell's icon, application manifest and version: Windows 8+
//! compatibility (layered child windows), per-monitor DPI v2, common
//! controls 6 for the shell dialogs we open, long paths. The version comes
//! from VERSION_NUMBER (build.ps1 sets it to the release's version).

use std::{env, fs, path::PathBuf};

fn main() {
  let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
  let manifest = dir.join("lunge-shell.manifest");
  let icon = dir.join("resources").join("icons").join("icon.ico");
  println!("cargo:rerun-if-changed={}", manifest.display());
  println!("cargo:rerun-if-changed={}", icon.display());
  println!("cargo:rerun-if-env-changed=VERSION_NUMBER");

  let windows_target = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
  // the resource compiler is the Windows SDK's rc.exe: checks from another
  // OS (cargo check for the Windows target) build no resources
  if !(windows_target && cfg!(windows)) {
    return;
  }
  let version = env::var("VERSION_NUMBER").unwrap_or_else(|_| "0.0.0".into());
  let mut parts: Vec<u16> = version.split('.').filter_map(|p| p.parse().ok()).collect();
  parts.resize(4, 0);
  let numbers = parts.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
  let text = parts.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(".");
  let quote = |p: &PathBuf| p.display().to_string().replace('\\', "\\\\");
  let rc = format!(
    r#"1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Logical Lunge"
      VALUE "FileDescription", "Logical Lunge shell"
      VALUE "FileVersion", "{text}"
      VALUE "InternalName", "lunge-shell"
      VALUE "OriginalFilename", "lunge-shell.exe"
      VALUE "ProductName", "Logical Lunge"
      VALUE "ProductVersion", "{text}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#,
    icon = quote(&icon),
    manifest = quote(&manifest),
  );
  let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("lunge-shell.rc");
  fs::write(&out, rc).expect("lunge-shell.rc");
  embed_resource::compile(&out, embed_resource::NONE)
    .manifest_required()
    .expect("lunge-shell.rc");
}
