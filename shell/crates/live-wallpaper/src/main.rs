//! Logical Lunge live wallpaper: videos behind the desktop icons, one
//! window per monitor, decoded on the GPU by Media Foundation. The core
//! starts it when a live wallpaper is set (state\live-wallpaper.json) and
//! tells it to reload or stop through its window; it pauses while a
//! fullscreen app covers a monitor, the screen is locked or off,
//! or the computer runs on battery.
//! `lunge-wallpaper --frame <video> <png>` saves one frame as a picture
//! (the static wallpaper under the live one). Copied as LogicalLunge.scr
//! it is the video screen saver (saver.rs).

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod config;
mod fit;
mod log;

#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod frame;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod saver;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn main() {
  use windows::{
    core::w,
    Win32::{
      Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0},
      System::Threading::{
        CreateMutexW, ReleaseMutex, WaitForSingleObject,
      },
      UI::HiDpi::{
        SetProcessDpiAwarenessContext,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
      },
    },
  };

  let args: Vec<String> = std::env::args().collect();
  // LogicalLunge.scr (this program under the screen saver's name)
  if saver::wanted(&args) {
    saver::main(&args);
    return;
  }
  if args.len() == 4 && args[1] == "--frame" {
    let code = match frame::save(
      std::path::Path::new(&args[2]),
      std::path::Path::new(&args[3]),
    ) {
      Ok(()) => 0,
      Err(err) => {
        log::line(&format!("no frame from {}: {err:?}", args[2]));
        1
      }
    };
    std::process::exit(code);
  }
  unsafe {
    // One at a time: the core starts it whenever a live wallpaper is set.
    // A player that is just closing (told to stop a moment before a new
    // wallpaper came) is waited for; a running one keeps the wallpaper.
    let Ok(mutex) =
      CreateMutexW(None, false, w!("LogicalLunge.LiveWallpaper"))
    else {
      return;
    };
    let got = WaitForSingleObject(mutex, 5000);
    if got != WAIT_OBJECT_0 && got != WAIT_ABANDONED {
      return;
    }
    // the manifest says so too; this covers a build without it
    let _ = SetProcessDpiAwarenessContext(
      DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    );
    ui::run();
    let _ = ReleaseMutex(mutex);
  }
}

#[cfg(not(windows))]
fn main() {}
