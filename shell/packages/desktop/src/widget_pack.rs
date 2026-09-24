use std::{
  collections::HashMap,
  fs::{self},
  path::{Path, PathBuf},
  sync::Arc,
};

use anyhow::Context;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
  app_settings::AppSettings,
  common::{read_and_parse_json, LengthValue, PathExt},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPack {
  /// Unique identifier for the pack.
  pub id: String,

  /// Type of the pack (always a local pack; Logical Lunge has no
  /// marketplace).
  pub r#type: WidgetPackType,

  /// Deserialized pack config.
  #[serde(flatten)]
  pub config: WidgetPackConfig,

  /// Path to the pack config file.
  pub config_path: PathBuf,

  /// Path to the directory containing the pack config.
  ///
  /// This is the parent directory of `config_path`.
  pub directory_path: PathBuf,
}

impl WidgetPack {
  /// Returns a list of file patterns to include for all widgets in the
  /// pack.
  pub fn include_files(&self) -> Vec<String> {
    self
      .config
      .widgets
      .iter()
      .flat_map(|widget| widget.include_files.clone())
      .collect()
  }
}

/// Deserialized widget pack.
///
/// This is the type of the `zpack.json` file.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPackConfig {
  /// JSON schema URL to validate the widget pack file.
  #[serde(rename = "$schema")]
  pub schema: Option<String>,

  /// Name of the pack.
  pub name: String,

  /// Version of the pack.
  pub version: String,

  /// Description of the pack.
  #[serde(default)]
  pub description: String,

  /// Tags of the pack.
  #[serde(default)]
  pub tags: Vec<String>,

  /// Preview images of the pack.
  #[serde(default)]
  pub preview_images: Vec<String>,

  /// URL of the repository containing the pack.
  #[serde(default)]
  pub repository_url: String,

  /// Widgets in the pack.
  #[serde(default)]
  pub widgets: Vec<WidgetConfig>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WidgetPackType {
  Custom,
}

/// Deserialized widget config.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetConfig {
  /// Name of the widget.
  pub name: String,

  /// Relative path to entry point HTML file.
  pub html_path: PathBuf,

  /// Whether to show the Tauri window above/below all others.
  pub z_order: ZOrder,

  /// Whether the Tauri window should be shown in the taskbar.
  pub shown_in_taskbar: bool,

  /// Whether the Tauri window should be focused when opened.
  pub focused: bool,

  /// Whether the Tauri window should have resize handles.
  pub resizable: bool,

  /// Whether the Tauri window frame should be transparent.
  pub transparent: bool,

  /// Files to include as part of the widget.
  #[serde(default)]
  pub include_files: Vec<String>,

  /// How network requests should be cached.
  #[serde(default)]
  pub caching: WidgetCaching,

  /// Privileges for the widget.
  #[serde(default)]
  pub privileges: WidgetPrivileges,

  /// Where to place the widget. Add alias for `defaultPlacements` for
  /// compatibility with v2.3.0 and earlier.
  #[serde(alias = "defaultPlacements")]
  pub presets: Vec<WidgetPreset>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ZOrder {
  BottomMost,
  Normal,
  TopMost,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WidgetCaching {
  /// Default duration to cache network resources for (in seconds).
  pub default_duration: u32,

  /// Custom cache rules.
  pub rules: Vec<WidgetCachingRule>,
}

impl Default for WidgetCaching {
  fn default() -> Self {
    Self {
      default_duration: 604800,
      rules: Vec::new(),
    }
  }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetCachingRule {
  /// URL regex pattern to match.
  pub url_regex: String,

  /// Duration to cache the matched requests for (in seconds).
  pub duration: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPreset {
  #[serde(default = "default_preset_name")]
  pub name: String,

  #[serde(flatten)]
  pub placement: WidgetPlacement,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPlacement {
  /// Anchor-point of the widget.
  pub anchor: AnchorPoint,

  /// Offset from the anchor-point.
  pub offset_x: LengthValue,

  /// Offset from the anchor-point.
  pub offset_y: LengthValue,

  /// Width of the widget in % or physical pixels.
  pub width: LengthValue,

  /// Height of the widget in % or physical pixels.
  pub height: LengthValue,

  /// Monitor(s) to place the widget on.
  pub monitor_selection: MonitorSelection,

  /// How to reserve space for the widget.
  #[serde(default)]
  pub dock_to_edge: DockConfig,
}

#[derive(
  Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum,
)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AnchorPoint {
  TopLeft,
  TopCenter,
  TopRight,
  CenterLeft,
  Center,
  CenterRight,
  BottomLeft,
  BottomCenter,
  BottomRight,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "match", rename_all = "snake_case")]
pub enum MonitorSelection {
  All,
  Primary,
  Secondary,
  Index(usize),
  Name(String),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetPrivileges {
  /// Shell commands that the widget is allowed to run.
  pub shell_commands: Vec<ShellPrivilege>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellPrivilege {
  /// Program name (if in PATH) or full path to the program.
  pub program: String,

  /// Arguments to pass to the program.
  pub args_regex: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockConfig {
  /// Whether to dock the widget to the monitor edge and reserve screen
  /// space for it.
  #[serde(default = "default_bool::<false>")]
  pub enabled: bool,

  /// Edge to dock the widget to.
  pub edge: Option<DockEdge>,

  /// Margin to reserve after the widget window. Can be positive or
  /// negative.
  #[serde(default)]
  pub window_margin: LengthValue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DockEdge {
  Top,
  Bottom,
  Left,
  Right,
}

impl DockEdge {
  pub fn is_horizontal(&self) -> bool {
    matches!(self, Self::Top | Self::Bottom)
  }
}

#[derive(Debug)]
pub struct WidgetPackManager {
  /// Map of widget packs by their ID's.
  pub widget_packs: Arc<Mutex<HashMap<String, WidgetPack>>>,
}

impl WidgetPackManager {
  /// Reads the pack config files within the config directory.
  ///
  /// Returns a new `WidgetPackManager` instance.
  pub fn new(app_settings: Arc<AppSettings>) -> anyhow::Result<Self> {
    let widget_packs = Self::read_widget_packs(&app_settings.config_dir)?;

    Ok(Self {
      widget_packs: Arc::new(Mutex::new(widget_packs)),
    })
  }

  /// Finds all valid widget packs within the user's config directory.
  ///
  /// Widget packs are at the 2nd-level of the config directory
  /// (i.e. `<CONFIG_DIR>/*/zpack.json`).
  ///
  /// Returns a hashmap of widget pack ID's to `WidgetPack` instances.
  fn read_widget_packs(
    config_dir: &Path,
  ) -> anyhow::Result<HashMap<String, WidgetPack>> {
    // Get paths to the subdirectories within the config directory.
    let pack_dirs = fs::read_dir(config_dir)
      .with_context(|| {
        format!(
          "Failed to read config directory: {}",
          config_dir.display()
        )
      })?
      .filter_map(|entry| Some(entry.ok()?.path()))
      .filter(|path| path.is_dir());

    let mut packs = HashMap::new();

    // Parse the found config files.
    for pack_dir in pack_dirs {
      let pack_config_path = pack_dir.join("zpack.json");

      if !pack_config_path.exists() {
        warn!(
          "Skipping subdirectory at '{}' because it has no `zpack.json` file.",
          pack_dir.display()
        );

        continue;
      }

      match Self::read_widget_pack(&pack_config_path) {
        Ok(pack) => {
          tracing::info!(
            "Found valid widget pack at: {}",
            pack_dir.display()
          );

          packs.insert(pack.id.clone(), pack);
        }
        Err(err) => {
          error!("{:?}", err);
        }
      }
    }

    Ok(packs)
  }

  /// Reads a widget pack from a directory. Expects the pack config
  /// file (`zpack.json`) to be present.
  ///
  /// Returns a `WidgetPack` instance.
  pub fn read_widget_pack(config_path: &Path) -> anyhow::Result<WidgetPack> {
    let pack_config = read_and_parse_json::<WidgetPackConfig>(config_path)
      .map_err(|err| {
        anyhow::anyhow!(
          "Failed to parse widget pack at '{}': {:?}",
          config_path.display(),
          err
        )
      })?;

    let pack_dir = config_path.parent().with_context(|| {
      format!(
        "Invalid widget pack config path: {}.",
        config_path.display()
      )
    })?;

    let pack = WidgetPack {
      id: pack_config.name.to_string(),
      r#type: WidgetPackType::Custom,
      config_path: config_path.canonicalize_pretty()?,
      directory_path: pack_dir.canonicalize_pretty()?,
      config: pack_config,
    };

    Ok(pack)
  }

  /// Returns all widget packs as a hashmap.
  pub async fn widget_packs(&self) -> HashMap<String, WidgetPack> {
    self.widget_packs.lock().await.clone()
  }

  /// Finds a widget pack by ID.
  pub async fn widget_pack_by_id(
    &self,
    pack_id: &str,
  ) -> Option<WidgetPack> {
    let widget_packs = self.widget_packs.lock().await;
    widget_packs.get(pack_id).cloned()
  }
}

/// Helper function for setting a default value for a boolean field.
const fn default_bool<const V: bool>() -> bool {
  V
}

/// Helper function for setting the default value for a
/// `WidgetPreset::name` field.
fn default_preset_name() -> String {
  "default".into()
}
