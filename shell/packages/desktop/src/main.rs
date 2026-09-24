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
  fmt::{self, writer::MakeWriterExt},
  layer::SubscriberExt,
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
mod common;
mod config_migration;
mod monitor_state;
mod providers;
mod shell_state;
mod widget_factory;
mod widget_pack;

#[macro_use]
extern crate rocket;

/// Main entry point for the application.
///
/// Conditionally starts Zebar or runs a CLI command based on the given
/// subcommand.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

              // If unable to start Zebar, the error is fatal and a message
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
      commands::shell_exec,
      commands::shell_spawn,
      commands::shell_write,
      commands::shell_kill,
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

/// Starts Zebar - either with a specific widget or all widgets.
async fn start_app(app: &mut tauri::App, cli: Cli) -> anyhow::Result<()> {
  let config_dir = match cli.command() {
    CliCommand::Startup(args) => args.config_dir,
    _ => None,
  }
  .unwrap_or(
    app
      .path()
      .resolve(".glzr/zebar", BaseDirectory::Home)
      .context("Unable to get home directory.")?,
  );

  setup_logging(&cli, &config_dir)?;

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

  // If this is not the first instance of the app, this will emit within
  // the original instance and exit immediately. The CLI command is
  // guaranteed to be one of the open commands here.
  setup_single_instance(app, widget_factory.clone())?;

  // Start the asset server.
  setup_asset_server().await?;

  // Prevent windows from showing up in the dock on MacOS.
  #[cfg(target_os = "macos")]
  app.set_activation_policy(tauri::ActivationPolicy::Accessory);

  // Allow assets to be resolved from the config directory.
  app
    .asset_protocol_scope()
    .allow_directory(&app_settings.config_dir, true)?;

  app.manage(ShellState::new(app.handle(), widget_factory.clone()));
  app.handle().plugin(tauri_plugin_dialog::init())?;
  app.handle().plugin(tauri_plugin_shell::init())?;

  // Initialize `ProviderManager` in Tauri state.
  let (manager, emit_rx) = ProviderManager::new(app.handle());
  app.manage(manager.clone());

  // Open widgets based on CLI command.
  open_widgets_by_cli_command(cli, widget_factory.clone()).await?;

  // Logical Lunge: no tray icon, widget manager / settings window or
  // marketplace -- the shell starts its own widget pack and is the only UI.
  listen_events(app.handle(), monitor_state, widget_factory, manager, emit_rx);

  Ok(())
}

/// Listens for events and updates state accordingly.
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
      let res = tokio::select! {
        Ok(widget_state) = widget_open_rx.recv() => {
          info!("Widget opened.");
          let _ = app_handle.emit("widget-opened", widget_state);
          Ok(())
        },
        Ok(widget_id) = widget_close_rx.recv() => {
          info!("Widget closed.");
          let _ = app_handle.emit("widget-closed", widget_id);
          Ok(())
        },
        Ok(_) = monitors_change_rx.recv() => {
          info!("Monitors changed.");
          widget_factory.relaunch_all().await
        },
        Some(provider_emission) = emit_rx.recv() => {
          info!("Provider emission: {:?}", provider_emission);
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
              open_widgets_by_cli_command(cli, widget_factory).await
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

/// Opens widgets based on CLI command.
async fn open_widgets_by_cli_command(
  cli: Cli,
  widget_factory: Arc<WidgetFactory>,
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
      widget_factory.startup().await
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
/// Error logs are saved to `~/.glzr/zebar/errors.log`.
fn setup_logging(cli: &Cli, config_dir: &Path) -> anyhow::Result<()> {
  let log_level = match cli.command() {
    CliCommand::Startup(args) => args.verbosity.level(),
    _ => Level::INFO,
  };

  let error_writer =
    tracing_appender::rolling::never(config_dir, "errors.log");

  let subscriber = tracing_subscriber::registry()
    .with(
      // Output to stdout with specified verbosity level.
      fmt::Layer::new()
        .with_writer(std::io::stdout.with_max_level(log_level)),
    )
    .with(
      // Output to error log file.
      fmt::Layer::new()
        .with_writer(error_writer.with_max_level(Level::ERROR)),
    );

  tracing::subscriber::set_global_default(subscriber)?;

  info!("Starting with log level {:?}.", log_level.to_string());

  Ok(())
}

