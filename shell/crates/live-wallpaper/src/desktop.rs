//! Where the wallpaper windows go: behind the desktop icons.
//! - Windows 11 24H2 and later ("raised desktop"): Progman is
//!   `WS_EX_NOREDIRECTIONBITMAP`, the icons (SHELLDLL_DefView) are a
//!   layered child of Progman and a child WorkerW at the bottom draws the
//!   wallpaper. Ours is a `WS_EX_LAYERED` child of Progman right under the
//!   icons, above that WorkerW (Microsoft's guidance for this layout;
//!   Lively does the same).
//! - Before that: Progman spawns a top-level WorkerW behind the icons on
//!   message 0x052C and ours becomes its child (as Sucrose and Lively do).
//! - Without Explorer (no Progman): top-level tool windows kept at the
//!   bottom of the z-order.
//! - Explorer there but neither layout found (its desktop is still being
//!   made, or a new Windows changed it): nothing is shown, the player
//!   tries again (a window under Progman would only be decoded, never
//!   seen).

use windows::{
  core::{w, PCWSTR},
  Win32::{
    Foundation::{BOOL, COLORREF, HWND, LPARAM, POINT, RECT, WPARAM},
    Graphics::Gdi::MapWindowPoints,
    UI::WindowsAndMessaging::*,
  },
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layer {
  Raised {
    progman: isize,
    defview: isize,
    workerw: isize,
  },
  Classic {
    workerw: isize,
  },
  Bare,
  Missing,
}

/// The window ours are children of (0: top-level).
pub fn parent(layer: Layer) -> isize {
  match layer {
    Layer::Raised { progman, .. } => progman,
    Layer::Classic { workerw } => workerw,
    Layer::Bare | Layer::Missing => 0,
  }
}

fn hwnd(h: isize) -> HWND {
  HWND(h as _)
}

fn find(parent: HWND, after: HWND, class: PCWSTR) -> HWND {
  unsafe { FindWindowExW(parent, after, class, PCWSTR::null()) }
    .unwrap_or_default()
}

pub fn find_layer() -> Layer {
  unsafe {
    let progman =
      FindWindowW(w!("Progman"), PCWSTR::null()).unwrap_or_default();
    if progman.is_invalid() {
      return Layer::Bare;
    }
    // Progman spawns the WorkerW behind the icons (nothing happens when it
    // is there already)
    let _ = SendMessageTimeoutW(
      progman,
      0x052C,
      WPARAM(0xD),
      LPARAM(1),
      SMTO_NORMAL,
      1000,
      None,
    );
    let raised = GetWindowLongPtrW(progman, GWL_EXSTYLE)
      & WS_EX_NOREDIRECTIONBITMAP.0 as isize
      != 0;
    if raised {
      let defview = find(progman, HWND::default(), w!("SHELLDLL_DefView"));
      if !defview.is_invalid() {
        let workerw = find(progman, HWND::default(), w!("WorkerW"));
        return Layer::Raised {
          progman: progman.0 as isize,
          defview: defview.0 as isize,
          workerw: workerw.0 as isize,
        };
      }
    }
    // the top-level window holding the icons; the WorkerW right after it
    // draws the wallpaper
    let mut found = HWND::default();
    let _ = EnumWindows(
      Some(icons_host),
      LPARAM(&mut found as *mut HWND as isize),
    );
    if !found.is_invalid() {
      return Layer::Classic {
        workerw: found.0 as isize,
      };
    }
    Layer::Missing
  }
}

unsafe extern "system" fn icons_host(top: HWND, lp: LPARAM) -> BOOL {
  if !find(top, HWND::default(), w!("SHELLDLL_DefView")).is_invalid() {
    let workerw = find(HWND::default(), top, w!("WorkerW"));
    if !workerw.is_invalid() {
      *(lp.0 as *mut HWND) = workerw;
      return BOOL(0);
    }
  }
  BOOL(1)
}

/// Makes `window` (created as a hidden `WS_POPUP`) the wallpaper window of
/// `rect` (screen pixels) and shows it.
pub fn attach(window: HWND, layer: Layer, rect: RECT) -> bool {
  unsafe {
    match layer {
      Layer::Missing => false,
      Layer::Bare => {
        set_child(window, false);
        let _ = SetParent(window, HWND::default());
        SetWindowPos(
          window,
          HWND_BOTTOM,
          rect.left,
          rect.top,
          rect.right - rect.left,
          rect.bottom - rect.top,
          SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .is_ok()
      }
      Layer::Classic { workerw } => {
        place_child(window, hwnd(workerw), HWND_TOP, rect)
      }
      Layer::Raised {
        progman,
        defview,
        workerw,
      } => {
        // a child of the no-redirection-bitmap Progman is drawn only when
        // layered; the classic layout has no such need (and its WorkerW
        // children show a video swap chain only when not layered)
        let ex = GetWindowLongPtrW(window, GWL_EXSTYLE);
        SetWindowLongPtrW(window, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        let _ = SetLayeredWindowAttributes(window, COLORREF(0), 255, LWA_ALPHA);
        let ok = place_child(window, hwnd(progman), hwnd(defview), rect);
        // the WorkerW drawing the static wallpaper stays under ours
        if workerw != 0 {
          let _ = SetWindowPos(
            hwnd(workerw),
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
          );
        }
        ok
      }
    }
  }
}

unsafe fn set_child(window: HWND, child: bool) {
  let style = GetWindowLongPtrW(window, GWL_STYLE);
  let (on, off) = if child {
    (WS_CHILD.0, WS_POPUP.0)
  } else {
    (WS_POPUP.0, WS_CHILD.0)
  };
  SetWindowLongPtrW(
    window,
    GWL_STYLE,
    (style & !(off as isize)) | on as isize,
  );
}

unsafe fn place_child(
  window: HWND,
  parent: HWND,
  after: HWND,
  rect: RECT,
) -> bool {
  // WS_CHILD before SetParent, as the documentation asks
  set_child(window, true);
  if SetParent(window, parent).is_err() {
    return false;
  }
  // from screen pixels into the parent's client area (it spans the whole
  // desktop)
  let mut at = [POINT {
    x: rect.left,
    y: rect.top,
  }];
  MapWindowPoints(HWND::default(), parent, &mut at);
  SetWindowPos(
    window,
    after,
    at[0].x,
    at[0].y,
    rect.right - rect.left,
    rect.bottom - rect.top,
    SWP_NOACTIVATE | SWP_SHOWWINDOW,
  )
  .is_ok()
}
