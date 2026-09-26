// Logical Lunge: only the providers the shell uses (disk, ip, keyboard,
// komorebi and weather were removed).
#[cfg(windows)]
pub mod audio;
pub mod battery;
pub mod cpu;
mod host;
#[cfg(windows)]
pub mod media;
pub mod memory;
pub mod network;
mod provider;
mod provider_config;
mod provider_function;
mod provider_manager;
mod provider_output;
#[cfg(windows)]
pub mod systray;

pub use provider::*;
pub use provider_config::*;
pub use provider_function::*;
pub use provider_manager::*;
pub use provider_output::*;