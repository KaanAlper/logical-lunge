//! The input method (IME: Chinese, Japanese, Korean ...) for the Super
//! menu's text field: the text being composed is read here and drawn in the
//! field (underlined, like a browser); the candidate list opens at the
//! caret. A few imm32 calls, declared here (the `windows` crate's Ime
//! feature is not built).

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, POINT, RECT};

pub const WM_IME_SETCONTEXT: u32 = 0x0281;
pub const WM_IME_STARTCOMPOSITION: u32 = 0x010D;
pub const WM_IME_ENDCOMPOSITION: u32 = 0x010E;
pub const WM_IME_COMPOSITION: u32 = 0x010F;
/// lParam of WM_IME_SETCONTEXT: the IME's own composition window
pub const ISC_SHOWUICOMPOSITIONWINDOW: isize = 0x8000_0000u32 as i32 as isize;
pub const GCS_COMPSTR: u32 = 0x0008;
pub const GCS_CURSORPOS: u32 = 0x0080;
pub const GCS_RESULTSTR: u32 = 0x0800;

const CFS_POINT: u32 = 0x0002;
const CFS_EXCLUDE: u32 = 0x0080;
const NI_COMPOSITIONSTR: u32 = 0x0015;
const CPS_CANCEL: u32 = 0x0004;

#[repr(C)]
struct CompositionForm {
  style: u32,
  pos: POINT,
  area: RECT,
}

#[repr(C)]
struct CandidateForm {
  index: u32,
  style: u32,
  pos: POINT,
  area: RECT,
}

#[link(name = "imm32")]
extern "system" {
  fn ImmGetContext(hwnd: HWND) -> isize;
  fn ImmReleaseContext(hwnd: HWND, himc: isize) -> i32;
  fn ImmGetCompositionStringW(himc: isize, index: u32, buf: *mut c_void, len: u32) -> i32;
  fn ImmSetCompositionWindow(himc: isize, form: *const CompositionForm) -> i32;
  fn ImmSetCandidateWindow(himc: isize, form: *const CandidateForm) -> i32;
  fn ImmNotifyIME(himc: isize, action: u32, index: u32, value: u32) -> i32;
}

fn with_context<T>(hwnd: HWND, f: impl FnOnce(isize) -> T) -> Option<T> {
  unsafe {
    let himc = ImmGetContext(hwnd);
    if himc == 0 {
      return None;
    }
    let out = f(himc);
    ImmReleaseContext(hwnd, himc);
    Some(out)
  }
}

/// The composition (`GCS_COMPSTR`) or the committed text (`GCS_RESULTSTR`).
pub fn string(hwnd: HWND, which: u32) -> String {
  with_context(hwnd, |himc| unsafe {
    let bytes = ImmGetCompositionStringW(himc, which, std::ptr::null_mut(), 0);
    if bytes <= 0 {
      return String::new();
    }
    let mut buf = vec![0u16; bytes as usize / 2];
    ImmGetCompositionStringW(himc, which, buf.as_mut_ptr().cast(), bytes as u32);
    String::from_utf16_lossy(&buf)
  })
  .unwrap_or_default()
}

/// The caret inside the composition, in UTF-16 units.
pub fn cursor(hwnd: HWND) -> usize {
  with_context(hwnd, |himc| unsafe { ImmGetCompositionStringW(himc, GCS_CURSORPOS, std::ptr::null_mut(), 0) })
    .unwrap_or(0)
    .max(0) as usize
}

/// Puts the IME's windows at the caret (client pixels): the candidate list
/// below the line, never over it.
pub fn place(hwnd: HWND, caret: POINT, line_h: i32) {
  let _ = with_context(hwnd, |himc| unsafe {
    let comp = CompositionForm { style: CFS_POINT, pos: caret, area: RECT::default() };
    ImmSetCompositionWindow(himc, &comp);
    let line = RECT { left: caret.x, top: caret.y, right: caret.x + 1, bottom: caret.y + line_h };
    let cand = CandidateForm { index: 0, style: CFS_EXCLUDE, pos: POINT { x: caret.x, y: caret.y + line_h }, area: line };
    ImmSetCandidateWindow(himc, &cand);
  });
}

/// Drops an unfinished composition (the menu closed).
pub fn cancel(hwnd: HWND) {
  let _ = with_context(hwnd, |himc| unsafe { ImmNotifyIME(himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0) });
}

/// UTF-16 position -> char position in `s`.
pub fn char_index(s: &str, utf16: usize) -> usize {
  let mut units = 0;
  for (i, c) in s.chars().enumerate() {
    if units >= utf16 {
      return i;
    }
    units += c.len_utf16();
  }
  s.chars().count()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn utf16_positions_map_to_chars() {
    assert_eq!(char_index("にほん", 2), 2);
    assert_eq!(char_index("a😀b", 3), 2);
    assert_eq!(char_index("ab", 9), 2);
  }
}
