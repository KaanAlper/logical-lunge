use crate::animations::AnimationsConfig;
use crate::colors::ColorBrushConfig;
use crate::effects::EffectsConfig;
use crate::render_backend::RenderBackendConfig;
use crate::utils::{get_adjusted_radius, get_window_corner_preference};
use crate::{APP_STATE, BG_SERVICES, DirectXDevices, IS_WINDOWS_11};
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMWCP_DEFAULT, DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWCP_ROUNDSMALL,
};

/// The `borders:` section of the window manager config. Unknown keys (e.g. those of a standalone
/// tacky-borders config.yaml: `watch_config_changes`, `enable_ipc_server`, `komorebi_colors`) are
/// ignored, so an older config never turns the borders off.
#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    #[serde(alias = "rendering_backend")]
    pub render_backend: RenderBackendConfig,
    #[serde(default = "serde_default_global")]
    pub global: Global,
    #[serde(default)]
    pub window_rules: Vec<WindowRule>,
}

// Show borders even if the config.yaml is completely empty
// NOTE: This is intentionally kept separate from the Default trait because I want the
// width/offset zeroed out when config deserialization fails and falls back to Config::default()
fn serde_default_global() -> Global {
    Global {
        border_width: WidthConfig::serde_default(),
        border_offset: OffsetConfig::serde_default(),
        ..Default::default()
    }
}

#[derive(Debug, Default, Clone, Deserialize, PartialEq)]
pub struct Global {
    #[serde(default = "WidthConfig::serde_default")]
    pub border_width: WidthConfig,
    #[serde(default = "OffsetConfig::serde_default")]
    pub border_offset: OffsetConfig,
    #[serde(default)]
    pub border_radius: RadiusConfig,
    #[serde(default)]
    pub border_z_order: ZOrderMode,
    #[serde(default = "serde_default_bool::<true>")]
    pub follow_native_border: bool,
    #[serde(default)]
    pub active_color: ColorBrushConfig,
    #[serde(default)]
    pub inactive_color: ColorBrushConfig,
    #[serde(default)]
    pub animations: AnimationsConfig,
    #[serde(default)]
    pub effects: EffectsConfig,
    #[serde(alias = "init_delay")]
    #[serde(default = "serde_default_u64::<250>")]
    pub initialize_delay: u64, // Adjust delay when creating new windows/borders
    #[serde(alias = "restore_delay")]
    #[serde(default = "serde_default_u64::<200>")]
    pub unminimize_delay: u64, // Adjust delay when restoring minimized windows
}

pub fn serde_default_u64<const V: u64>() -> u64 {
    V
}

pub fn serde_default_i32<const V: i32>() -> i32 {
    V
}

// f32 cannot be a const, so we have to do the following instead
pub fn serde_default_f32<const V: i32>() -> f32 {
    V as f32
}

pub fn serde_default_bool<const V: bool>() -> bool {
    V
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct WindowRule {
    #[serde(rename = "match")]
    pub kind: Option<MatchKind>,
    pub name: Option<String>,
    pub strategy: Option<MatchStrategy>,
    pub border_width: Option<WidthConfig>,
    pub border_offset: Option<OffsetConfig>,
    pub border_radius: Option<RadiusConfig>,
    pub border_z_order: Option<ZOrderMode>,
    pub follow_native_border: Option<bool>,
    pub active_color: Option<ColorBrushConfig>,
    pub inactive_color: Option<ColorBrushConfig>,
    pub animations: Option<AnimationsConfig>,
    pub effects: Option<EffectsConfig>,
    #[serde(alias = "init_delay")]
    pub initialize_delay: Option<u64>,
    #[serde(alias = "restore_delay")]
    pub unminimize_delay: Option<u64>,
    pub enabled: Option<EnableMode>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum MatchKind {
    Title,
    Class,
    Process,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum MatchStrategy {
    Equals,
    Contains,
    Regex,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct WidthConfig(f32);

impl WidthConfig {
    pub fn new(width: f32) -> Self {
        Self(width)
    }

    // TODO: Maybe rename this and other to_x methods to to_raw or smth idk
    /// Returns a DPI-adjusted raw width value
    pub fn to_width(&self, dpi: f32) -> i32 {
        (self.0 as f32 * dpi / 96.0).round() as i32
    }

    fn serde_default() -> Self {
        Self(4.0)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum OffsetConfig {
    Uniform(i32),
    PerSide(Offset),
}

impl OffsetConfig {
    pub fn new(offset: i32) -> Self {
        Self::Uniform(offset)
    }

    /// Returns a DPI-adjusted offset for each side
    pub fn to_offset(&self, dpi: f32) -> Offset {
        let scale = |value: i32| (value as f32 * dpi / 96.0).round() as i32;
        match *self {
            Self::Uniform(offset) => Offset::new(scale(offset)),
            Self::PerSide(offset) => Offset {
                top: scale(offset.top),
                left: scale(offset.left),
                right: scale(offset.right),
                bottom: scale(offset.bottom),
            },
        }
    }

    fn serde_default() -> Self {
        Self::Uniform(-1)
    }
}

impl Default for OffsetConfig {
    fn default() -> Self {
        Self::Uniform(0)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Offset {
    #[serde(default)]
    pub top: i32,
    #[serde(default)]
    pub left: i32,
    #[serde(default)]
    pub right: i32,
    #[serde(default)]
    pub bottom: i32,
}

impl Offset {
    pub fn new(offset: i32) -> Self {
        Self {
            top: offset,
            left: offset,
            right: offset,
            bottom: offset,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub enum RadiusConfig {
    #[default]
    Auto,
    Square,
    Round,
    RoundSmall,
    #[serde(untagged)]
    Custom(f32),
}

impl RadiusConfig {
    pub fn to_radius(&self, border_width: i32, dpi: u32, tracking_window: HWND) -> f32 {
        match self {
            // We also check Custom(-1.0) for legacy reasons (don't wanna break anyone's old config)
            RadiusConfig::Auto | RadiusConfig::Custom(-1.0) => {
                // I believe this will error on Windows 10, so we'll just use a default
                match get_window_corner_preference(tracking_window).unwrap_or(DWMWCP_DEFAULT) {
                    DWMWCP_DEFAULT => {
                        if *IS_WINDOWS_11 {
                            get_adjusted_radius(8.0, dpi, border_width)
                        } else {
                            0.0
                        }
                    }
                    DWMWCP_DONOTROUND => 0.0,
                    DWMWCP_ROUND => get_adjusted_radius(8.0, dpi, border_width),
                    DWMWCP_ROUNDSMALL => get_adjusted_radius(4.0, dpi, border_width),
                    _ => 0.0,
                }
            }
            RadiusConfig::Square => 0.0,
            RadiusConfig::Round => get_adjusted_radius(8.0, dpi, border_width),
            RadiusConfig::RoundSmall => get_adjusted_radius(4.0, dpi, border_width),
            RadiusConfig::Custom(radius) => get_adjusted_radius(*radius, dpi, border_width),
        }
    }
}
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
pub enum EnableMode {
    #[default]
    Auto,
    #[serde(untagged)]
    Bool(bool),
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
pub enum ZOrderMode {
    #[default]
    AboveWindow,
    BelowWindow,
}

impl Config {
    /// Parses the `borders:` YAML handed over by the window manager.
    pub fn from_yaml(yaml: &str) -> anyhow::Result<Self> {
        let yaml = if yaml.trim().is_empty() { "{}" } else { yaml };
        serde_yaml_ng::from_str(yaml).map_err(anyhow::Error::new)
    }

    pub fn create() -> anyhow::Result<Self> {
        Self::from_yaml(&crate::config_source())
    }

    pub fn reload() {
        let new_config = match Self::create() {
            Ok(config) => {
                BG_SERVICES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).reload(&config);

                let mut directx_devices_opt = APP_STATE.directx_devices.write().unwrap_or_else(std::sync::PoisonError::into_inner);

                if config.render_backend == RenderBackendConfig::V2 && directx_devices_opt.is_none()
                {
                    let direct_x_devices = DirectXDevices::new(&APP_STATE.render_factory)
                        .unwrap_or_else(|err| {
                            error!("could not create directx devices: {err:#}");
                            panic!("could not create directx devices: {err:#}");
                        });

                    *directx_devices_opt = Some(direct_x_devices);
                } else if config.render_backend == RenderBackendConfig::Legacy
                    && directx_devices_opt.is_some()
                {
                    *directx_devices_opt = None;
                }

                config
            }
            Err(err) => {
                // No message boxes: the error goes to the log and the borders keep defaults
                error!("could not reload border config: {err:#}");
                Config::default()
            }
        };
        *APP_STATE.config.write().unwrap_or_else(std::sync::PoisonError::into_inner) = new_config;
    }

    pub fn is_theme_aware_enabled(&self) -> bool {
        Self::is_color_theme_aware(&self.global.active_color)
            || Self::is_color_theme_aware(&self.global.inactive_color)
            || self.window_rules.iter().any(|rule| {
                rule.active_color
                    .as_ref()
                    .map_or(false, Self::is_color_theme_aware)
                    || rule
                        .inactive_color
                        .as_ref()
                        .map_or(false, Self::is_color_theme_aware)
            })
    }

    fn is_color_theme_aware(config: &ColorBrushConfig) -> bool {
        matches!(config, ColorBrushConfig::ThemeAware(_))
    }
}
