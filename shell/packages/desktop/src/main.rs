// Prevent additional console window on Windows in release mode.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![feature(iterator_try_collect)]

use std::{env, path::Path, sync::Arc};

use anyhow::Context;
use clap::Parser;
use tauri::{
  async_runtime::block_on, path::BaseDirectory, AppHandle, Emitter,
  Manager, RunEvent,
};
use tokio::{sync::mpsc, task};
use tracing::{error, info, Level};
use tracing_subscriber::{
  filter::LevelFilter,
  fmt::{self, MakeWriter},
  layer::SubscriberExt,
  Layer,
};

#[cfg(target_os = "windows")]
use crate::common::windows::WindowExtWindows;
use crate::{
  app_settings::AppSettings,
  asset_server::setup_asset_server,
  cli::{Cli, CliCommand, MonitorType, QueryArgs},
  monitor_state::MonitorState,
  providers::{ProviderEmission, ProviderManager},
  shell_state::ShellState,
  widget_factory::{WidgetFactory, WidgetOpenOptions},
  widget_pack::{MonitorSelection, WidgetPackManager, WidgetPlacement},
};

mod app_settings;
mod asset_server;
mod cli;
mod commands;
mod everything;
mod screensaver;
mod common;
mod monitor_state;
#[cfg(windows)]
mod native_bar;
mod providers;
mod shell_state;
mod widget_factory;
mod widget_pack;
mod web_scale;
mod desktop_widgets;
#[cfg(windows)]
mod desktop_shell;
#[cfg(windows)]
mod web_menu;

#[macro_use]
extern crate rocket;

/// Main entry point for the application.
///
/// Conditionally starts the shell or runs a CLI command based on the given
/// subcommand.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
  // no "drive not ready" / "cannot open file" boxes of Windows: errors come
  // back to us and show as our cards (the crash box: Windows Error
  // Reporting excludes our exes, set by the installer)
  unsafe {
    use windows::Win32::System::Diagnostics::Debug::{SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX};
    let mode = SetErrorMode(SEM_FAILCRITICALERRORS);
    SetErrorMode(mode | SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
  }
  // Attach to parent console on Windows in release mode.
  #[cfg(all(windows, not(debug_assertions)))]
  {
    use windows::Win32::System::Console::{
      AttachConsole, ATTACH_PARENT_PROCESS,
    };
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
  }

  tauri::async_runtime::set(tokio::runtime::Handle::current());

  let app = tauri::Builder::default()
    .setup(|app| {
      task::block_in_place(|| {
        block_on(async move {
          let cli = Cli::parse();

          match cli.command() {
            CliCommand::Query(args) => output_query(app, args),
            _ => {
              let start_res = start_app(app, cli).await;

              // If unable to start the shell, the error is fatal and a message
              // dialog is shown.
              if let Err(err) = &start_res {
                // TODO: Show error dialog.
                error!("{:?}", err);
              };

              start_res
            }
          }?;

          Ok(())
        })
      })
    })
    .invoke_handler(tauri::generate_handler![
      commands::widget_packs,
      commands::widget_states,
      commands::start_widget,
      commands::start_widget_preset,
      commands::stop_widget_preset,
      commands::listen_provider,
      commands::unlisten_provider,
      commands::call_provider_function,
      commands::set_always_on_top,
      commands::set_skip_taskbar,
      commands::set_webview_visible,
      commands::shell_exec,
      commands::shell_spawn,
      commands::shell_write,
      commands::shell_kill,
      commands::everything_search,
      commands::everything_search_page,
      commands::screensaver_state,
      commands::screensaver_set,
      commands::screensaver_run,
      web_menu::desktop_menu_current,
      web_menu::desktop_menu_action,
      web_menu::app_properties,
      desktop_widgets::desktop_widgets_load,
      desktop_widgets::desktop_widgets_update,
      desktop_widgets::desktop_widgets_bootstrap,
      desktop_widgets::desktop_widgets_regions,
      desktop_widgets::desktop_widgets_editing,
      desktop_widgets::desktop_widgets_cursor,
    ])
    .build(tauri::generate_context!())?;

  app.run(|app, event| {
    if let RunEvent::ExitRequested { code, api, .. } = &event {
      // Logical Lunge: Tauri requests an exit when the last window closes
      // (`code` is `None`). The shell keeps running without windows, so
      // that request is refused here instead of keeping a hidden
      // placeholder webview open (it started a second WebView2 browser:
      // six processes, ~145 MB). Exits with a code (`AppHandle::exit`)
      // cannot be prevented and proceed.
      if code.is_none() {
        api.prevent_exit();
        return;
      }

      // Deallocate any appbars on Windows.
      #[cfg(target_os = "windows")]
      {
        for (_, window) in app.webview_windows() {
          let _ = window.as_ref().window().deallocate_app_bar();
        }
      }
    }
  });

  Ok(())
}

/// Query state and print to the console.
fn output_query(app: &tauri::App, args: QueryArgs) -> anyhow::Result<()> {
  match args {
    QueryArgs::Monitors => {
      let monitors = MonitorState::new(&app.handle());
      cli::print_and_exit(monitors.output_str());
      Ok(())
    }
  }
}

/// Starts the shell - either with a specific widget or all widgets.
async fn start_app(app: &mut tauri::App, cli: Cli) -> anyhow::Result<()> {
  let config_dir = match cli.command() {
    CliCommand::Startup(args) => args.config_dir,
    _ => None,
  }
  .unwrap_or(
    // Logical Lunge: the UI ships with the app, in `ui` next to the exe.
    env::current_exe()?
      .parent()
      .context("Unable to get the executable's directory.")?
      .join("ui"),
  );

  setup_logging(&cli, app.handle())?;
  // A panic's message and place go to shell.log (the bar and providers
  // recover from it; only "panic" was logged, with no way to tell why).
  let default_hook = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    tracing::error!("panic: {}", info);
    default_hook(info);
  }));

  // Initialize `AppSettings` in Tauri state.
  let app_settings = Arc::new(AppSettings::new(app.handle(), config_dir)?);
  app.manage(app_settings.clone());

  // Initialize `WidgetPackManager` in Tauri state.
  let widget_pack_manager =
    Arc::new(WidgetPackManager::new(app_settings.clone())?);
  app.manage(widget_pack_manager.clone());

  // Initialize `MonitorState` in Tauri state.
  let monitor_state = Arc::new(MonitorState::new(app.handle()));
  app.manage(monitor_state.clone());

  // Initialize `WidgetFactory` in Tauri state.
  let widget_factory = Arc::new(WidgetFactory::new(
    app.handle(),
    app_settings.clone(),
    widget_pack_manager.clone(),
    monitor_state.clone(),
  ));
  app.manage(widget_factory.clone());
  {
    let factory = widget_factory.clone();
    task::spawn(async move {
      let mut last = web_scale::factor();
      let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
      loop {
        interval.tick().await;
        let current = web_scale::factor();
        if current != last {
          factory.apply_interface_scale(current).await;
          last = current;
        }
      }
    });
  }

  // Logical Lunge: `LL_NATIVE_BAR=demo` runs only the native bar, as a
  // second instance next to the running shell (for testing): no single
  // instance handoff, no asset server, no web widgets.
  let native_bar = std::env::var("LL_NATIVE_BAR").unwrap_or_default();
  let demo = native_bar == "demo";

  // If this is not the first instance of the app, this will emit within
  // the original instance and exit immediately. The CLI command is
  // guaranteed to be one of the open commands here.
  if !demo {
    setup_single_instance(app, widget_factory.clone())?;

    // Start the asset server.
    setup_asset_server().await?;
  }

  // Prevent windows from showing up in the dock on MacOS.
  #[cfg(target_os = "macos")]
  app.set_activation_policy(tauri::ActivationPolicy::Accessory);

  // Allow assets to be resolved from the config directory.
  app
    .asset_protocol_scope()
    .allow_directory(&app_settings.config_dir, true)?;

  app.manage(ShellState::new(app.handle(), widget_factory.clone()));
  web_menu::listen(app.handle());
  desktop_widgets::start(app.handle());
  app.handle().plugin(tauri_plugin_dialog::init())?;
  app.handle().plugin(tauri_plugin_shell::init())?;

  // Initialize `ProviderManager` in Tauri state.
  let (manager, emit_rx) = ProviderManager::new(app.handle());
  app.manage(manager.clone());

  // Logical Lunge: the bar is native (Direct2D, no WebView) when the user's
  // prefs say "bar": "native" or LL_NATIVE_BAR=on (web until the native
  // bar is verified: native_bar::prefs_bar). If the native
  // bar cannot start, dies later, or died at the last starts, the web bar is
  // opened instead: there is always a bar.
  let pack_dir = app_settings.config_dir.join("logical-lunge");
  let mut web_bar = true;
  #[cfg(windows)]
  {
    let want_native = match native_bar.as_str() {
      "demo" | "on" | "native" => true,
      "off" | "web" => false,
      _ => native_bar::prefs_bar(&pack_dir) != "web",
    };
    if want_native && !demo && native_bar::crash_loop() {
      error!("Native bar: it failed at the last starts, using the web bar this time.");
    } else if want_native {
      let fallback: Box<dyn Fn() + Send + Sync> = {
        let factory = widget_factory.clone();
        let rt = tokio::runtime::Handle::current();
        Box::new(move || {
          let factory = factory.clone();
          rt.spawn(async move {
            if let Err(err) = factory
              .start_widget_by_id("logical-lunge", "bar", &WidgetOpenOptions::Preset("default".into()), false)
              .await
            {
              error!("Web bar (fallback): {:?}", err);
            }
          });
        })
      };
      let emit: Box<dyn Fn(&str) + Send + Sync> = {
        let handle = app.handle().clone();
        Box::new(move |event| {
          if let Err(err) = handle.emit(event, ()) {
            tracing::warn!("Native bar: event {}: {:?}", event, err);
          }
        })
      };
      match native_bar::start(manager.clone(), native_bar::Options { pack_dir: pack_dir.clone(), demo, emit }, fallback) {
        Ok(()) => {
          web_bar = demo;
          NATIVE_BAR_UP.store(!demo, std::sync::atomic::Ordering::Release);
        }
        Err(err) => error!("Native bar: {:?}; using the web bar.", err),
      }
    }
  }

  // Open widgets based on CLI command.
  if !demo {
    open_widgets_by_cli_command(cli, widget_factory.clone(), !web_bar).await?;
  }

  // Logical Lunge: no tray icon, widget manager / settings window or
  // marketplace -- the shell starts its own widget pack and is the only UI.
  listen_events(app.handle(), monitor_state, widget_factory, manager, emit_rx);

  Ok(())
}

/// Listens for events and updates state accordingly.
/// Widgets opened again on a monitor change, one change at a time.
static RELAUNCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn listen_events(
  app_handle: &AppHandle,
  monitor_state: Arc<MonitorState>,
  widget_factory: Arc<WidgetFactory>,
  manager: Arc<ProviderManager>,
  mut emit_rx: mpsc::UnboundedReceiver<ProviderEmission>,
) {
  let app_handle = app_handle.clone();
  let mut widget_open_rx = widget_factory.open_tx.subscribe();
  let mut widget_close_rx = widget_factory.close_tx.subscribe();
  let mut monitors_change_rx = monitor_state.change_tx.subscribe();

  task::spawn(async move {
    loop {
      let res: anyhow::Result<()> = tokio::select! {
        Ok(widget_state) = widget_open_rx.recv() => {
          info!("Widget opened.");
          let _ = app_handle.emit("widget-opened", widget_state);
          Ok(())
        },
        Ok(widget_id) = widget_close_rx.recv() => {
          info!("Widget closed.");
          // its helpers (an event stream, the keyboard's input) end with it
          if let Some(shell) = app_handle.try_state::<ShellState>() {
            shell.kill_widget(&widget_id);
          }
          let _ = app_handle.emit("widget-closed", widget_id);
          Ok(())
        },
        Ok(_) = monitors_change_rx.recv() => {
          info!("Monitors changed.");
          // in its own task, one at a time: building the widgets again takes
          // seconds, and this loop also forwards the bar's provider updates
          let widget_factory = widget_factory.clone();
          task::spawn(async move {
            let _one = RELAUNCH.lock().await;
            if let Err(err) = widget_factory.relaunch_all().await {
              error!("{:?}", err);
            }
          });
          Ok(())
        },
        Some(provider_emission) = emit_rx.recv() => {
          // debug: formatting every emission (tray icons as number arrays)
          // cost time on each update even with nothing to show it
          tracing::debug!("Provider emission: {:?}", provider_emission);
          #[cfg(windows)]
          native_bar::forward(&provider_emission);
          let _ = app_handle.emit("provider-emit", provider_emission.clone());
          manager.update_cache(provider_emission).await;
          Ok(())
        },
      };

      if let Err(err) = res {
        error!("{:?}", err);
      }
    }
  });
}

/// Setup single instance Tauri plugin.
fn setup_single_instance(
  app: &tauri::App,
  widget_factory: Arc<WidgetFactory>,
) -> anyhow::Result<()> {
  app.handle().plugin(tauri_plugin_single_instance::init(
    move |_, args, _| {
      let widget_factory = widget_factory.clone();

      task::spawn(async move {
        let res = match Cli::try_parse_from(args) {
          Ok(cli) => {
            // No-op if no subcommand is provided.
            if cli.command() != CliCommand::Empty {
              open_widgets_by_cli_command(cli, widget_factory, NATIVE_BAR_UP.load(std::sync::atomic::Ordering::Acquire)).await
            } else {
              Ok(())
            }
          }
          _ => Err(anyhow::anyhow!("Failed to parse CLI arguments.")),
        };

        if let Err(err) = res {
          error!("{:?}", err);
        }
      });
    },
  ))?;

  Ok(())
}

/// The native bar is up: a later "startup" (second instance) must not open the web bar.
static NATIVE_BAR_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Opens widgets based on CLI command.
async fn open_widgets_by_cli_command(
  cli: Cli,
  widget_factory: Arc<WidgetFactory>,
  native_bar_up: bool,
) -> anyhow::Result<()> {
  let res = match cli.command() {
    CliCommand::StartWidget(args) => {
      widget_factory
        .start_widget_by_id(
          &args.pack_id,
          &args.widget_name,
          &WidgetOpenOptions::Standalone(WidgetPlacement {
            anchor: args.anchor,
            offset_x: args.offset_x,
            offset_y: args.offset_y,
            width: args.width,
            height: args.height,
            monitor_selection: match args.monitor_type {
              MonitorType::All => MonitorSelection::All,
              MonitorType::Primary => MonitorSelection::Primary,
              MonitorType::Secondary => MonitorSelection::Secondary,
            },
            dock_to_edge: Default::default(),
          }),
          false,
        )
        .await
    }
    CliCommand::StartWidgetPreset(args) => {
      widget_factory
        .start_widget_by_id(
          &args.pack_id,
          &args.widget_name,
          &WidgetOpenOptions::Preset(args.preset_name),
          false,
        )
        .await
    }
    CliCommand::Startup(_) | CliCommand::Empty => {
      widget_factory.startup_skipping(if native_bar_up { &["bar"] } else { &[] }).await
    }
    _ => unreachable!(),
  };

  if let Err(err) = res {
    error!("Failed to open widgets: {:?}", err);
  }

  Ok(())
}

/// Initialize logging with the verbosity level specified in the CLI args.
///
/// shell.log, kept small for weeks of uptime: past 4 MB it becomes
/// shell.log.old (the core and the bug report read it by this name, so it
/// is not renamed by date).
struct LogFile {
  path: std::path::PathBuf,
  file: std::sync::Mutex<Option<std::fs::File>>,
}

impl LogFile {
  const MAX: u64 = 4 * 1024 * 1024;

  fn new(path: std::path::PathBuf) -> Self {
    Self {
      path,
      file: std::sync::Mutex::new(None),
    }
  }
}

impl std::io::Write for &LogFile {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
    let full = file
      .as_ref()
      .and_then(|f| f.metadata().ok())
      .is_some_and(|m| m.len() > LogFile::MAX);
    if full {
      *file = None;
      let _ = std::fs::rename(&self.path, self.path.with_extension("log.old"));
    }
    if file.is_none() {
      *file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&self.path)
        .ok();
    }
    match file.as_mut() {
      Some(f) => f.write(buf),
      None => Ok(buf.len()),
    }
  }

  fn flush(&mut self) -> std::io::Result<()> {
    let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
    file.as_mut().map_or(Ok(()), |f| f.flush())
  }
}

impl<'a> MakeWriter<'a> for LogFile {
  type Writer = &'a LogFile;

  fn make_writer(&'a self) -> Self::Writer {
    self
  }
}

/// Warnings and errors are saved to `%LOCALAPPDATA%/LogicalLunge/logs/shell.log`.
fn setup_logging(cli: &Cli, app: &AppHandle) -> anyhow::Result<()> {
  let log_level = match cli.command() {
    CliCommand::Startup(args) => args.verbosity.level(),
    _ => Level::INFO,
  };

  // Logical Lunge's common log folder, next to the other parts' logs.
  let log_dir = app
    .path()
    .resolve("LogicalLunge/logs", BaseDirectory::LocalData)
    .context("Unable to resolve the log directory.")?;

  let _ = std::fs::create_dir_all(&log_dir);
  let file_writer = LogFile::new(log_dir.join("shell.log"));

  // Each layer filters before formatting (a writer's level filter only
  // drops what was already formatted).
  let subscriber = tracing_subscriber::registry()
    .with(
      // Output to stdout with specified verbosity level.
      fmt::Layer::new()
        .with_writer(std::io::stdout)
        .with_filter(LevelFilter::from_level(log_level)),
    )
    .with(
      // Output to the log file, without terminal colors.
      fmt::Layer::new()
        .with_ansi(false)
        .with_writer(file_writer)
        .with_filter(LevelFilter::WARN),
    );

  tracing::subscriber::set_global_default(subscriber)?;

  info!("Starting with log level {:?}.", log_level.to_string());

  Ok(())
}

