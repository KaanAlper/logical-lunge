use serde::{Deserialize, Serialize};
use systray_util::{ImageFormat, Systray, SystrayIcon, SystrayIconAction};

use crate::providers::{
  CommonProviderState, Provider, ProviderFunction,
  ProviderFunctionResponse, ProviderInputMsg, RuntimeType,
  SystrayFunction,
};

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SystrayProviderConfig {}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystrayOutput {
  pub icons: Vec<SystrayOutputIcon>,
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystrayOutputIcon {
  pub id: String,
  pub tooltip: String,
  pub icon_bytes: Vec<u8>,
  pub icon_hash: String,
}

impl TryFrom<SystrayIcon> for SystrayOutputIcon {
  type Error = anyhow::Error;

  fn try_from(icon: SystrayIcon) -> Result<Self, Self::Error> {
    Ok(SystrayOutputIcon {
      id: icon.stable_id.to_string(),
      tooltip: icon.tooltip.clone(),
      icon_bytes: icon.to_image_format(ImageFormat::Png)?,
      icon_hash: icon
        .icon_image_hash
        .ok_or(anyhow::anyhow!("Missing hash for icon image."))?,
    })
  }
}

impl std::fmt::Debug for SystrayOutputIcon {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "SystrayOutputIcon {{ id: {}, tooltip: {} }}",
      self.id, self.tooltip
    )
  }
}

pub struct SystrayProvider {
  _config: SystrayProviderConfig,
  common: CommonProviderState,
}

impl SystrayProvider {
  pub fn new(
    _config: SystrayProviderConfig,
    common: CommonProviderState,
  ) -> SystrayProvider {
    SystrayProvider { _config, common }
  }

  fn handle_function(
    systray: &mut Systray,
    function: SystrayFunction,
  ) -> anyhow::Result<ProviderFunctionResponse> {
    match &function {
      SystrayFunction::IconHoverEnter(args) => systray.send_action(
        &args.icon_id.parse()?,
        &SystrayIconAction::HoverEnter,
      ),
      SystrayFunction::IconHoverLeave(args) => systray.send_action(
        &args.icon_id.parse()?,
        &SystrayIconAction::HoverLeave,
      ),
      SystrayFunction::IconHoverMove(args) => systray.send_action(
        &args.icon_id.parse()?,
        &SystrayIconAction::HoverMove,
      ),
      SystrayFunction::IconLeftClick(args) => systray.send_action(
        &args.icon_id.parse()?,
        &SystrayIconAction::LeftClick,
      ),
      SystrayFunction::IconLeftDoubleClick(args) => systray.send_action(
        &args.icon_id.parse()?,
        &SystrayIconAction::LeftDoubleClick,
      ),
      SystrayFunction::IconRightClick(args) => systray.send_action(
        &args.icon_id.parse()?,
        &SystrayIconAction::RightClick,
      ),
      SystrayFunction::IconMiddleClick(args) => systray.send_action(
        &args.icon_id.parse()?,
        &SystrayIconAction::MiddleClick,
      ),
    }?;

    Ok(ProviderFunctionResponse::Null)
  }
}

#[async_trait]
impl Provider for SystrayProvider {
  fn runtime_type(&self) -> RuntimeType {
    RuntimeType::Async
  }

  async fn start_async(&mut self) {
    let Ok(mut systray) = Systray::new() else {
      self.common.emitter.emit_output::<SystrayOutput>(Err(
        anyhow::anyhow!("Failed to initialize systray."),
      ));

      return;
    };

    // Logical Lunge: some programs update their tray icon or tooltip dozens
    // of times a second. Each update re-encoded every icon as PNG and sent
    // them all to the bar, which parsed and redrew them (~33 times a
    // second while idle). Updates are now coalesced (at most one output
    // per 250 ms; the first one after a quiet period goes out at once),
    // nothing is sent when no icon, tooltip or image changed, and PNGs are
    // cached by image hash. An animated icon (e.g. Google Drive's sync
    // spinner, ~30 frames a second) only changes its image: such updates go
    // out at most once a second.
    const MIN_GAP: std::time::Duration = std::time::Duration::from_millis(250);
    const IMAGE_ONLY_GAP: std::time::Duration =
      std::time::Duration::from_secs(1);
    let mut last_emit_at = tokio::time::Instant::now() - IMAGE_ONLY_GAP;
    let mut dirty = true;
    let mut listening = true;
    let mut next_emit = tokio::time::Instant::now();
    let mut last_sig: Option<Vec<(String, String, String)>> = None;
    let mut png_cache: std::collections::HashMap<String, Vec<u8>> =
      std::collections::HashMap::new();

    loop {
      tokio::select! {
        event = systray.events(), if listening => {
          // The tray listener ended: stop polling (it'd return at once).
          if event.is_none() {
            listening = false;
          }
          dirty = true;
        }
        _ = tokio::time::sleep_until(next_emit), if dirty => {
          dirty = false;
          next_emit = tokio::time::Instant::now() + MIN_GAP;

          let icons = systray
            .icons()
            .into_iter()
            .filter(|icon| icon.is_visible)
            .collect::<Vec<_>>();

          let sig = icons
            .iter()
            .map(|icon| {
              (
                icon.stable_id.to_string(),
                icon.tooltip.clone(),
                icon.icon_image_hash.clone().unwrap_or_default(),
              )
            })
            .collect::<Vec<_>>();

          if last_sig.as_ref() == Some(&sig) {
            continue;
          }

          let image_only = last_sig.as_ref().is_some_and(|last| {
            last.len() == sig.len()
              && last
                .iter()
                .zip(&sig)
                .all(|(a, b)| a.0 == b.0 && a.1 == b.1)
          });

          if image_only && last_emit_at.elapsed() < IMAGE_ONLY_GAP {
            dirty = true;
            next_emit = last_emit_at + IMAGE_ONLY_GAP;
            continue;
          }

          last_sig = Some(sig);
          last_emit_at = tokio::time::Instant::now();

          let mut output_icons = Vec::with_capacity(icons.len());
          for icon in icons {
            let Some(hash) = icon.icon_image_hash.clone() else {
              continue;
            };
            let bytes = match png_cache.get(&hash) {
              Some(bytes) => bytes.clone(),
              None => match icon.to_image_format(ImageFormat::Png) {
                Ok(bytes) => {
                  png_cache.insert(hash.clone(), bytes.clone());
                  bytes
                }
                Err(_) => continue,
              },
            };
            output_icons.push(SystrayOutputIcon {
              id: icon.stable_id.to_string(),
              tooltip: icon.tooltip.clone(),
              icon_bytes: bytes,
              icon_hash: hash,
            });
          }

          // Only the images still shown stay cached.
          png_cache.retain(|hash, _| {
            output_icons.iter().any(|icon| &icon.icon_hash == hash)
          });

          self.common.emitter.emit_output(Ok(SystrayOutput {
            icons: output_icons,
          }));
        }
        Some(input) = self.common.input.async_rx.recv() => {
          match input {
            ProviderInputMsg::Stop => {
              break;
            }
            ProviderInputMsg::Function(
              ProviderFunction::Systray(systray_function),
              sender,
            ) => {
              let res = Self::handle_function(&mut systray, systray_function).map_err(|err| err.to_string());
              sender.send(res).unwrap();
            }
            _ => {}
          }
        }
      }
    }
  }
}
