//! How a video fills a monitor: like a wallpaper's "fill", the middle of
//! the video at the monitor's aspect ratio (no bars, the excess cut
//! equally from both sides).

/// The part of the video to show, in 0..1 of its width and height: (left,
/// top, right, bottom).
pub fn cover(
  video_w: u32,
  video_h: u32,
  screen_w: u32,
  screen_h: u32,
) -> (f32, f32, f32, f32) {
  if video_w == 0 || video_h == 0 || screen_w == 0 || screen_h == 0 {
    return (0.0, 0.0, 1.0, 1.0);
  }
  let video = video_w as f64 / video_h as f64;
  let screen = screen_w as f64 / screen_h as f64;
  if video > screen {
    // wider than the screen: cut the sides
    let keep = screen / video;
    let side = ((1.0 - keep) / 2.0) as f32;
    (side, 0.0, 1.0 - side, 1.0)
  } else {
    let keep = video / screen;
    let side = ((1.0 - keep) / 2.0) as f32;
    (0.0, side, 1.0, 1.0 - side)
  }
}

#[cfg(test)]
mod tests {
  use super::cover;

  #[test]
  fn same_aspect_shows_everything() {
    assert_eq!(cover(1920, 1080, 2560, 1440), (0.0, 0.0, 1.0, 1.0));
  }

  #[test]
  fn ultrawide_screen_cuts_top_and_bottom() {
    let (l, t, r, b) = cover(1920, 1080, 3440, 1440);
    assert_eq!((l, r), (0.0, 1.0));
    assert!(
      (t - 0.1279).abs() < 0.001 && (b - 0.8721).abs() < 0.001,
      "{t} {b}"
    );
  }

  #[test]
  fn portrait_screen_cuts_the_sides() {
    let (l, t, r, b) = cover(1920, 1080, 1080, 1920);
    assert_eq!((t, b), (0.0, 1.0));
    assert!(
      (l - 0.3418).abs() < 0.001 && (r - 0.6582).abs() < 0.001,
      "{l} {r}"
    );
  }

  #[test]
  fn unknown_size_shows_everything() {
    assert_eq!(cover(0, 0, 1920, 1080), (0.0, 0.0, 1.0, 1.0));
  }
}
