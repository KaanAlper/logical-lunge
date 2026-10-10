//! Win32 declarations, the hook thread and its two hook procedures.
//!
//! Declared by hand (no `windows` crate features to keep in step with each
//! edition's workspace): the library only needs a handful of calls.

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
pub fn start_thread() -> i32 {
  0
}
#[cfg(not(windows))]
pub fn signal(_event: isize) {}
#[cfg(not(windows))]
pub fn request_reinstall(_which: u32) {}

#[cfg(windows)]
#[allow(non_snake_case, clippy::upper_case_acronyms)]
pub mod imp {
  use std::cell::UnsafeCell;
  use std::panic::{catch_unwind, AssertUnwindSafe};
  use std::sync::atomic::{AtomicI64, AtomicIsize, AtomicU32, AtomicU64, Ordering};

  use crate::inject::{Op, Queue};
  use crate::keys::{self, KeyState};
  use crate::mouse::{self, MouseState};
  use crate::sys::Sys;
  use crate::{kind, ring, table, Ev, FORGET, LAST_KEY_TICK, LAST_MOUSE_TICK};

  #[repr(C)]
  #[derive(Clone, Copy, Default)]
  pub struct POINT {
    pub x: i32,
    pub y: i32,
  }

  #[repr(C)]
  #[derive(Clone, Copy, Default)]
  pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
  }

  #[repr(C)]
  pub struct KBDLLHOOKSTRUCT {
    pub vk: u32,
    pub scan: u32,
    pub flags: u32,
    pub time: u32,
    pub extra: usize,
  }

  #[repr(C)]
  pub struct MSLLHOOKSTRUCT {
    pub pt: POINT,
    pub data: u32,
    pub flags: u32,
    pub time: u32,
    pub extra: usize,
  }

  #[repr(C)]
  #[derive(Default)]
  pub struct MSG {
    pub hwnd: isize,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub pt: POINT,
    pub private: u32,
  }

  #[repr(C)]
  #[derive(Default)]
  pub struct GUITHREADINFO {
    pub size: u32,
    pub flags: u32,
    pub active: isize,
    pub focus: isize,
    pub capture: isize,
    pub menu_owner: isize,
    pub move_size: isize,
    pub caret: isize,
    pub caret_rect: RECT,
  }

  #[repr(C)]
  #[derive(Default)]
  struct MODULEINFO {
    base: usize,
    size: u32,
    entry: usize,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  struct MOUSEINPUT {
    dx: i32,
    dy: i32,
    data: u32,
    flags: u32,
    time: u32,
    extra: usize,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  struct KEYBDINPUT {
    vk: u16,
    scan: u16,
    flags: u32,
    time: u32,
    extra: usize,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  union INPUT_U {
    mi: MOUSEINPUT,
    ki: KEYBDINPUT,
  }

  #[repr(C)]
  #[derive(Clone, Copy)]
  struct INPUT {
    kind: u32,
    u: INPUT_U,
  }

  #[cfg(target_pointer_width = "64")]
  const _: () = assert!(std::mem::size_of::<INPUT>() == 40);

  #[link(name = "user32")]
  extern "system" {
    fn SetWindowsHookExW(id: i32, proc_: usize, module: isize, thread: u32) -> isize;
    fn UnhookWindowsHookEx(hook: isize) -> i32;
    fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
    fn GetMessageW(msg: *mut MSG, hwnd: isize, min: u32, max: u32) -> i32;
    fn PeekMessageW(msg: *mut MSG, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn PostThreadMessageW(thread: u32, msg: u32, wparam: usize, lparam: isize) -> i32;
    pub fn GetAsyncKeyState(vk: i32) -> i16;
    fn SendInput(count: u32, inputs: *const INPUT, size: i32) -> u32;
    pub fn GetForegroundWindow() -> isize;
    pub fn GetAncestor(hwnd: isize, flags: u32) -> isize;
    pub fn GetClassNameW(hwnd: isize, name: *mut u16, max: i32) -> i32;
    pub fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
    pub fn GetGUIThreadInfo(thread: u32, info: *mut GUITHREADINFO) -> i32;
    pub fn WindowFromPoint(pt: POINT) -> isize;
    pub fn GetDoubleClickTime() -> u32;
    pub fn GetSystemMetrics(index: i32) -> i32;
    fn InternalGetWindowText(hwnd: isize, text: *mut u16, max: i32) -> i32;
  }

  #[link(name = "shell32")]
  extern "system" {
    fn SHQueryUserNotificationState(state: *mut i32) -> i32;
  }

  #[link(name = "kernel32")]
  extern "system" {
    fn CreateEventW(attrs: usize, manual: i32, initial: i32, name: usize) -> isize;
    fn SetEvent(event: isize) -> i32;
    fn WaitForSingleObject(handle: isize, ms: u32) -> u32;
    fn GetCurrentThread() -> isize;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcess() -> isize;
    fn SetThreadPriority(thread: isize, priority: i32) -> i32;
    fn GetModuleHandleExW(flags: u32, name: usize, module: *mut isize) -> i32;
    fn K32GetModuleInformation(process: isize, module: isize, info: *mut MODULEINFO, size: u32) -> i32;
    fn VirtualLock(address: usize, size: usize) -> i32;
    fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
    pub fn GetTickCount() -> u32;
  }

  /// The core's mark on keys and clicks it injects itself ("LLK1").
  pub const LL_MARK: usize = 0x4C4C_4B31;

  const WH_KEYBOARD_LL: i32 = 13;
  const WH_MOUSE_LL: i32 = 14;
  const WM_APP_REINSTALL: u32 = 0x8000 + 1;
  const THREAD_PRIORITY_TIME_CRITICAL: i32 = 15;
  /// Windows removes a hook that answers slower than LowLevelHooksTimeout
  /// (300 ms by default): anything past this is reported to the core's log
  const SLOW_MS: u64 = 100;

  /// Hook-thread-only state: the hook procedures run one at a time on the
  /// one thread that installed them.
  struct HookCell<T>(UnsafeCell<T>);
  unsafe impl<T> Sync for HookCell<T> {}
  static KEYS: HookCell<KeyState> = HookCell(UnsafeCell::new(KeyState::new()));
  static MOUSE: HookCell<MouseState> = HookCell(UnsafeCell::new(MouseState::new()));

  /// Hook timings for the core's log: calls, total and longest (µs), per
  /// hook; read and reset by `li_stats`.
  pub struct Stats {
    calls: AtomicU64,
    total_us: AtomicU64,
    max_us: AtomicU64,
  }
  pub static KEY_STATS: Stats = Stats { calls: AtomicU64::new(0), total_us: AtomicU64::new(0), max_us: AtomicU64::new(0) };
  pub static MOUSE_STATS: Stats = Stats { calls: AtomicU64::new(0), total_us: AtomicU64::new(0), max_us: AtomicU64::new(0) };

  impl Stats {
    fn add(&self, us: u64) {
      self.calls.fetch_add(1, Ordering::Relaxed);
      self.total_us.fetch_add(us, Ordering::Relaxed);
      self.max_us.fetch_max(us, Ordering::Relaxed);
    }

    pub fn take(&self) -> (u64, u64, u64) {
      (self.calls.swap(0, Ordering::Relaxed), self.total_us.swap(0, Ordering::Relaxed), self.max_us.swap(0, Ordering::Relaxed))
    }
  }

  static THREAD_ID: AtomicU32 = AtomicU32::new(0);
  static KEY_HOOK: AtomicIsize = AtomicIsize::new(0);
  static MOUSE_HOOK: AtomicIsize = AtomicIsize::new(0);
  static FREQ: AtomicI64 = AtomicI64::new(0);

  /// What the hook procedures send back to Windows, sent by the injector
  /// thread once they returned (see `inject`).
  static INJECT: Queue = Queue::new();
  static INJECT_EVENT: AtomicIsize = AtomicIsize::new(0);
  const THREAD_PRIORITY_HIGHEST: i32 = 2;
  const INFINITE: u32 = 0xFFFF_FFFF;

  /// Hook thread: queues `ops` (in order) and wakes the injector. Nothing
  /// is sent without the injector (its thread could not start).
  fn inject(ops: &[Op]) {
    let event = INJECT_EVENT.load(Ordering::Acquire);
    if event == 0 {
      return;
    }
    for op in ops {
      if !INJECT.push(*op) {
        break;
      }
    }
    unsafe { SetEvent(event) };
  }

  fn key_input(vk: u8, flags: u32) -> INPUT {
    INPUT { kind: 1, u: INPUT_U { ki: KEYBDINPUT { vk: vk as u16, scan: 0, flags, time: 0, extra: LL_MARK } } }
  }

  fn mouse_input(flags: u32, dx: i32, dy: i32) -> INPUT {
    INPUT { kind: 0, u: INPUT_U { mi: MOUSEINPUT { dx, dy, data: 0, flags, time: 0, extra: LL_MARK } } }
  }

  /// A screen point in SendInput's absolute coordinates (0..65535 over the
  /// virtual desktop).
  fn absolute(x: i32, y: i32) -> (i32, i32) {
    unsafe {
      let (vx, vy) = (GetSystemMetrics(76), GetSystemMetrics(77));
      let (vw, vh) = (GetSystemMetrics(78).max(2), GetSystemMetrics(79).max(2));
      (((x - vx) as i64 * 65535 / (vw - 1) as i64) as i32, ((y - vy) as i64 * 65535 / (vh - 1) as i64) as i32)
    }
  }

  fn move_to(x: i32, y: i32) -> INPUT {
    let (nx, ny) = absolute(x, y);
    mouse_input(0x0001 | 0x8000 | 0x4000, nx, ny) // MOVE | ABSOLUTE | VIRTUALDESK
  }

  /// Injector thread: sends what the hooks queued, everything pending in one
  /// SendInput (nothing else comes in between).
  fn run_injector(event: isize) {
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) };
    let mut batch: Vec<INPUT> = Vec::with_capacity(crate::inject::CAPACITY * 2);
    loop {
      unsafe { WaitForSingleObject(event, INFINITE) };
      batch.clear();
      while let Some(op) = INJECT.pop() {
        match op {
          Op::Dummy => {
            batch.push(key_input(0xE8, 0));
            batch.push(key_input(0xE8, 0x2)); // KEYUP
          }
          Op::Release(vk, extended) => batch.push(key_input(vk, 0x2 | extended as u32)), // KEYUP | EXTENDEDKEY
          Op::LeftDown(x, y) => {
            batch.push(move_to(x, y));
            batch.push(mouse_input(0x0002, 0, 0)); // LEFTDOWN
          }
          Op::MoveTo(x, y) => batch.push(move_to(x, y)),
        }
      }
      if !batch.is_empty() {
        unsafe { SendInput(batch.len() as u32, batch.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
      }
    }
  }

  fn start_injector() {
    if INJECT_EVENT.load(Ordering::Acquire) != 0 {
      return;
    }
    let event = unsafe { CreateEventW(0, 0, 0, 0) };
    if event == 0 {
      return;
    }
    let spawned = std::thread::Builder::new().name("ll-inject".into()).stack_size(64 * 1024).spawn(move || run_injector(event));
    if spawned.is_ok() {
      INJECT_EVENT.store(event, Ordering::Release);
    }
  }

  pub fn signal(event: isize) {
    if event != 0 {
      unsafe { SetEvent(event) };
    }
  }

  pub fn down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
  }

  fn now() -> i64 {
    let mut t = 0;
    unsafe { QueryPerformanceCounter(&mut t) };
    t
  }

  fn elapsed_us(start: i64) -> u64 {
    let f = FREQ.load(Ordering::Relaxed).max(1);
    ((now() - start).max(0) as u64).saturating_mul(1_000_000) / f as u64
  }

  fn module() -> isize {
    let mut h = 0;
    // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | UNCHANGED_REFCOUNT
    unsafe { GetModuleHandleExW(0x4 | 0x2, key_proc as *const () as usize, &mut h) };
    h
  }

  pub fn start_thread() -> i32 {
    if THREAD_ID.load(Ordering::Acquire) != 0 {
      return (KEY_HOOK.load(Ordering::Acquire) != 0 && MOUSE_HOOK.load(Ordering::Acquire) != 0) as i32;
    }
    unsafe {
      let mut f = 0;
      QueryPerformanceFrequency(&mut f);
      FREQ.store(f, Ordering::Relaxed);
      if ring::event() == 0 {
        ring::set_event(CreateEventW(0, 0, 0, 0));
      }
      start_injector();
      let ready = CreateEventW(0, 1, 0, 0);
      let spawned = std::thread::Builder::new()
        .name("ll-input".into())
        .stack_size(256 * 1024)
        .spawn(move || run(ready));
      if spawned.is_err() {
        return 0;
      }
      WaitForSingleObject(ready, 3000);
    }
    (KEY_HOOK.load(Ordering::Acquire) != 0 && MOUSE_HOOK.load(Ordering::Acquire) != 0) as i32
  }

  pub fn request_reinstall(which: u32) {
    let thread = THREAD_ID.load(Ordering::Acquire);
    if thread != 0 {
      unsafe { PostThreadMessageW(thread, WM_APP_REINSTALL, which as usize, 0) };
    }
  }

  fn install(which: usize) {
    let hmod = module();
    unsafe {
      if which & 1 != 0 {
        let fresh = SetWindowsHookExW(WH_KEYBOARD_LL, key_proc as *const () as usize, hmod, 0);
        if fresh != 0 {
          let old = KEY_HOOK.swap(fresh, Ordering::AcqRel);
          if old != 0 {
            UnhookWindowsHookEx(old);
          }
        }
      }
      if which & 2 != 0 {
        let fresh = SetWindowsHookExW(WH_MOUSE_LL, mouse_proc as *const () as usize, hmod, 0);
        if fresh != 0 {
          let old = MOUSE_HOOK.swap(fresh, Ordering::AcqRel);
          if old != 0 {
            UnhookWindowsHookEx(old);
          }
        }
      }
    }
  }

  /// The module image (code, the ring, the tables) and the hook thread's
  /// stack stay in memory: a page fault under memory pressure must not delay
  /// a hook. (The core sets the process' working set floor this needs.)
  fn lock_memory() {
    unsafe {
      let hmod = module();
      let mut info = MODULEINFO::default();
      if hmod != 0 && K32GetModuleInformation(GetCurrentProcess(), hmod, &mut info, std::mem::size_of::<MODULEINFO>() as u32) != 0 {
        VirtualLock(info.base, info.size as usize);
      }
      let (ring_at, ring_size) = ring::storage();
      VirtualLock(ring_at as usize, ring_size);
      // the stack the hook procedures run on: committed first (the pages
      // below the current frame are only reserved), then locked
      touch_stack();
      let (mut low, mut high) = (0usize, 0usize);
      GetCurrentThreadStackLimits(&mut low, &mut high);
      let from = ((&low as *const usize as usize) & !0xFFF).saturating_sub(STACK_LOCKED);
      if from > low && high > from {
        VirtualLock(from, high - from);
      }
    }
  }

  /// How much stack below the message loop's frame stays locked: the hook
  /// procedures need a few kB, Windows' dispatch above them a few more.
  const STACK_LOCKED: usize = 64 * 1024;

  #[inline(never)]
  fn touch_stack() {
    let mut probe = [0u8; STACK_LOCKED + 4096];
    std::hint::black_box(&mut probe);
  }

  fn run(ready: isize) {
    unsafe {
      SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
      let mut msg = MSG::default();
      PeekMessageW(&mut msg, 0, 0, 0, 0); // the thread's message queue
      THREAD_ID.store(GetCurrentThreadId(), Ordering::Release);
      lock_memory();
      install(3);
      // both or neither: the core falls back to its own hooks otherwise
      if KEY_HOOK.load(Ordering::Acquire) == 0 || MOUSE_HOOK.load(Ordering::Acquire) == 0 {
        for hook in [&KEY_HOOK, &MOUSE_HOOK] {
          let h = hook.swap(0, Ordering::AcqRel);
          if h != 0 {
            UnhookWindowsHookEx(h);
          }
        }
      }
      SetEvent(ready);
      while GetMessageW(&mut msg, 0, 0, 0) > 0 {
        if msg.hwnd == 0 && msg.message == WM_APP_REINSTALL {
          // a held Win keeps its hook (its state would mix) unless Windows
          // removed it (force): then the held keys are stale too
          let force = msg.wparam & 4 != 0;
          let keys = &mut *KEYS.0.get();
          if force && msg.wparam & 1 != 0 {
            keys.forget();
          }
          let which = if keys.win_down() && !force { msg.wparam & 2 } else { msg.wparam & 3 };
          install(which);
          continue;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
      }
    }
  }

  unsafe extern "system" fn key_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    LAST_KEY_TICK.store(GetTickCount(), Ordering::Relaxed);
    if code < 0 || lparam == 0 {
      return CallNextHookEx(0, code, wparam, lparam);
    }
    let start = now();
    let k = &*(lparam as *const KBDLLHOOKSTRUCT);
    let keys = &mut *KEYS.0.get();
    if FORGET.swap(false, Ordering::AcqRel) {
      keys.forget();
    }
    let ours = k.flags & 0x10 != 0 && k.extra == LL_MARK; // LLKHF_INJECTED with our mark
    // a panic must not cross into Windows: the key goes on unhandled
    let swallow = catch_unwind(AssertUnwindSafe(|| keys::decide(&Win, keys, wparam as u32, k.vk, k.flags, ours))).unwrap_or(false);
    let us = elapsed_us(start);
    KEY_STATS.add(us);
    if us > SLOW_MS * 1000 {
      ring::push(Ev { kind: kind::SLOW_KEY, vk: k.vk as u16, id: (us / 1000) as i32, ..Ev::default() });
    }
    if swallow {
      1
    } else {
      CallNextHookEx(0, code, wparam, lparam)
    }
  }

  unsafe extern "system" fn mouse_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    LAST_MOUSE_TICK.store(GetTickCount(), Ordering::Relaxed);
    if code < 0 || lparam == 0 {
      return CallNextHookEx(0, code, wparam, lparam);
    }
    let start = now();
    let m = &*(lparam as *const MSLLHOOKSTRUCT);
    let st = &mut *MOUSE.0.get();
    let injected = m.flags & 1 != 0; // LLMHF_INJECTED
    let swallow =
      catch_unwind(AssertUnwindSafe(|| mouse::decide(&Win, st, wparam as u32, m.pt.x, m.pt.y, m.time, injected, m.data))).unwrap_or(false);
    let us = elapsed_us(start);
    MOUSE_STATS.add(us);
    if us > SLOW_MS * 1000 {
      ring::push(Ev { kind: kind::SLOW_MOUSE, id: (us / 1000) as i32, ..Ev::default() });
    }
    if swallow {
      1
    } else {
      CallNextHookEx(0, code, wparam, lparam)
    }
  }

  /// The root window's class is the desktop's (the icons' host).
  pub fn is_desktop_window(hwnd: isize) -> bool {
    if hwnd == 0 {
      return false;
    }
    let root = unsafe { GetAncestor(hwnd, 2) }; // GA_ROOT
    root != 0 && (class_is(root, "Progman") || class_is(root, "WorkerW"))
  }

  pub fn class_is(hwnd: isize, name: &str) -> bool {
    let mut buf = [0u16; 16];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    n > 0 && buf[..n as usize].iter().copied().eq(name.encode_utf16())
  }

  /// The desktop's keyboard focus is in an edit box (an icon being renamed).
  pub fn focus_is_edit(foreground: isize) -> bool {
    let mut info = GUITHREADINFO { size: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    let thread = unsafe { GetWindowThreadProcessId(foreground, std::ptr::null_mut()) };
    let got = unsafe { GetGUIThreadInfo(thread, &mut info) != 0 };
    got && info.focus != 0 && class_is(info.focus, "Edit")
  }

  fn root_class_is_desktop(root: isize) -> bool {
    root != 0 && (class_is(root, "Progman") || class_is(root, "WorkerW"))
  }

  /// The core's own window titles (Names.TitlePrefix, "lunge-...").
  fn title_is_shell(hwnd: isize) -> bool {
    let mut buf = [0u16; 32];
    // InternalGetWindowText never sends a message (GetWindowText would wait
    // on a window of the core's own, busy UI thread)
    let n = unsafe { InternalGetWindowText(hwnd, buf.as_mut_ptr(), buf.len() as i32) }.max(0) as usize;
    let title = &buf[..n.min(buf.len())];
    let starts = |p: &str| {
      let mut it = title.iter();
      p.encode_utf16().all(|c| it.next() == Some(&c))
    };
    starts("Logical Lunge \u{b7}") || starts("lunge-")
  }

  /// Windows, from the hook thread.
  pub struct Win;

  impl Sys for Win {
    fn down(&self, vk: u32) -> bool {
      down(vk as i32)
    }
    fn now_ms(&self) -> u32 {
      unsafe { GetTickCount() }
    }
    fn flag(&self, id: u32) -> bool {
      crate::flag(id)
    }
    fn set_flag(&self, id: u32, on: bool) {
      crate::FLAGS[id as usize].store(on as i32, Ordering::Relaxed);
    }
    fn lookup(&self, kind: u8, mods: u16, vk: u32) -> Option<(i32, u8)> {
      if vk > 0xFF {
        return None;
      }
      table::lookup(kind, mods as u8, vk as u8)
    }
    fn capturable(&self, vk: u32) -> bool {
      table::capturable(vk)
    }
    fn push(&self, ev: Ev) {
      ring::push(Ev { time: unsafe { GetTickCount() }, ..ev });
    }
    fn desktop_in_front(&self) -> bool {
      is_desktop_window(unsafe { GetForegroundWindow() })
    }
    fn desktop_focus_is_edit(&self) -> bool {
      focus_is_edit(unsafe { GetForegroundWindow() })
    }
    fn foreground_is_shell(&self) -> bool {
      let fg = unsafe { GetAncestor(GetForegroundWindow(), 2) };
      fg == 0 || root_class_is_desktop(fg) || class_is(fg, "Shell_TrayWnd") || title_is_shell(fg)
    }
    fn exclusive_fullscreen(&self) -> bool {
      let mut st = 0;
      unsafe { SHQueryUserNotificationState(&mut st) == 0 && st == 3 } // QUNS_RUNNING_D3D_FULL_SCREEN
    }
    fn desktop_at(&self, x: i32, y: i32) -> bool {
      is_desktop_window(unsafe { WindowFromPoint(POINT { x, y }) })
    }
    fn edit_at(&self, x: i32, y: i32) -> bool {
      let h = unsafe { WindowFromPoint(POINT { x, y }) };
      h != 0 && class_is(h, "Edit")
    }
    fn double_click(&self, t0: u32, x0: i32, y0: i32, t1: u32, x1: i32, y1: i32) -> bool {
      unsafe {
        t1.wrapping_sub(t0) <= GetDoubleClickTime()
          && (x1 - x0).abs() * 2 <= GetSystemMetrics(36)
          && (y1 - y0).abs() * 2 <= GetSystemMetrics(37)
      }
    }
    fn dragged(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
      unsafe { (x1 - x0).abs() * 2 > GetSystemMetrics(68) || (y1 - y0).abs() * 2 > GetSystemMetrics(69) }
    }
    // queued: sent by the injector after the hook procedure returned
    fn suppress_start(&self) {
      inject(&[Op::Dummy]);
    }
    fn release_key(&self, vk: u32, extended: bool) {
      inject(&[Op::Release(vk as u8, extended)]);
    }
    fn replay_left_down(&self, x: i32, y: i32, to_x: i32, to_y: i32) {
      inject(&[Op::LeftDown(x, y), Op::MoveTo(to_x, to_y)]);
    }
    // both hooks run on this thread: the keyboard hook's state is read here
    fn win_held(&self) -> bool {
      unsafe { (*KEYS.0.get()).win_down() }
    }
    fn win_combo(&self) {
      unsafe { (*KEYS.0.get()).mark_combo() }
    }
  }
}
