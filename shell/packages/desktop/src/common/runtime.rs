//! The shell's async runtime, reachable from any thread.
//!
//! The native bar starts and rebuilds on plain threads of its own; a
//! `tokio::spawn` there panics ("there is no reactor running") and the bar
//! never comes back. `main` stores the runtime's handle once; code that may
//! run outside the runtime spawns through `handle()`.

use std::sync::OnceLock;

use tokio::runtime::Handle;

static RUNTIME: OnceLock<Handle> = OnceLock::new();

/// Stores the runtime `main` runs on (later calls keep the first).
pub fn init(handle: Handle) {
  let _ = RUNTIME.set(handle);
}

/// The current thread's runtime if it is inside one, else the stored one.
pub fn handle() -> Option<Handle> {
  Handle::try_current().ok().or_else(|| RUNTIME.get().cloned())
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  #[test]
  fn a_plain_thread_reaches_the_stored_runtime() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    super::init(rt.handle().clone());
    let (tx, rx) = std::sync::mpsc::channel();
    // a std thread outside any runtime, like the bar's start and rebuilds
    let thread = std::thread::spawn(move || {
      let handle = super::handle().expect("the stored runtime");
      handle.spawn(async move {
        let _ = tx.send(true);
      });
    });
    thread.join().unwrap();
    rt.block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(true));
  }
}
