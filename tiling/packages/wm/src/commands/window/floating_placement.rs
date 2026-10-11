//! Logical Lunge: where a new floating window opens.
//!
//! Every floating window used to open in the middle of its workspace, so
//! several of them (launchers that can't take their tile, small apps)
//! landed on the same spot and covered each other. A window still opens in
//! the middle when that spot is free, as in Hyprland; otherwise it goes
//! where it covers the least of the other floating windows (KWin's
//! "minimal overlapping" placement), and when every spot is taken it
//! cascades from the last one. A dialog opens over the window that owns it.
//!
//! Only rects go in and out, so the placement is tested on its own.

use wm_platform::Rect;

/// Room kept between a placed window and the floating windows next to it.
const SPACING: i32 = 10;

/// How far each cascaded window is moved from the one before it.
const CASCADE_STEP: i32 = 32;

/// Where a new floating window that has no owner opens on `work_area` (its
/// workspace inside the outer gaps, not under the bar), given the floating
/// windows already shown there (`others`, oldest first).
///
/// In the middle when that spot covers none of them; otherwise at the spot
/// that covers the least of them, nearest to the middle; when every spot
/// covers one, a step down-right of the last one. A window bigger than the
/// work area is cut to it.
#[must_use]
pub fn place_free_floating(
  window: &Rect,
  work_area: &Rect,
  others: &[Rect],
) -> Rect {
  let centred = fit_into(&window.translate_to_center(work_area), work_area);

  // only the windows on this work area can be covered
  let others = others
    .iter()
    .filter(|other| other.intersection_area(work_area) > 0)
    .collect::<Vec<_>>();

  if covered_area(&centred, &others) == 0 {
    return centred;
  }

  match least_covering(&centred, work_area, &others) {
    Some(free) => free,
    None => cascade(&centred, work_area, &others),
  }
}

/// Where a dialog opens: centred over the window that owns it, kept inside
/// `work_area`.
#[must_use]
pub fn center_on_owner(window: &Rect, owner: &Rect, work_area: &Rect) -> Rect {
  fit_into(&window.translate_to_center(owner), work_area)
}

/// Moves (and when bigger, cuts) a rect so that it lies inside `area`.
fn fit_into(rect: &Rect, area: &Rect) -> Rect {
  let width = rect.width().min(area.width());
  let height = rect.height().min(area.height());

  Rect::from_xy(
    rect.x().clamp(area.left, area.right - width),
    rect.y().clamp(area.top, area.bottom - height),
    width,
    height,
  )
}

/// How much of the other windows a rect covers, in px².
fn covered_area(rect: &Rect, others: &[&Rect]) -> i64 {
  others
    .iter()
    .map(|other| i64::from(rect.intersection_area(other)))
    .sum()
}

/// The free spot nearest to the middle, among the spots next to the other
/// windows' edges and the work area's own edges. `None` when every spot
/// covers some other window.
fn least_covering(
  centred: &Rect,
  work_area: &Rect,
  others: &[&Rect],
) -> Option<Rect> {
  let (width, height) = (centred.width(), centred.height());

  let mut xs = vec![centred.x(), work_area.left, work_area.right - width];
  let mut ys = vec![centred.y(), work_area.top, work_area.bottom - height];
  for other in others {
    xs.extend([other.right + SPACING, other.left - SPACING - width, other.left]);
    ys.extend([other.bottom + SPACING, other.top - SPACING - height, other.top]);
  }

  let xs = sorted_in(xs, work_area.left, work_area.right - width);
  let ys = sorted_in(ys, work_area.top, work_area.bottom - height);

  let distance = |rect: &Rect| {
    let dx = i64::from(rect.x() - centred.x());
    let dy = i64::from(rect.y() - centred.y());
    dx * dx + dy * dy
  };

  xs.iter()
    .flat_map(|&x| ys.iter().map(move |&y| Rect::from_xy(x, y, width, height)))
    .filter(|rect| covered_area(rect, others) == 0)
    .min_by_key(distance)
}

/// Candidate positions kept in `[min, max]`, without repeats.
fn sorted_in(mut values: Vec<i32>, min: i32, max: i32) -> Vec<i32> {
  for value in &mut values {
    *value = (*value).clamp(min, max.max(min));
  }
  values.sort_unstable();
  values.dedup();
  values
}

/// A step down-right of the last floating window, starting again from the
/// work area's top-left corner when it would leave it, and skipping corners
/// another window already opened at (so no two sit exactly on each other).
fn cascade(centred: &Rect, work_area: &Rect, others: &[&Rect]) -> Rect {
  let Some(last) = others.last() else {
    return centred.clone();
  };

  let (width, height) = (centred.width(), centred.height());
  let max_x = work_area.right - width;
  let max_y = work_area.bottom - height;
  let taken = |x: i32, y: i32| {
    others.iter().any(|other| other.left == x && other.top == y)
  };

  let (mut x, mut y) = (last.left, last.top);
  for _ in 0..=others.len() {
    x += CASCADE_STEP;
    y += CASCADE_STEP;
    if x > max_x {
      x = work_area.left;
    }
    if y > max_y {
      y = work_area.top;
    }
    if !taken(x, y) {
      break;
    }
  }

  fit_into(&Rect::from_xy(x, y, width, height), work_area)
}

#[cfg(test)]
mod tests {
  use super::*;

  const WORK_AREA: (i32, i32, i32, i32) = (10, 60, 1900, 1010);

  fn work_area() -> Rect {
    let (x, y, width, height) = WORK_AREA;
    Rect::from_xy(x, y, width, height)
  }

  fn overlap(a: &Rect, b: &Rect) -> i32 {
    a.intersection_area(b)
  }

  #[test]
  fn first_floating_window_opens_centred() {
    let window = Rect::from_xy(0, 0, 800, 600);
    let placed = place_free_floating(&window, &work_area(), &[]);
    assert_eq!(placed, window.translate_to_center(&work_area()));
  }

  #[test]
  fn opens_centred_when_the_middle_is_free() {
    let window = Rect::from_xy(0, 0, 400, 300);
    let corner = Rect::from_xy(20, 70, 300, 200);
    let placed = place_free_floating(&window, &work_area(), &[corner]);
    assert_eq!(placed, window.translate_to_center(&work_area()));
  }

  #[test]
  fn second_floating_window_does_not_cover_the_first() {
    let window = Rect::from_xy(0, 0, 600, 400);
    let first = place_free_floating(&window, &work_area(), &[]);
    let second =
      place_free_floating(&window, &work_area(), &[first.clone()]);

    assert_eq!(overlap(&first, &second), 0);
    assert!(work_area().contains_rect(&second));
    // it stays beside the first, at the same height (nearest the middle)
    assert_eq!(second.y(), first.y());
  }

  #[test]
  fn third_floating_window_covers_neither() {
    let window = Rect::from_xy(0, 0, 500, 400);
    let mut placed = Vec::new();
    for _ in 0..3 {
      let next = place_free_floating(&window, &work_area(), &placed);
      placed.push(next);
    }

    for (index, a) in placed.iter().enumerate() {
      assert!(work_area().contains_rect(a));
      for b in &placed[index + 1..] {
        assert_eq!(overlap(a, b), 0, "{a:?} covers {b:?}");
      }
    }
  }

  #[test]
  fn full_work_area_cascades_from_the_last_window() {
    let window = Rect::from_xy(0, 0, 1200, 800);
    let first = place_free_floating(&window, &work_area(), &[]);
    let second =
      place_free_floating(&window, &work_area(), &[first.clone()]);
    let third = place_free_floating(
      &window,
      &work_area(),
      &[first.clone(), second.clone()],
    );

    // no room beside a 1200x800 window: each one is a step from the last
    assert_eq!(
      (second.x(), second.y()),
      (first.x() + CASCADE_STEP, first.y() + CASCADE_STEP)
    );
    assert_eq!(
      (third.x(), third.y()),
      (second.x() + CASCADE_STEP, second.y() + CASCADE_STEP)
    );
    assert!(work_area().contains_rect(&third));
  }

  #[test]
  fn cascade_starts_again_from_the_corner_at_the_edge() {
    let window = Rect::from_xy(0, 0, 1200, 800);
    let area = work_area();
    let last = Rect::from_xy(area.right - 1200, area.bottom - 800, 1200, 800);
    let middle = window.translate_to_center(&area);

    let placed = place_free_floating(&window, &area, &[middle, last]);

    assert_eq!((placed.x(), placed.y()), (area.left, area.top));
  }

  #[test]
  fn many_windows_never_sit_exactly_on_each_other() {
    let window = Rect::from_xy(0, 0, 1000, 700);
    let mut placed = Vec::new();
    for _ in 0..12 {
      let next = place_free_floating(&window, &work_area(), &placed);
      assert!(work_area().contains_rect(&next));
      assert!(
        !placed.iter().any(|p: &Rect| p.x() == next.x() && p.y() == next.y()),
        "{next:?} opened on top of another window"
      );
      placed.push(next);
    }
  }

  #[test]
  fn dialog_opens_centred_on_its_owner() {
    let owner = Rect::from_xy(100, 100, 1000, 800);
    let dialog = Rect::from_xy(0, 0, 400, 200);
    let placed = center_on_owner(&dialog, &owner, &work_area());
    let (placed, owner) = (placed.center_point(), owner.center_point());
    assert_eq!((placed.x, placed.y), (owner.x, owner.y));
  }

  #[test]
  fn dialog_of_an_owner_at_the_edge_stays_on_screen() {
    let owner = Rect::from_xy(1700, 900, 400, 300);
    let dialog = Rect::from_xy(0, 0, 600, 400);
    let placed = center_on_owner(&dialog, &owner, &work_area());
    assert!(work_area().contains_rect(&placed));
    assert_eq!((placed.width(), placed.height()), (600, 400));
  }

  #[test]
  fn window_bigger_than_the_work_area_is_cut_to_it() {
    let window = Rect::from_xy(-50, -50, 2600, 1500);
    let placed = place_free_floating(&window, &work_area(), &[]);
    assert_eq!(placed, work_area());

    let dialog = center_on_owner(&window, &work_area(), &work_area());
    assert_eq!(dialog, work_area());
  }

  #[test]
  fn monitor_left_of_the_primary_has_negative_coordinates() {
    // a 1920x1080 monitor left of the primary one, bar on top
    let area = Rect::from_ltrb(-1910, 60, -10, 1070);
    let window = Rect::from_xy(200, 200, 600, 400);

    let first = place_free_floating(&window, &area, &[]);
    assert_eq!(first, window.translate_to_center(&area));

    let second = place_free_floating(&window, &area, &[first.clone()]);
    assert!(area.contains_rect(&second));
    assert_eq!(overlap(&first, &second), 0);
  }

  #[test]
  fn windows_on_another_monitor_are_not_avoided() {
    let window = Rect::from_xy(0, 0, 800, 600);
    let elsewhere =
      window.translate_to_center(&Rect::from_ltrb(-1910, 60, -10, 1070));
    let placed = place_free_floating(&window, &work_area(), &[elsewhere]);
    assert_eq!(placed, window.translate_to_center(&work_area()));
  }
}
