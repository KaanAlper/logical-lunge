//! illogical-impulse's bar and screen corners: under the bar's two ends a
//! concave corner in the bar's colour (the bar "hugs" the screen), and at the
//! screen's bottom corners a black rounded corner (ii's fake screen rounding),
//! both with radius 23.
//!
//! Four small layered windows per monitor, painted once with
//! `UpdateLayeredWindow` (again only when the bar's colour changes): no
//! timers, nothing per frame. Like the bar they are not topmost, so a
//! fullscreen window in front (a game, a video, Super+F) covers them and
//! nothing sits over it. Clicks go through them (`WS_EX_TRANSPARENT`).

use std::sync::Once;

use windows::{
  core::{w, PCWSTR},
  Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::{
      CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA,
      AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
      CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, ShowWindow, UpdateLayeredWindow,
      SW_SHOWNOACTIVATE, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
      WS_POPUP,
    },
  },
};

use super::gfx::Rgba;

const RADIUS: f32 = 23.0;
const CLASS: PCWSTR = w!("LungeCorner");
const BLACK: (u8, u8, u8) = (0, 0, 0);

/// Which corner: where the square sits and which way its curve faces.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
  /// under the bar, at the left / right screen edge, in the bar's colour
  HugLeft,
  HugRight,
  /// the screen's bottom corners, black
  BottomLeft,
  BottomRight,
}

impl Kind {
  /// The centre of the circle the corner's curve follows, in the square's pixels.
  fn centre(self, s: f32) -> (f32, f32) {
    match self {
      Kind::HugLeft => (s, s),
      Kind::HugRight => (0.0, s),
      Kind::BottomLeft => (s, 0.0),
      Kind::BottomRight => (0.0, 0.0),
    }
  }
}

pub(super) struct Corners {
  windows: Vec<(HWND, Kind, POINT)>,
  size: i32,
  /// the bar's colour the hug corners were last painted in
  bar: (u8, u8, u8),
}

impl Corners {
  /// The corners of the monitor `mon` whose bar is `bar_h` px tall, at `scale`.
  pub(super) fn new(mon: RECT, bar_h: i32, scale: f32, bar: Rgba) -> anyhow::Result<Self> {
    let size = (RADIUS * scale).round().max(1.0) as i32;
    let bar = (bar.0, bar.1, bar.2);
    let at = [
      (Kind::HugLeft, POINT { x: mon.left, y: mon.top + bar_h }),
      (Kind::HugRight, POINT { x: mon.right - size, y: mon.top + bar_h }),
      (Kind::BottomLeft, POINT { x: mon.left, y: mon.bottom - size }),
      (Kind::BottomRight, POINT { x: mon.right - size, y: mon.bottom - size }),
    ];
    let mut corners = Corners { windows: Vec::with_capacity(4), size, bar };
    for (kind, pos) in at {
      let hwnd = create()?;
      corners.windows.push((hwnd, kind, pos));
      paint(hwnd, kind, size, pos, if matches!(kind, Kind::HugLeft | Kind::HugRight) { bar } else { BLACK })?;
      unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
      }
    }
    Ok(corners)
  }

  /// Paints the hug corners again when the bar's colour changed (a theme or accent change).
  pub(super) fn set_bar_color(&mut self, bar: Rgba) {
    let bar = (bar.0, bar.1, bar.2);
    if bar == self.bar {
      return;
    }
    self.bar = bar;
    for &(hwnd, kind, pos) in &self.windows {
      if matches!(kind, Kind::HugLeft | Kind::HugRight) {
        if let Err(err) = paint(hwnd, kind, self.size, pos, bar) {
          tracing::warn!("Bar corners: paint: {:?}", err);
        }
      }
    }
  }
}

impl Drop for Corners {
  fn drop(&mut self) {
    for &(hwnd, _, _) in &self.windows {
      unsafe {
        let _ = DestroyWindow(hwnd);
      }
    }
  }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
  DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn create() -> anyhow::Result<HWND> {
  static REGISTER: Once = Once::new();
  unsafe {
    let instance = GetModuleHandleW(None)?;
    REGISTER.call_once(|| {
      RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(), lpszClassName: CLASS, ..Default::default() });
    });
    Ok(CreateWindowExW(
      WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
      CLASS,
      PCWSTR::null(),
      WS_POPUP,
      0,
      0,
      0,
      0,
      None,
      None,
      instance,
      None,
    )?)
  }
}

/// The corner's coverage at pixel (x, y): outside the curve, with a one-pixel soft edge.
fn coverage(kind: Kind, s: f32, x: i32, y: i32) -> f32 {
  let (cx, cy) = kind.centre(s);
  let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
  ((dx * dx + dy * dy).sqrt() - s + 0.5).clamp(0.0, 1.0)
}

/// Draws the corner into a premultiplied 32-bit bitmap and hands it to the window at `pos`.
fn paint(hwnd: HWND, kind: Kind, size: i32, pos: POINT, rgb: (u8, u8, u8)) -> anyhow::Result<()> {
  unsafe {
    let screen = GetDC(None);
    let dc = CreateCompatibleDC(screen);
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER {
      biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
      biWidth: size,
      biHeight: -size,
      biPlanes: 1,
      biBitCount: 32,
      biCompression: BI_RGB.0,
      ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let result = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, None, 0).map_err(anyhow::Error::from).and_then(|bitmap| {
      let old = SelectObject(dc, bitmap);
      let pixels = std::slice::from_raw_parts_mut(bits as *mut [u8; 4], (size * size) as usize);
      let s = size as f32;
      for y in 0..size {
        for x in 0..size {
          let a = coverage(kind, s, x, y);
          let premul = |c: u8| (c as f32 * a).round() as u8;
          pixels[(y * size + x) as usize] = [premul(rgb.2), premul(rgb.1), premul(rgb.0), (a * 255.0).round() as u8];
        }
      }
      let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
      let shown = UpdateLayeredWindow(
        hwnd,
        screen,
        Some(&pos),
        Some(&SIZE { cx: size, cy: size }),
        dc,
        Some(&POINT::default()),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
      );
      SelectObject(dc, old);
      let _ = DeleteObject(bitmap);
      shown.map_err(anyhow::Error::from)
    });
    let _ = DeleteDC(dc);
    ReleaseDC(None, screen);
    result
  }
}

#[cfg(test)]
mod tests {
  use super::{coverage, Kind};

  #[test]
  fn fills_outside_the_curve_only() {
    let s = 23.0;
    // the hug corner fills the screen edge's corner, not the curve's inside
    assert_eq!(coverage(Kind::HugLeft, s, 0, 0), 1.0);
    assert_eq!(coverage(Kind::HugLeft, s, 22, 22), 0.0);
    assert_eq!(coverage(Kind::HugRight, s, 22, 0), 1.0);
    assert_eq!(coverage(Kind::HugRight, s, 0, 22), 0.0);
    // the screen's bottom corners: filled at the very corner, open towards the screen's middle
    assert_eq!(coverage(Kind::BottomLeft, s, 0, 22), 1.0);
    assert_eq!(coverage(Kind::BottomLeft, s, 22, 0), 0.0);
    assert_eq!(coverage(Kind::BottomRight, s, 22, 22), 1.0);
    assert_eq!(coverage(Kind::BottomRight, s, 0, 0), 0.0);
  }
}
