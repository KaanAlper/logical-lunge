//! EcoQoS for the shell's background workers (the providers' polls): Windows
//! runs them on efficient cores at a low clock, away from a game's threads.
//! The bar's own UI thread keeps the normal quality of service.

use windows::Win32::System::Threading::{
  GetCurrentThread, SetThreadInformation, ThreadPowerThrottling, THREAD_POWER_THROTTLING_CURRENT_VERSION,
  THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
};

/// Marks the calling thread as background work (EcoQoS). Older Windows
/// versions without it simply keep the default.
pub fn eco_qos_current_thread() {
  let state = THREAD_POWER_THROTTLING_STATE {
    Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
    ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
    StateMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
  };
  unsafe {
    let _ = SetThreadInformation(
      GetCurrentThread(),
      ThreadPowerThrottling,
      &state as *const _ as *const core::ffi::c_void,
      std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
    );
  }
}
