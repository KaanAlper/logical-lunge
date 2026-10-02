use std::{collections::HashMap, sync::Arc};

use tauri::{State, Window};
use crate::everything::FileHit;

#[tauri::command]
pub async fn everything_search(query: String, max_results: Option<u32>) -> Result<Vec<FileHit>, String> {
  tokio::task::spawn_blocking(move || crate::everything::query(&query, max_results.unwrap_or(10)))
    .await.map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn screensaver_state() -> Result<crate::screensaver::State, String> {
  tokio::task::spawn_blocking(crate::screensaver::state).await.map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn screensaver_run(path: String, configure: bool) -> Result<(), String> {
  tokio::task::spawn_blocking(move || crate::screensaver::run(&path, configure)).await.map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn screensaver_set(enabled: bool, minutes: u32, secure: bool, selected: String) -> Result<crate::screensaver::State, String> {
  tokio::task::spawn_blocking(move || crate::screensaver::set(enabled, minutes, secure, &selected)).await.map_err(|err| err.to_string())?
}

#[cfg(target_os = "macos")]
use crate::common::macos::WindowExtMacOs;
#[cfg(target_os = "windows")]
use crate::common::windows::WindowExtWindows;
use crate::{
  providers::{
    ProviderConfig, ProviderFunction, ProviderFunctionResponse,
    ProviderManager,
  },
  shell_state::{ShellCommandArgs, ShellState},
  widget_factory::{WidgetFactory, WidgetOpenOptions, WidgetState},
  widget_pack::{WidgetPack, WidgetPackManager, WidgetPlacement},
};

#[tauri::command]
pub async fn widget_packs(
  widget_pack_manager: State<'_, Arc<WidgetPackManager>>,
) -> Result<Vec<WidgetPack>, String> {
  Ok(
    widget_pack_manager
      .widget_packs()
      .await
      .values()
      .cloned()
      .collect(),
  )
}

#[tauri::command]
pub async fn widget_states(
  widget_factory: State<'_, Arc<WidgetFactory>>,
) -> Result<HashMap<String, WidgetState>, String> {
  Ok(widget_factory.states().await)
}

#[tauri::command]
pub async fn start_widget(
  pack_id: String,
  widget_name: String,
  placement: WidgetPlacement,
  is_preview: bool,
  widget_factory: State<'_, Arc<WidgetFactory>>,
) -> anyhow::Result<(), String> {
  widget_factory
    .start_widget_by_id(
      &pack_id,
      &widget_name,
      &WidgetOpenOptions::Standalone(placement),
      is_preview,
    )
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn start_widget_preset(
  pack_id: String,
  widget_name: String,
  preset_name: String,
  is_preview: bool,
  widget_factory: State<'_, Arc<WidgetFactory>>,
) -> anyhow::Result<(), String> {
  widget_factory
    .start_widget_by_id(
      &pack_id,
      &widget_name,
      &WidgetOpenOptions::Preset(preset_name),
      is_preview,
    )
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn stop_widget_preset(
  pack_id: String,
  widget_name: String,
  preset_name: String,
  widget_factory: State<'_, Arc<WidgetFactory>>,
) -> anyhow::Result<(), String> {
  widget_factory
    .stop_by_preset(&pack_id, &widget_name, &preset_name)
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn listen_provider(
  config_hash: String,
  config: ProviderConfig,
  provider_manager: State<'_, Arc<ProviderManager>>,
) -> anyhow::Result<(), String> {
  provider_manager
    .create(config_hash, config)
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn unlisten_provider(
  config_hash: String,
  provider_manager: State<'_, Arc<ProviderManager>>,
) -> anyhow::Result<(), String> {
  provider_manager
    .stop(config_hash)
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn call_provider_function(
  config_hash: String,
  function: ProviderFunction,
  provider_manager: State<'_, Arc<ProviderManager>>,
) -> anyhow::Result<ProviderFunctionResponse, String> {
  provider_manager
    .call_function(config_hash, function)
    .await
    .map_err(|err| err.to_string())
}

/// Tauri's implementation of `always_on_top` places the window above
/// all normal windows (but not the MacOS menu bar). The following instead
/// sets the z-order of the window to be above the menu bar.
#[tauri::command]
pub fn set_always_on_top(window: Window) -> anyhow::Result<(), String> {
  #[cfg(target_os = "macos")]
  let res = window.set_above_menu_bar();

  #[cfg(not(target_os = "macos"))]
  let res = window.set_always_on_top(true);

  res.map_err(|err| err.to_string())
}

#[tauri::command]
pub fn set_skip_taskbar(
  window: Window,
  skip: bool,
) -> anyhow::Result<(), String> {
  window
    .set_skip_taskbar(skip)
    .map_err(|err| err.to_string())?;

  #[cfg(target_os = "windows")]
  window
    .set_tool_window(skip)
    .map_err(|err| err.to_string())?;

  Ok(())
}

/// Logical Lunge: tells WebView2 whether the widget is on screen. Hiding
/// the window alone does not stop the browser: animations, timers and
/// paints keep running in a hidden widget. With `IsVisible` false the page
/// becomes `hidden`, rendering stops and timers are throttled.
///
/// A window that got keyboard focus while its webview was invisible (the
/// core shows the overview with Win32 and focuses it right away) did not
/// pass the focus on to the browser. Focus is moved again here, but only
/// when the window is in the foreground and the focus is not already in
/// the browser: an extra focus event turned focus fights into a loop.
#[tauri::command]
pub fn set_webview_visible(
  webview: tauri::Webview,
  visible: bool,
) -> anyhow::Result<(), String> {
  #[cfg(target_os = "windows")]
  {
    let refocus = visible
      && webview
        .window()
        .hwnd()
        .is_ok_and(|hwnd| focus_outside_browser(hwnd.0));

    webview
      .with_webview(move |platform| unsafe {
        let _ = platform.controller().SetIsVisible(visible);
      })
      .map_err(|err| err.to_string())?;

    if refocus {
      let _ = webview.set_focus();
    }
  }

  #[cfg(not(target_os = "windows"))]
  let _ = (webview, visible);

  Ok(())
}

/// Whether `window` is the foreground window while keyboard focus is on
/// the window itself (or nowhere) rather than in its browser.
#[cfg(target_os = "windows")]
fn focus_outside_browser(window: *mut std::ffi::c_void) -> bool {
  use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{
      GetForegroundWindow, GetGUIThreadInfo, IsChild, IsWindowVisible, GUITHREADINFO,
    },
  };

  unsafe {
    // A hidden window can stay the foreground window (the core hides the
    // overview with Win32): focusing its browser then reopened the page.
    if GetForegroundWindow().0 != window || !IsWindowVisible(HWND(window)).as_bool() {
      return false;
    }

    let mut info = GUITHREADINFO {
      cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
      ..Default::default()
    };

    if GetGUIThreadInfo(0, &mut info).is_err() || info.hwndFocus.is_invalid()
    {
      return true;
    }

    // Any focused descendant belongs to the embedded content. Do not tie
    // focus recovery to an engine-specific class name.
    info.hwndFocus == HWND(window) || !IsChild(HWND(window), info.hwndFocus).as_bool()
  }
}

#[tauri::command]
pub async fn shell_exec(
  program: String,
  args: ShellCommandArgs,
  options: shell_util::CommandOptions,
  window: Window,
  shell_state: State<'_, ShellState>,
) -> anyhow::Result<shell_util::ShellExecOutput, String> {
  let widget_id = window.label();
  shell_state
    .exec(&widget_id, &program, args, &options)
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn shell_spawn(
  program: String,
  args: ShellCommandArgs,
  options: shell_util::CommandOptions,
  window: Window,
  shell_state: State<'_, ShellState>,
) -> anyhow::Result<shell_util::ProcessId, String> {
  let widget_id = window.label();
  shell_state
    .spawn(&widget_id, &program, args, &options)
    .await
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn shell_write(
  pid: shell_util::ProcessId,
  buffer: shell_util::Buffer,
  shell_state: State<'_, ShellState>,
) -> anyhow::Result<(), String> {
  shell_state
    .write(pid, buffer)
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn shell_kill(
  pid: shell_util::ProcessId,
  shell_state: State<'_, ShellState>,
) -> anyhow::Result<(), String> {
  shell_state.kill(pid).map_err(|err| err.to_string())
}
