use tracing::info;
use wm_common::{try_warn, WindowRuleEvent};
use wm_platform::NativeWindow;

use crate::{
  commands::window::run_window_rules, traits::WindowGetters,
  user_config::UserConfig, wm_state::WmState,
};

pub fn handle_window_title_changed(
  native_window: &NativeWindow,
  state: &mut WmState,
  config: &mut UserConfig,
) -> anyhow::Result<()> {
  let found_window = state.window_from_native(native_window);

  if let Some(window) = found_window {
    info!("Window title changed: {window}");

    let title = try_warn!(window.native().title());
    // A Store app names its frame once its own window is in it (see
    // `process_name`): its name, not the host's, from then on.
    #[cfg(target_os = "windows")]
    let process_name = (window.native_properties().class_name
      == "ApplicationFrameWindow")
      .then(|| window.native().process_name().ok())
      .flatten();
    #[cfg(not(target_os = "windows"))]
    let process_name: Option<String> = None;

    window.update_native_properties(|properties| {
      properties.title = title;
      if let Some(name) = process_name {
        properties.process_name = name;
      }
    });

    // Run window rules for title change events.
    run_window_rules(
      window,
      &WindowRuleEvent::TitleChange,
      state,
      config,
    )?;
  }

  Ok(())
}
