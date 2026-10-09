mod error;
mod systray;
mod tray_spy;
mod util;

pub use error::*;
pub use systray::*;
pub(crate) use tray_spy::*;
pub use tray_spy::set_quiet;
pub(crate) use util::*;

pub type Result<T> = std::result::Result<T, Error>;
