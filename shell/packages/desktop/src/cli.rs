//! `lunge-shell [startup] [--config-dir <ui folder>] [-v | -q | --log-level <level>]`
//!
//! The core starts the shell without arguments; `startup` is still accepted
//! from older launchers and changes nothing.

use std::path::PathBuf;

use clap::{Args, Parser, ValueEnum};
use tracing::Level;

#[derive(Clone, Debug, Parser)]
#[clap(
  about = "Logical Lunge shell: the bar, the Super menu, the panels and the desktop widgets.",
  long_about = None
)]
pub struct Cli {
  /// accepted and ignored ("startup")
  #[clap(hide = true)]
  pub command: Option<String>,

  /// The shell's UI folder (fonts, translations); the default is `ui` next
  /// to the executable.
  #[clap(long, value_hint = clap::ValueHint::DirPath)]
  pub config_dir: Option<PathBuf>,

  /// Logging verbosity.
  #[clap(flatten)]
  pub verbosity: Verbosity,
}

/// Verbosity flags to be used with `#[command(flatten)]`.
#[derive(Args, Clone, Debug, PartialEq)]
#[clap(about = None, long_about = None)]
pub struct Verbosity {
  /// Enables verbose logging.
  #[clap(short = 'v', long, action)]
  verbose: bool,

  /// Disables logging.
  #[clap(short = 'q', long, action, conflicts_with = "verbose")]
  quiet: bool,

  /// Set log level directly (overrides verbose/quiet flags).
  ///
  /// Can also be set via `LOG_LEVEL` environment variable.
  #[clap(long, env = "LOG_LEVEL", value_enum)]
  log_level: Option<LogLevel>,
}

impl Verbosity {
  /// Gets the log level based on the verbosity flags.
  #[must_use]
  pub fn level(&self) -> Level {
    if let Some(level) = &self.log_level {
      return level.clone().into();
    }
    match (self.verbose, self.quiet) {
      (true, _) => Level::DEBUG,
      (_, true) => Level::ERROR,
      _ => Level::INFO,
    }
  }
}

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum LogLevel {
  Debug,
  Info,
  Warn,
  Error,
}

impl From<LogLevel> for Level {
  fn from(log_level: LogLevel) -> Self {
    match log_level {
      LogLevel::Debug => Level::DEBUG,
      LogLevel::Info => Level::INFO,
      LogLevel::Warn => Level::WARN,
      LogLevel::Error => Level::ERROR,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_the_launchers_arguments() {
    let cli = Cli::try_parse_from(["lunge-shell"]).unwrap();
    assert_eq!(cli.command, None);
    assert_eq!(cli.verbosity.level(), Level::INFO);
    let cli = Cli::try_parse_from(["lunge-shell", "startup", "--config-dir", "C:\\ui", "-v"]).unwrap();
    assert_eq!(cli.command.as_deref(), Some("startup"));
    assert_eq!(cli.config_dir, Some(PathBuf::from("C:\\ui")));
    assert_eq!(cli.verbosity.level(), Level::DEBUG);
  }
}
