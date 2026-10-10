//! Logical Lunge: the dim behind Hyprland's open special workspace
//! (`decoration:dim_special`, 0.2 in illogical-impulse). One click-through
//! black window at that opacity over the monitor's workspace area, just
//! under the special workspace's windows; it fades in on
//! `specialWorkspaceIn` (280 ms) and out on `specialWorkspaceOut` (120 ms).
//! It lives on a thread of its own: the window manager only hands over
//! where it goes.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use log::{error, warn};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, BLACK_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW,
    RegisterClassExW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
    HWND_TOPMOST, LWA_ALPHA, MSG, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SW_HIDE, WM_APP, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// Where the dim goes: the area (left, top, right, bottom) and the windows
/// that stay above it. `None` hides it.
pub type Target = Option<((i32, i32, i32, i32), Vec<isize>)>;

/// Hyprland's `dim_special` in ii.
const STRENGTH: f32 = 0.2;
const FADE_IN_MS: f32 = 280.0;
const FADE_OUT_MS: f32 = 120.0;
const WM_APP_TARGET: u32 = WM_APP + 0x51;
const FADE_TIMER: usize = 1;

static PENDING: Mutex<Option<Target>> = Mutex::new(None);
static THREAD_ID: OnceLock<u32> = OnceLock::new();

struct Fade {
    /// the window (as a number: the state is shared with the timer)
    hwnd: isize,
    from: f32,
    to: f32,
    started: Instant,
    ms: f32,
    shown: bool,
}

/// Fade state (the backdrop thread only).
static FADE: Mutex<Option<Fade>> = Mutex::new(None);

/// Shows the dim at `target` (or hides it). Returns at once.
pub fn set_special_backdrop(target: Target) {
    crate::guarded("set_special_backdrop", (), || {
        *PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(target);
        let thread_id = *THREAD_ID.get_or_init(spawn);
        if thread_id != 0 {
            unsafe {
                if let Err(err) = PostThreadMessageW(thread_id, WM_APP_TARGET, WPARAM(0), LPARAM(0)) {
                    warn!("special backdrop: {err}");
                }
            }
        }
    });
}

fn spawn() -> u32 {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("special-backdrop".into()).spawn(move || {
        let hwnd = create_window();
        let _ = tx.send(unsafe { GetCurrentThreadId() });
        let Some(hwnd) = hwnd else { return };
        run(hwnd);
    });
    if let Err(err) = spawned {
        error!("special backdrop: could not start: {err}");
        return 0;
    }
    rx.recv().unwrap_or(0)
}

fn create_window() -> Option<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: w!("LogicalLungeSpecialDim"),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("LogicalLungeSpecialDim"),
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        );
        match hwnd {
            Ok(hwnd) => {
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
                Some(hwnd)
            }
            Err(err) => {
                error!("special backdrop: could not create its window: {err}");
                None
            }
        }
    }
}

fn run(hwnd: HWND) {
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if message.hwnd.is_invalid() && message.message == WM_APP_TARGET {
                let target = PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
                if let Some(target) = target {
                    apply(hwnd, target);
                }
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn current_alpha() -> f32 {
    FADE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map_or(0.0, |fade| fade.value())
}

fn apply(hwnd: HWND, target: Target) {
    let from = current_alpha();
    unsafe {
        match target {
            Some(((left, top, right, bottom), above)) => {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    left,
                    top,
                    right - left,
                    bottom - top,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                // the special workspace's windows go back above it (they
                // are topmost too; the last one raised is on top)
                for window in above {
                    let _ = SetWindowPos(
                        HWND(window as _),
                        Some(HWND_TOPMOST),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                    );
                }
                start_fade(hwnd, from, STRENGTH, FADE_IN_MS, true);
            }
            None => start_fade(hwnd, from, 0.0, FADE_OUT_MS, false),
        }
    }
}

impl Fade {
    fn value(&self) -> f32 {
        let t = (self.started.elapsed().as_secs_f32() * 1000.0 / self.ms).clamp(0.0, 1.0);
        // Hyprland's emphasizedDecel going in, emphasizedAccel going out
        // (close enough with a cubic ease)
        let eased = if self.to > self.from { 1.0 - (1.0 - t).powi(3) } else { t * t * t };
        self.from + (self.to - self.from) * eased
    }

    fn done(&self) -> bool {
        self.started.elapsed().as_secs_f32() * 1000.0 >= self.ms
    }
}

fn start_fade(hwnd: HWND, from: f32, to: f32, ms: f32, shown: bool) {
    *FADE.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(Fade { hwnd: hwnd.0 as isize, from, to, started: Instant::now(), ms, shown });
    unsafe {
        SetTimer(Some(hwnd), FADE_TIMER, 15, None);
    }
    step();
}

/// One frame of the fade; the window hides when it has faded out.
fn step() {
    let mut guard = FADE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(fade) = guard.as_ref() else { return };
    let value = fade.value();
    let hwnd = HWND(fade.hwnd as _);
    let done = fade.done();
    let shown = fade.shown;
    unsafe {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), (value * 255.0).round() as u8, LWA_ALPHA);
        if done {
            let _ = KillTimer(Some(hwnd), FADE_TIMER);
            if !shown {
                let _ = ShowWindow(hwnd, SW_HIDE);
                *guard = None;
            }
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TIMER && wparam.0 == FADE_TIMER {
        step();
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
