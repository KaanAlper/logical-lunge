//! Whether any of a monitor's wallpaper can be seen past the windows on
//! it. Strips narrower than [`SLIVER`] (the gaps between tiled windows, a
//! screen edge) don't count: a video only there is not worth decoding.

use windows::Win32::Foundation::RECT;

/// The widest a strip of wallpaper between windows can be without counting
/// as the wallpaper showing (gaps are 8 px, outer gaps 5 px, scaled).
pub const SLIVER: i32 = 48;

/// True when a square wider than [`SLIVER`] fits somewhere on `monitor`
/// without touching any window in `windows`.
///
/// Asked about the square's top-left corner, exactly: the places it can
/// take on the monitor, minus, for each window, the places where the
/// square would touch it (the window grown up and left by the square's
/// size). Some place left over means the wallpaper shows.
pub fn shows(monitor: RECT, windows: &[RECT]) -> bool {
  let n = SLIVER + 1;
  let corners = RECT { left: monitor.left, top: monitor.top, right: monitor.right - n + 1, bottom: monitor.bottom - n + 1 };
  if corners.right <= corners.left || corners.bottom <= corners.top {
    return false;
  }
  let mut open = vec![corners];
  for w in windows {
    let touching = RECT { left: w.left - n + 1, top: w.top - n + 1, right: w.right, bottom: w.bottom };
    let mut next = Vec::with_capacity(open.len() + 4);
    for r in open {
      subtract(r, touching, &mut next);
    }
    if next.is_empty() {
      return false;
    }
    // a layout too broken up to follow: rather keep playing
    if next.len() > 4096 {
      return true;
    }
    open = next;
  }
  true
}

/// `r` minus `w`, as up to four rectangles: the bands above and below `w`
/// across all of `r`, and the pieces left and right of it in between.
fn subtract(r: RECT, w: RECT, out: &mut Vec<RECT>) {
  let left = w.left.max(r.left);
  let top = w.top.max(r.top);
  let right = w.right.min(r.right);
  let bottom = w.bottom.min(r.bottom);
  if left >= right || top >= bottom {
    out.push(r);
    return;
  }
  if r.top < top {
    out.push(RECT { left: r.left, top: r.top, right: r.right, bottom: top });
  }
  if bottom < r.bottom {
    out.push(RECT { left: r.left, top: bottom, right: r.right, bottom: r.bottom });
  }
  if r.left < left {
    out.push(RECT { left: r.left, top, right: left, bottom });
  }
  if right < r.right {
    out.push(RECT { left: right, top, right: r.right, bottom });
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT { left, top, right, bottom }
  }

  /// The same question asked the slow way: is there an uncovered square of
  /// SLIVER + 1 px anywhere, checking every pixel.
  fn shows_by_pixels(monitor: RECT, windows: &[RECT]) -> bool {
    let (w, h) = ((monitor.right - monitor.left) as usize, (monitor.bottom - monitor.top) as usize);
    let mut free = vec![vec![true; w]; h];
    for win in windows {
      for y in win.top.max(monitor.top)..win.bottom.min(monitor.bottom) {
        for x in win.left.max(monitor.left)..win.right.min(monitor.right) {
          free[(y - monitor.top) as usize][(x - monitor.left) as usize] = false;
        }
      }
    }
    // largest free square ending at each pixel
    let need = (SLIVER + 1) as usize;
    let mut size = vec![vec![0usize; w]; h];
    for y in 0..h {
      for x in 0..w {
        if free[y][x] {
          let s = if x == 0 || y == 0 { 1 } else { 1 + size[y - 1][x].min(size[y][x - 1]).min(size[y - 1][x - 1]) };
          size[y][x] = s;
          if s >= need {
            return true;
          }
        }
      }
    }
    false
  }

  #[test]
  fn tiled_layouts_hide_the_wallpaper_and_an_empty_one_shows_it() {
    let m = rect(0, 0, 1920, 1080);
    // bar on top, three columns with 8 px gaps and 5 px outer gaps
    let bar = rect(0, 0, 1920, 40);
    let cols = [rect(5, 45, 636, 1075), rect(644, 45, 1275, 1075), rect(1283, 45, 1915, 1075)];
    let mut tiled = vec![bar];
    tiled.extend_from_slice(&cols);
    assert!(!shows(m, &tiled));
    assert!(shows(m, &[bar]), "an empty workspace shows the wallpaper");
    assert!(shows(m, &[]));
    // one column closed: its space shows
    assert!(shows(m, &tiled[..3]));
    // a window off this monitor changes nothing
    assert!(shows(m, &[bar, rect(-1280, 0, 0, 1024)]));
  }

  #[test]
  fn agrees_with_checking_every_pixel() {
    // generated layouts on a small monitor: windows of every size and place, overlapping or not
    let m = rect(100, 50, 420, 290);
    let mut seed: u64 = 0x9e3779b97f4a7c15;
    let mut next = |n: i32| {
      seed ^= seed << 13;
      seed ^= seed >> 7;
      seed ^= seed << 17;
      (seed % n as u64) as i32
    };
    for case in 0..400 {
      let count = 1 + next(6) as usize;
      let windows: Vec<RECT> = (0..count)
        .map(|_| {
          let (x, y) = (m.left - 40 + next(400), m.top - 40 + next(320));
          rect(x, y, x + 1 + next(260), y + 1 + next(220))
        })
        .collect();
      assert_eq!(shows(m, &windows), shows_by_pixels(m, &windows), "case {case}: {:?}", windows.iter().map(|r| (r.left, r.top, r.right, r.bottom)).collect::<Vec<_>>());
    }
  }
}
