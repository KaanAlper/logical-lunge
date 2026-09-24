use std::{
  fs::{self},
  path::{Path, PathBuf},
  sync::Arc,
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tauri::{path::BaseDirectory, AppHandle, Manager};
use tokio::sync::Mutex;

use crate::{
  common::{read_and_parse_json, PathExt},
  config_migration::apply_config_migrations,
};

pub const VERSION_NUMBER: &str = env!("VERSION_NUMBER");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsValue {
  /// JSON schema URL to validate the settings file.
  #[serde(rename = "$schema")]
  pub schema: Option<String>,

  /// Widget configs to be launched on startup.
  pub startup_configs: Vec<StartupConfig>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupConfig {
  /// ID of the widget pack to launch on startup.
  pub pack: String,

  /// Name of the widget within the widget pack to launch on startup.
  pub widget: String,

  /// Preset name within the widget config.
  pub preset: String,
}

#[derive(Debug)]
pub struct AppSettings {
  /// Directory where config files are stored.
  pub config_dir: PathBuf,

  /// Directory where webview cache files are stored.
  pub webview_cache_dir: PathBuf,

  /// Parsed app settings value.
  pub value: Arc<Mutex<AppSettingsValue>>,
}

impl AppSettings {
  /// Creates a new `AppSettings` instance.
  pub fn new(
    app_handle: &AppHandle,
    config_dir: PathBuf,
  ) -> anyhow::Result<Self> {
    let webview_cache_dir = app_handle
      .path()
      .resolve("zebar/webview-cache", BaseDirectory::Data)
      .context("Unable to resolve app data directory.")?;

    let migration_file = app_handle
      .path()
      .resolve("zebar/.migrations.json", BaseDirectory::Data)
      .context("Unable to resolve config migration file.")?;

    for dir in [&config_dir, &webview_cache_dir] {
      fs::create_dir_all(dir)?;
    }

    let settings = Self::read_settings_or_init(&config_dir, &migration_file);

    Ok(Self {
      config_dir: config_dir.canonicalize_pretty()?,
      webview_cache_dir: webview_cache_dir.canonicalize_pretty()?,
      value: Arc::new(Mutex::new(settings)),
    })
  }

  /// Reads the app settings file or initializes it with the default.
  ///
  /// Logical Lunge: never fails. A settings file that can't be read or
  /// parsed (e.g. edited by hand) is left untouched and the shell's
  /// default widgets start instead; otherwise the bar would never come
  /// back and the watchdog would restart Zebar in a loop.
  fn read_settings_or_init(
    config_dir: &Path,
    migration_file: &Path,
  ) -> AppSettingsValue {
    // Apply any pending config migrations before reading the settings
    // file.
    if let Err(err) = apply_config_migrations(config_dir, migration_file) {
      tracing::warn!("Failed to apply config migrations: {:?}", err);
    }

    let settings_path = config_dir.join("settings.json");

    // If the file does not exist, initialize a default.
    if !settings_path.exists() {
      if let Err(err) = Self::write_default(&settings_path) {
        tracing::warn!("Failed to write default settings: {:?}", err);
      }
    }

    read_and_parse_json(&settings_path).unwrap_or_else(|err| {
      tracing::error!(
        "Invalid settings file, starting the default widgets: {:?}",
        err
      );
      Self::default_value()
    })
  }

  /// Default settings: the Logical Lunge shell's widgets (the same list
  /// the installer writes).
  fn default_value() -> AppSettingsValue {
    AppSettingsValue {
      schema: None,
      startup_configs: [
        "bar",
        "overview",
        "sidebar-right",
        "toast",
        "osk",
        "update",
        "session",
      ]
      .into_iter()
      .map(|widget| StartupConfig {
        pack: "logical-lunge".into(),
        widget: widget.into(),
        preset: "default".into(),
      })
      .collect(),
    }
  }

  /// Writes the default settings to the given settings file path.
  fn write_default(settings_path: &Path) -> anyhow::Result<()> {
    tracing::info!("Initializing app settings from default.");

    fs::write(
      settings_path,
      serde_json::to_string_pretty(&Self::default_value())? + "\n",
    )?;

    Ok(())
  }

  /// Returns the widget configs to open on startup.
  pub async fn startup_configs(&self) -> Vec<StartupConfig> {
    self.value.lock().await.startup_configs.clone()
  }
}
