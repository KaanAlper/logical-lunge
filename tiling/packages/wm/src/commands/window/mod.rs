#[cfg(target_os = "windows")]
mod fullscreen_spoof;
mod ignore_window;
mod manage_window;
mod mouse_drag;
mod move_window_in_direction;
mod move_window_to_workspace;
mod pin_window;
mod resize_window;
mod run_window_rules;
mod set_window_position;
mod set_window_size;
mod unmanage_window;
mod update_window_state;

#[cfg(target_os = "windows")]
pub use fullscreen_spoof::*;
pub use ignore_window::*;
pub use manage_window::*;
pub use mouse_drag::*;
pub use move_window_in_direction::*;
pub use move_window_to_workspace::*;
pub use pin_window::*;
pub use resize_window::*;
pub use run_window_rules::*;
pub use set_window_position::*;
pub use set_window_size::*;
pub use unmanage_window::*;
pub use update_window_state::*;
