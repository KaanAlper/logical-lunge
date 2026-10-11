//! Logical Lunge: a look at another thread of this process while it is
//! stuck: how much processor time it has used (spinning or waiting) and
//! where it is (its call stack, as `module+offset` addresses that the
//! build's symbols resolve). Used by the window manager's stall watch, so a
//! report of a hung window manager shows where its thread was.

/// A thread that can be looked at from another thread.
pub struct ThreadSampler {
  #[cfg(target_os = "windows")]
  handle: windows::Win32::Foundation::HANDLE,
}

// The handle is a kernel handle, usable from any thread.
unsafe impl Send for ThreadSampler {}
unsafe impl Sync for ThreadSampler {}

impl ThreadSampler {
  /// The calling thread (to be looked at later from another thread).
  /// `None` where this is not supported.
  #[must_use]
  pub fn for_current_thread() -> Option<Self> {
    #[cfg(target_os = "windows")]
    {
      imp::open_current().map(|handle| Self { handle })
    }
    #[cfg(not(target_os = "windows"))]
    {
      None
    }
  }

  /// Processor time (kernel + user) the thread has used, in milliseconds.
  #[must_use]
  pub fn cpu_ms(&self) -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
      imp::cpu_ms(self.handle)
    }
    #[cfg(not(target_os = "windows"))]
    {
      None
    }
  }

  /// The thread's call stack, innermost frame first (at most `max`
  /// frames), as `module+0xoffset`. Empty when it can't be taken.
  ///
  /// The thread is paused for the moment it takes to read its stack.
  /// Nothing that could wait on the paused thread (a lock it may hold, the
  /// heap) is used meanwhile.
  #[must_use]
  pub fn stack(&self, max: usize) -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
      imp::stack(self.handle, max)
    }
    #[cfg(not(target_os = "windows"))]
    {
      let _ = max;
      Vec::new()
    }
  }
}

impl Drop for ThreadSampler {
  fn drop(&mut self) {
    #[cfg(target_os = "windows")]
    unsafe {
      let _ = windows::Win32::Foundation::CloseHandle(self.handle);
    }
  }
}

#[cfg(target_os = "windows")]
mod imp {
  use windows::Win32::{
    Foundation::{FILETIME, HANDLE, HMODULE},
    System::{
      Diagnostics::Debug::{
        GetThreadContext, RtlLookupFunctionEntry, RtlVirtualUnwind,
        CONTEXT, CONTEXT_FULL_AMD64, UNW_FLAG_NHANDLER,
      },
      LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW,
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
      },
      Threading::{
        GetCurrentThreadId, GetThreadTimes, OpenThread, ResumeThread,
        SuspendThread, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
        THREAD_SUSPEND_RESUME,
      },
    },
  };

  /// The most frames read in one sample (fixed: no allocation while the
  /// thread is paused).
  const MAX_FRAMES: usize = 64;

  /// x64 `CONTEXT` must be 16-byte aligned for `GetThreadContext`.
  #[repr(C, align(16))]
  struct AlignedContext(CONTEXT);

  pub(super) fn open_current() -> Option<HANDLE> {
    unsafe {
      OpenThread(
        THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
        false,
        GetCurrentThreadId(),
      )
    }
    .ok()
  }

  pub(super) fn cpu_ms(handle: HANDLE) -> Option<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
      FILETIME::default(),
      FILETIME::default(),
      FILETIME::default(),
      FILETIME::default(),
    );
    unsafe {
      GetThreadTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)
    }
    .ok()?;
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Some((ticks(kernel) + ticks(user)) / 10_000)
  }

  pub(super) fn stack(handle: HANDLE, max: usize) -> Vec<String> {
    let mut frames = [0u64; MAX_FRAMES];
    let count = capture(handle, &mut frames[..max.min(MAX_FRAMES)]);
    frames[..count].iter().map(|&pc| describe(pc)).collect()
  }

  /// Reads the paused thread's frames into `frames`; how many it read.
  /// Between suspend and resume only the unwinder runs (it reads the
  /// binaries' unwind tables), no allocation and no lock of ours.
  #[cfg(target_arch = "x86_64")]
  fn capture(handle: HANDLE, frames: &mut [u64]) -> usize {
    if unsafe { SuspendThread(handle) } == u32::MAX {
      return 0;
    }
    let mut context: AlignedContext = unsafe { std::mem::zeroed() };
    context.0.ContextFlags = CONTEXT_FULL_AMD64;
    let mut count = 0;
    if unsafe { GetThreadContext(handle, &mut context.0) }.is_ok() {
      while count < frames.len() {
        let pc = context.0.Rip;
        if pc == 0 {
          break;
        }
        frames[count] = pc;
        count += 1;
        let mut image_base = 0u64;
        let entry = unsafe { RtlLookupFunctionEntry(pc, &mut image_base, None) };
        if entry.is_null() {
          // A leaf function (no unwind data): the return address is on top
          // of the stack. Only the innermost frame can be one (a leaf calls
          // nothing); further down it means the walk went astray.
          let rsp = context.0.Rsp;
          if count > 1 || rsp == 0 || rsp % 8 != 0 {
            break;
          }
          context.0.Rip = unsafe { std::ptr::read(rsp as *const u64) };
          context.0.Rsp = rsp + 8;
        } else {
          let mut handler_data = std::ptr::null_mut();
          let mut establisher = 0u64;
          unsafe {
            RtlVirtualUnwind(
              UNW_FLAG_NHANDLER,
              image_base,
              pc,
              entry,
              &mut context.0,
              &mut handler_data,
              &mut establisher,
              None,
            );
          }
        }
      }
    }
    unsafe { ResumeThread(handle) };
    count
  }

  #[cfg(not(target_arch = "x86_64"))]
  fn capture(_handle: HANDLE, _frames: &mut [u64]) -> usize {
    0
  }

  /// `lunge-tiling.exe+0x1a2b3c` (or the bare address outside any module).
  fn describe(pc: u64) -> String {
    let mut module = HMODULE::default();
    let found = unsafe {
      GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
          | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        windows::core::PCWSTR(pc as *const u16),
        &mut module,
      )
    }
    .is_ok();
    if !found {
      return format!("{pc:#x}");
    }
    let mut path = [0u16; 260];
    let len = unsafe { GetModuleFileNameW(module, &mut path) } as usize;
    let full = String::from_utf16_lossy(&path[..len]);
    let name = full.rsplit(['\\', '/']).next().unwrap_or(&full);
    #[allow(clippy::cast_sign_loss)]
    let base = module.0 as u64;
    format!("{name}+{:#x}", pc - base)
  }
}
