use std::cell::RefCell;

use windows::{
  core::{ComInterface, IUnknown, IUnknown_Vtbl, GUID, HRESULT},
  Win32::{
    System::Com::{
      CoCreateInstance, CoInitializeEx, CoUninitialize, IServiceProvider,
      CLSCTX_ALL, CLSCTX_SERVER, COINIT_APARTMENTTHREADED,
    },
    UI::Shell::{ITaskbarList2, TaskbarList},
  },
};

/// COM class identifier (CLSID) for the Windows Shell that implements the
/// `IServiceProvider` interface.
const CLSID_IMMERSIVE_SHELL: GUID =
  GUID::from_u128(0xC2F03A33_21F5_47FA_B4BB_156362A2F239);

thread_local! {
  /// Manages per-thread COM initialization. COM must be initialized on each
  /// thread that uses it, so we store this in thread-local storage to handle
  /// the setup and cleanup automatically.
  ///
  /// Wrapped in `RefCell` to allow mutation via `COM_INIT.borrow_mut()`.
  pub(crate) static COM_INIT: RefCell<ComInit> = RefCell::new(ComInit::new());
}

pub(crate) struct ComInit {
  service_provider: Option<IServiceProvider>,
  application_view_collection: Option<IApplicationViewCollection>,
  taskbar_list: Option<ITaskbarList2>,
  /// Logical Lunge: when the interfaces were last re-created (see
  /// `with_retry`).
  last_refresh: Option<std::time::Instant>,
}

impl ComInit {
  /// Initializes COM on the current thread with apartment threading model.
  /// `COINIT_APARTMENTTHREADED` is required for shell COM objects.
  ///
  /// # Panics
  ///
  /// Panics if COM initialization fails. This is typically only possible
  /// if COM is already initialized with an incompatible threading model.
  #[must_use]
  pub(crate) fn new() -> Self {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
      .expect("Unable to initialize COM.");

    let service_provider = unsafe {
      CoCreateInstance(&CLSID_IMMERSIVE_SHELL, None, CLSCTX_ALL)
    }
    .ok();

    let application_view_collection = service_provider.as_ref().and_then(
      |provider: &IServiceProvider| unsafe {
        provider.QueryService(&IApplicationViewCollection::IID).ok()
      },
    );

    let taskbar_list =
      unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_SERVER) }.ok();

    Self {
      service_provider,
      application_view_collection,
      taskbar_list,
      last_refresh: None,
    }
  }

  /// Returns an instance of `IApplicationViewCollection`.
  pub(crate) fn application_view_collection(
    &self,
  ) -> crate::Result<&IApplicationViewCollection> {
    self.application_view_collection.as_ref().ok_or_else(|| {
      crate::Error::Platform(
        "Failed to query for `IApplicationViewCollection` instance."
          .to_string(),
      )
    })
  }

  /// Returns an instance of `ITaskbarList2`.
  pub(crate) fn taskbar_list(&self) -> crate::Result<&ITaskbarList2> {
    self.taskbar_list.as_ref().ok_or_else(|| {
      crate::Error::Platform(
        "Unable to create `ITaskbarList2` instance.".to_string(),
      )
    })
  }

  /// Refreshes cached COM interfaces.
  ///
  /// Called automatically by `with_retry` when COM operations fail due to
  /// stale interface pointers (e.g. after Explorer restarts).
  pub(crate) fn refresh(&mut self) {
    // Re-create the service provider.
    self.service_provider = unsafe {
      CoCreateInstance(&CLSID_IMMERSIVE_SHELL, None, CLSCTX_ALL)
    }
    .ok();

    // Re-create the application view collection.
    self.application_view_collection = self
      .service_provider
      .as_ref()
      .and_then(|provider: &IServiceProvider| unsafe {
        provider.QueryService(&IApplicationViewCollection::IID).ok()
      });

    // Re-create the taskbar list.
    self.taskbar_list =
      unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_SERVER) }.ok();
  }

  /// Executes a COM operation, refreshing interfaces on failure and
  /// retrying once. Use this for operations that may fail due to stale
  /// COM interfaces.
  ///
  /// Logical Lunge: only an interface that is missing (it could not be
  /// created) or an error that says its server is gone (Explorer restarted)
  /// re-creates them, at most once per `REFRESH_INTERVAL`. A window that has no
  /// application view (a new one, or one that just closed: "Element not
  /// found") failed every cloak and re-created all three interfaces each
  /// time, a cross-process activation in Explorer per window that left
  /// Windows' thread pool with more and more workers.
  pub fn with_retry<T, F>(&mut self, op: F) -> crate::Result<T>
  where
    F: Fn(&Self) -> crate::Result<T>,
  {
    match op(self) {
      Ok(result) => Ok(result),
      Err(err)
        if (self.application_view_collection.is_none()
          || self.taskbar_list.is_none()
          || is_stale_interface_error(&err))
          && self.last_refresh.is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL) =>
      {
        self.refresh();
        self.last_refresh = Some(std::time::Instant::now());
        op(self)
      }
      Err(err) => Err(err),
    }
  }
}

/// Logical Lunge: the shortest time between two re-creations of the COM
/// interfaces on one thread.
const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Logical Lunge: whether an error means the cached interfaces' server went
/// away, as opposed to an error about the window the call was for.
fn is_stale_interface_error(err: &crate::Error) -> bool {
  matches!(err, crate::Error::Windows(err) if is_stale_hresult(err.code().0))
}

#[allow(clippy::cast_possible_wrap)]
fn is_stale_hresult(code: i32) -> bool {
  [
    0x8001_0108_u32, // RPC_E_DISCONNECTED
    0x8001_0007,     // RPC_E_SERVER_DIED
    0x8001_0012,     // RPC_E_SERVER_DIED_DNE
    0x8007_06BA,     // RPC_S_SERVER_UNAVAILABLE
    0x8007_06BE,     // RPC_S_CALL_FAILED
    0x8004_01FD,     // CO_E_OBJNOTCONNECTED
  ]
  .contains(&(code as u32))
}

impl Default for ComInit {
  fn default() -> Self {
    Self::new()
  }
}

impl Drop for ComInit {
  fn drop(&mut self) {
    // Explicitly drop COM interfaces first.
    drop(self.taskbar_list.take());
    drop(self.application_view_collection.take());
    drop(self.service_provider.take());

    unsafe { CoUninitialize() };
  }
}

/// Undocumented COM interface for Windows shell functionality.
///
/// Note that filler methods are added to match the vtable layout.
#[windows_interface::interface("1841c6d7-4f9d-42c0-af41-8747538f10e5")]
pub unsafe trait IApplicationViewCollection: IUnknown {
  pub unsafe fn m1(&self);
  pub unsafe fn m2(&self);
  pub unsafe fn m3(&self);
  pub unsafe fn get_view_for_hwnd(
    &self,
    window: isize,
    application_view: *mut Option<IApplicationView>,
  ) -> HRESULT;
}

/// Undocumented COM interface for managing views in the Windows shell.
///
/// Note that filler methods are added to match the vtable layout.
#[windows_interface::interface("372E1D3B-38D3-42E4-A15B-8AB2B178F513")]
pub unsafe trait IApplicationView: IUnknown {
  pub unsafe fn m1(&self);
  pub unsafe fn m2(&self);
  pub unsafe fn m3(&self);
  pub unsafe fn m4(&self);
  pub unsafe fn m5(&self);
  pub unsafe fn m6(&self);
  pub unsafe fn m7(&self);
  pub unsafe fn m8(&self);
  pub unsafe fn m9(&self);
  pub unsafe fn set_cloak(
    &self,
    cloak_type: u32,
    cloak_flag: i32,
  ) -> HRESULT;
}

#[cfg(test)]
mod tests {
  use super::is_stale_hresult;

  #[test]
  fn only_a_gone_server_re_creates_the_interfaces() {
    // RPC_E_DISCONNECTED, RPC_S_SERVER_UNAVAILABLE
    assert!(is_stale_hresult(0x8001_0108_u32 as i32));
    assert!(is_stale_hresult(0x8007_06BA_u32 as i32));
    // a window without an application view: ERROR_NOT_FOUND, E_INVALIDARG,
    // an invalid window handle
    assert!(!is_stale_hresult(0x8007_0490_u32 as i32));
    assert!(!is_stale_hresult(0x8007_0057_u32 as i32));
    assert!(!is_stale_hresult(0x8007_0578_u32 as i32));
  }
}
