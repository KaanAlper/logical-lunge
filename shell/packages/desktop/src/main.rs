// Prevent additional console window on Windows in release mode.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Logical Lunge shell: the native bar, the Super menu, the panels, the
//! notification cards and the desktop widgets, all drawn with Direct2D and
//! DirectComposition on their own threads. This process only starts them,
//! runs the providers (audio, media, network, tray ...) that feed them and
//! keeps going while they run.

use std::{env, path::PathBuf, sync::Arc};

use anyhow::Context;
use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::{
  filter::LevelFilter,
  fmt::{self, MakeWriter},
  layer::SubscriberExt,
  Layer,
};

use crate::{
  cli::Cli,
  providers::{ProviderEmission, ProviderManager},
};

mod bus;
mod cli;
mod common;
mod everything;
#[cfg(windows)]
mod native_bar;
mod providers;
mod screensaver;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  // no "drive not ready" / "cannot open file" boxes of Windows: errors come
  // back to us and show as our cards (the crash box: Windows Error
  // Reporting excludes our exes, set by the installer)
  #[cfg(windows)]
  unsafe {
    use windows::Win32::System::Diagnostics::Debug::{
      SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOOPENFILEERRORBOX,
    };
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

  let cli = Cli::parse();
  setup_logging(&cli)?;
  // A panic's message and place go to shell.log (the bar and providers
  // recover from it; only "panic" was logged, with no way to tell why).
  let default_hook = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    tracing::error!("panic: {}", info);
    default_hook(info);
  }));

  if let Err(err) = run(cli).await {
    error!("{:?}", err);
    return Err(err);
  }
  // the bar, the panels and the providers run on their own threads
  std::future::pending::<()>().await;
  Ok(())
}

/// Starts the providers and the native bar.
async fn run(cli: Cli) -> anyhow::Result<()> {
  let cli_planned = cli.planned;
  // The UI ships with the app, in `ui` next to the exe; native code reads
  // its fonts and translations from `ui\logical-lunge`.
  let ui_dir = match cli.config_dir {
    Some(dir) => dir,
    None => env::current_exe()?
      .parent()
      .context("Unable to get the executable's directory.")?
      .join("ui"),
  };
  let pack_dir = ui_dir.join("logical-lunge");

  // `LL_NATIVE_BAR=demo` runs only the native bar, as a second instance
  // next to the running shell (for testing); `off` starts no bar
  // (debugging).
  let native_bar = env::var("LL_NATIVE_BAR").unwrap_or_default();
  let demo = native_bar == "demo";

  if !demo && !first_instance() {
    info!("The shell is already running; this start ends.");
    std::process::exit(0);
  }

  let (manager, emit_rx) = ProviderManager::new();
  forward_providers(manager.clone(), emit_rx);

  // The bar is native (Direct2D, no WebView). If it fails, it builds itself
  // again inside the shell (native_bar::start); if it keeps failing, or the
  // shell kept dying at its last starts (`crash_loop`), the shell goes on
  // without a bar and the core brings Windows' taskbar and Start menu back.
  #[cfg(windows)]
  if native_bar != "off" {
    // the shell's own events (cards, panel pages, the keyboard) reach the bar
    bus::subscribe(native_bar::on_bus);
    let opts = native_bar::Options { pack_dir, demo, native_overview: !demo };
    if !demo && !cli_planned && native_bar::crash_loop() {
      // The shell kept dying right after starting: run without the bar for
      // a while (the core gives Windows' taskbar back), then try it again —
      // giving up for good left the desktop barless until a restart.
      error!("Native bar: the shell died at its last starts; the bar is tried again in {} s.", CRASH_LOOP_PAUSE.as_secs());
      std::thread::spawn(move || {
        std::thread::sleep(CRASH_LOOP_PAUSE);
        if let Err(err) = native_bar::start(manager, opts) {
          error!("Native bar: start after the pause failed: {:?}", err);
        }
      });
    } else {
      // waits up to a few seconds for the first bars (not on a runtime thread)
      let started = tokio::task::block_in_place(|| native_bar::start(manager, opts));
      // A failed first start is logged; the bar's guard goes on trying.
      if let Err(err) = started {
        error!("Native bar: first start failed: {:?}", err);
      }
    }
  }
  #[cfg(not(windows))]
  let _ = (pack_dir, manager);

  Ok(())
}

/// How long a shell that kept dying at start runs without the bar.
#[cfg(windows)]
const CRASH_LOOP_PAUSE: std::time::Duration = std::time::Duration::from_secs(90);

/// One shell per session: a second start (the core's watchdog racing a
/// user's start) ends at once. The mutex lives as long as the process.
#[cfg(windows)]
fn first_instance() -> bool {
  use windows::{
    core::w,
    Win32::{Foundation::ERROR_ALREADY_EXISTS, System::Threading::CreateMutexW},
  };
  unsafe {
    match CreateMutexW(None, true, w!("Local\\LogicalLunge.Shell")) {
      // kept open on purpose: released when the process ends
      Ok(_handle) => windows::Win32::Foundation::GetLastError() != ERROR_ALREADY_EXISTS,
      // no mutex: better two shells than none
      Err(_) => true,
    }
  }
}

#[cfg(not(windows))]
fn first_instance() -> bool {
  true
}

/// Every provider emission goes to the bar (it keeps the ones it asked
/// for) and into the manager's cache (a later request gets the latest one).
fn forward_providers(
  manager: Arc<ProviderManager>,
  mut emit_rx: tokio::sync::mpsc::UnboundedReceiver<ProviderEmission>,
) {
  tokio::task::spawn(async move {
    while let Some(emission) = emit_rx.recv().await {
      // debug: formatting every emission (tray icons as number arrays)
      // cost time on each update even with nothing to show it
      tracing::debug!("Provider emission: {:?}", emission);
      #[cfg(windows)]
      native_bar::forward(&emission);
      manager.update_cache(emission).await;
    }
  });
}

/// shell.log, kept small for weeks of uptime: past 4 MB it becomes
/// shell.log.old (the core and the bug report read it by this name, so it
/// is not renamed by date).
struct LogFile {
  path: PathBuf,
  file: std::sync::Mutex<Option<std::fs::File>>,
}

impl LogFile {
  const MAX: u64 = 4 * 1024 * 1024;

  fn new(path: PathBuf) -> Self {
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

/// Warnings and errors are saved to `%LOCALAPPDATA%\LogicalLunge\logs\shell.log`,
/// next to the other parts' logs.
fn setup_logging(cli: &Cli) -> anyhow::Result<()> {
  let log_level = cli.verbosity.level();
  let log_dir = env::var_os("LOCALAPPDATA")
    .map(PathBuf::from)
    .context("LOCALAPPDATA is not set.")?
    .join("LogicalLunge")
    .join("logs");
  let _ = std::fs::create_dir_all(&log_dir);
  let file_writer = LogFile::new(log_dir.join("shell.log"));

  // Each layer filters before formatting (a writer's level filter only
  // drops what was already formatted).
  let subscriber = tracing_subscriber::registry()
    .with(
      fmt::Layer::new()
        .with_writer(std::io::stdout)
        .with_filter(LevelFilter::from_level(log_level)),
    )
    .with(
      fmt::Layer::new()
        .with_ansi(false)
        .with_writer(file_writer)
        .with_filter(LevelFilter::WARN),
    );

  tracing::subscriber::set_global_default(subscriber)?;
  info!("Starting with log level {:?}.", log_level.to_string());
  Ok(())
}
