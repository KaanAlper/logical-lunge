//! Logical Lunge: the geometry of Hyprland's dwindle layout (0.56,
//! `src/layout/algorithm/tiled/dwindle/DwindleAlgorithm.cpp` and
//! `src/layout/target/WindowTarget.cpp`), with illogical-impulse's
//! settings (`preserve_split = true`, `smart_split = false`,
//! `force_split = 0`, `split_width_multiplier = 1`, `default_split_ratio =
//! 1`, `use_active_for_splits = true`, `gaps_in = 4`, `gaps_out = 5`).
//!
//! The layout tree tiles *node boxes*: the workspace's work area (the
//! monitor minus reserved space minus `gaps_out`) is split by ratio, with
//! no gaps between the boxes. A window then sits in its node box less
//! `gaps_in` on every side that does not touch the work area. Two
//! neighbours are therefore `2 * gaps_in` apart (our `inner_gap`), and an
//! uneven split keeps its exact ratio between the node boxes.
//!
//! Pure functions only (no crate types), so the module also builds on its
//! own: `rustc --edition 2021 --test src/dwindle_math.rs`.

/// A box in pixels with fractional edges (Hyprland's `CBox`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bx {
  pub x: f64,
  pub y: f64,
  pub w: f64,
  pub h: f64,
}

impl Bx {
  pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
    Self { x, y, w, h }
  }
}

/// `gaps_in` of one side, horizontally and vertically (half our
/// `inner_gap`, which is the whole space between two windows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gaps {
  pub h: f64,
  pub v: f64,
}

/// Hyprland's `STICKS(a, b)` (macros.hpp): edges within 2px are the same.
pub fn sticks(a: f64, b: f64) -> bool {
  (a - b).abs() < 2.
}

/// The window inside its node box (WindowTarget.cpp `updatePos`): the node
/// box is rounded like `CBox::round` (x/y rounded, the far edges rounded),
/// then `gaps_in` comes off each side that is not on the work area's edge
/// (decided on the unrounded box). Returns x, y, width, height.
pub fn window_box(node: Bx, work: Bx, gaps: Gaps) -> (i32, i32, i32, i32) {
  let left = sticks(node.x, work.x);
  let right = sticks(node.x + node.w, work.x + work.w);
  let top = sticks(node.y, work.y);
  let bottom = sticks(node.y + node.h, work.y + work.h);

  let rx = node.x.round();
  let ry = node.y.round();
  let rw = (node.x + node.w - rx).round();
  let rh = (node.y + node.h - ry).round();

  let gl = if left { 0. } else { gaps.h };
  let gr = if right { 0. } else { gaps.h };
  let gt = if top { 0. } else { gaps.v };
  let gb = if bottom { 0. } else { gaps.v };

  #[allow(clippy::cast_possible_truncation)]
  (
    (rx + gl).round() as i32,
    (ry + gt).round() as i32,
    (rw - gl - gr).max(0.).round() as i32,
    (rh - gt - gb).max(0.).round() as i32,
  )
}

/// The gaps a window loses along one axis in its node box (left + right,
/// or top + bottom): what a target window length must add to become a
/// node length.
pub fn gap_along(node: Bx, work: Bx, gaps: Gaps, horizontal: bool) -> f64 {
  if horizontal {
    let l = if sticks(node.x, work.x) { 0. } else { gaps.h };
    let r = if sticks(node.x + node.w, work.x + work.w) { 0. } else { gaps.h };
    l + r
  } else {
    let t = if sticks(node.y, work.y) { 0. } else { gaps.v };
    let b = if sticks(node.y + node.h, work.y + work.h) { 0. } else { gaps.v };
    t + b
  }
}

/// The children's node boxes (`recalcSizePosRecursive`): the parent box is
/// cut along the split by the children's shares, without gaps. A binary
/// split with ratio `r` gives the first child `w / 2 * r` (shares `r / 2`
/// and `1 - r / 2`). The last child always ends at the parent's edge.
pub fn partition(parent: Bx, horizontal: bool, shares: &[f64]) -> Vec<Bx> {
  let total: f64 = shares.iter().sum();
  let total = if total > 0. { total } else { 1. };
  let length = if horizontal { parent.w } else { parent.h };
  let mut out = Vec::with_capacity(shares.len());
  let mut start = 0.;

  for (i, share) in shares.iter().enumerate() {
    let end = if i + 1 == shares.len() {
      length
    } else {
      start + length * share / total
    };
    out.push(if horizontal {
      Bx::new(parent.x + start, parent.y, end - start, parent.h)
    } else {
      Bx::new(parent.x, parent.y + start, parent.w, end - start)
    });
    start = end;
  }

  out
}

/// The split of a new parent node (`addTarget`, line 149): side by side
/// when the box is wider than tall (`split_width_multiplier = 1`). With
/// `preserve_split` the direction then never changes.
pub fn side_by_side(node: Bx) -> bool {
  node.w > node.h
}

/// Whether the new window takes the first half of the split node
/// (`force_split = 0`, lines 213-222): the point (cursor, or the focal
/// point of a move) left of / above the box's middle. A point outside the
/// box counts the same way.
pub fn new_is_first(node: Bx, point: (f64, f64)) -> bool {
  if side_by_side(node) {
    point.0 < node.x + node.w / 2.
  } else {
    point.1 < node.y + node.h / 2.
  }
}

/// Squared distance from a point to a box, 0 inside
/// (`vecToRectDistanceSquared`, used by `getClosestNode`).
pub fn distance_sq(point: (f64, f64), b: Bx) -> f64 {
  let dx = (b.x - point.0).max(0.).max(point.0 - (b.x + b.w));
  let dy = (b.y - point.1).max(0.).max(point.1 - (b.y + b.h));
  dx * dx + dy * dy
}

/// The index of the closest box (first one on a tie, as `getClosestNode`).
pub fn closest(point: (f64, f64), boxes: &[Bx]) -> Option<usize> {
  let mut best: Option<(usize, f64)> = None;
  for (i, b) in boxes.iter().enumerate() {
    let d = distance_sq(point, *b);
    if best.is_none_or(|(_, bd)| d < bd) {
      best = Some((i, d));
    }
  }
  best.map(|(i, _)| i)
}

/// Hyprland's split ratio for a binary split from the first child's share
/// (`first = ratio / 2`), and back; both clamp to 0.1..1.9.
pub fn ratio_from_share(first_share: f64) -> f64 {
  (first_share * 2.).clamp(0.1, 1.9)
}

pub fn share_from_ratio(ratio: f64) -> f64 {
  ratio.clamp(0.1, 1.9) / 2.
}

#[cfg(test)]
mod tests {
  //! Hyprland's dwindle (ported line by line below as `hypr`) against our
  //! tree model (`ours`: splits holding child shares, as the WM's
  //! containers with `tiling_size`), both using the functions above for
  //! what they share. Every sequence must give the same window rectangles.

  use super::*;

  const GAPS: Gaps = Gaps { h: 4., v: 4. };

  // ---- Hyprland (DwindleAlgorithm.cpp)

  mod hypr {
    use super::super::*;

    #[derive(Clone, Debug)]
    pub struct Node {
      pub parent: Option<usize>,
      pub children: [Option<usize>; 2],
      pub window: Option<u32>,
      pub box_: Bx,
      pub split_top: bool,
      pub ratio: f64,
      pub alive: bool,
    }

    #[derive(Default)]
    pub struct Dwindle {
      pub nodes: Vec<Node>,
      pub order: Vec<usize>, // m_dwindleNodesData order (for ties)
    }

    impl Dwindle {
      fn recalc(&mut self, n: usize) {
        let node = self.nodes[n].clone();
        if let [Some(a), Some(b)] = node.children {
          // preserve_split = 1: splitTop is never recomputed (line 35)
          let side = !node.split_top;
          if side {
            let first = node.box_.w / 2. * node.ratio;
            self.nodes[a].box_ = Bx::new(node.box_.x, node.box_.y, first, node.box_.h);
            self.nodes[b].box_ = Bx::new(node.box_.x + first, node.box_.y, node.box_.w - first, node.box_.h);
          } else {
            let first = node.box_.h / 2. * node.ratio;
            self.nodes[a].box_ = Bx::new(node.box_.x, node.box_.y, node.box_.w, first);
            self.nodes[b].box_ = Bx::new(node.box_.x, node.box_.y + first, node.box_.w, node.box_.h - first);
          }
          self.recalc(a);
          self.recalc(b);
        }
      }

      fn root(&self) -> Option<usize> {
        self.order.iter().copied().find(|&n| self.nodes[n].alive && self.nodes[n].parent.is_none())
      }

      pub fn layout(&mut self, work: Bx) {
        if let Some(r) = self.root() {
          self.nodes[r].box_ = work;
          self.recalc(r);
        }
      }

      pub fn node_of(&self, w: u32) -> Option<usize> {
        self.order.iter().copied().find(|&n| self.nodes[n].alive && self.nodes[n].window == Some(w))
      }

      fn leaves(&self) -> Vec<usize> {
        self.order.iter().copied().filter(|&n| self.nodes[n].alive && self.nodes[n].window.is_some()).collect()
      }

      /// addTarget with use_active_for_splits = 1 (lines 95-105).
      pub fn add(&mut self, w: u32, active: Option<u32>, mouse: (f64, f64), work: Bx, focal: Option<(f64, f64)>) {
        let pnode = self.nodes.len();
        self.nodes.push(Node { parent: None, children: [None, None], window: Some(w), box_: work, split_top: false, ratio: 1., alive: true });
        self.order.push(pnode);
        let point = focal.unwrap_or(mouse);

        let leaves: Vec<usize> = self.leaves().into_iter().filter(|&n| n != pnode).collect();
        let opening = if let Some(f) = focal {
          let boxes: Vec<Bx> = leaves.iter().map(|&n| self.nodes[n].box_).collect();
          closest(f, &boxes).map(|i| leaves[i])
        } else {
          active.and_then(|a| self.node_of(a)).filter(|&n| n != pnode).or_else(|| {
            let boxes: Vec<Bx> = leaves.iter().map(|&n| self.nodes[n].box_).collect();
            closest(mouse, &boxes).map(|i| leaves[i])
          })
        };

        let Some(on) = opening else {
          self.nodes[pnode].box_ = work;
          return;
        };

        let np = self.nodes.len();
        let obox = self.nodes[on].box_;
        let side = obox.w > obox.h * 1.;
        self.nodes.push(Node { parent: self.nodes[on].parent, children: [None, None], window: None, box_: obox, split_top: !side, ratio: 1., alive: true });
        self.order.push(np);
        let first = if side { point.0 < obox.x + obox.w / 2. } else { point.1 < obox.y + obox.h / 2. };
        self.nodes[np].children = if first { [Some(pnode), Some(on)] } else { [Some(on), Some(pnode)] };
        if let Some(p) = self.nodes[on].parent {
          if self.nodes[p].children[0] == Some(on) {
            self.nodes[p].children[0] = Some(np);
          } else {
            self.nodes[p].children[1] = Some(np);
          }
        }
        self.nodes[on].parent = Some(np);
        self.nodes[pnode].parent = Some(np);
        self.layout(work);
      }

      /// removeTarget (lines 268-310).
      pub fn remove(&mut self, w: u32, work: Bx) {
        let Some(n) = self.node_of(w) else { return };
        let Some(p) = self.nodes[n].parent else {
          self.nodes[n].alive = false;
          return;
        };
        let sib = if self.nodes[p].children[0] == Some(n) { self.nodes[p].children[1] } else { self.nodes[p].children[0] }.unwrap();
        self.nodes[sib].parent = self.nodes[p].parent;
        if let Some(pp) = self.nodes[p].parent {
          if self.nodes[pp].children[0] == Some(p) {
            self.nodes[pp].children[0] = Some(sib);
          } else {
            self.nodes[pp].children[1] = Some(sib);
          }
        }
        self.nodes[p].alive = false;
        self.nodes[n].alive = false;
        self.layout(work);
      }

      /// layoutmsg splitratio (lines 749-767).
      pub fn split_ratio(&mut self, w: u32, delta: f64, work: Bx) {
        if let Some(n) = self.node_of(w) {
          if let Some(p) = self.nodes[n].parent {
            self.nodes[p].ratio = (self.nodes[p].ratio + delta).clamp(0.1, 1.9);
            self.layout(work);
          }
        }
      }

      pub fn rects(&self, work: Bx) -> Vec<(u32, (i32, i32, i32, i32))> {
        let mut out: Vec<_> = self.leaves().into_iter().map(|n| (self.nodes[n].window.unwrap(), window_box(self.nodes[n].box_, work, super::GAPS))).collect();
        out.sort_by_key(|(w, _)| *w);
        out
      }
    }
  }

  // ---- ours: splits with child shares (the WM's SplitContainer +
  // tiling_size), inserting with the module's decisions

  mod ours {
    use super::super::*;

    #[derive(Clone, Debug)]
    pub enum T {
      Win(u32),
      Split { horizontal: bool, kids: Vec<(T, f64)> },
    }

    pub struct Tree {
      pub root: Option<T>, // workspace's single child or root split
    }

    fn collect(t: &T, b: Bx, out: &mut Vec<(u32, Bx)>) {
      match t {
        T::Win(w) => out.push((*w, b)),
        T::Split { horizontal, kids } => {
          let shares: Vec<f64> = kids.iter().map(|(_, s)| *s).collect();
          for ((k, _), kb) in kids.iter().zip(partition(b, *horizontal, &shares)) {
            collect(k, kb, out);
          }
        }
      }
    }

    impl Tree {
      pub fn boxes(&self, work: Bx) -> Vec<(u32, Bx)> {
        let mut out = vec![];
        if let Some(r) = &self.root {
          collect(r, work, &mut out);
        }
        out
      }

      fn replace(t: &mut T, target: u32, new: u32, first: bool, horizontal: bool) -> bool {
        match t {
          T::Win(w) if *w == target => {
            let old = T::Win(*w);
            let kids = if first { vec![(T::Win(new), 0.5), (old, 0.5)] } else { vec![(old, 0.5), (T::Win(new), 0.5)] };
            *t = T::Split { horizontal, kids };
            true
          }
          T::Win(_) => false,
          T::Split { kids, .. } => kids.iter_mut().any(|(k, _)| Self::replace(k, target, new, first, horizontal)),
        }
      }

      /// dwindle_place: the focused window, else the closest to the
      /// cursor; split along its node box, the new window on the point's
      /// half.
      pub fn add(&mut self, w: u32, active: Option<u32>, mouse: (f64, f64), work: Bx, focal: Option<(f64, f64)>) {
        let boxes = self.boxes(work);
        let point = focal.unwrap_or(mouse);
        let target = if focal.is_some() {
          closest(point, &boxes.iter().map(|(_, b)| *b).collect::<Vec<_>>()).map(|i| boxes[i].0)
        } else {
          active.filter(|a| boxes.iter().any(|(w, _)| w == a)).or_else(|| closest(mouse, &boxes.iter().map(|(_, b)| *b).collect::<Vec<_>>()).map(|i| boxes[i].0))
        };
        let Some(target) = target else {
          self.root = Some(T::Win(w));
          return;
        };
        let tbox = boxes.iter().find(|(id, _)| *id == target).unwrap().1;
        let first = new_is_first(tbox, point);
        Self::replace(self.root.as_mut().unwrap(), target, w, first, side_by_side(tbox));
      }

      fn remove_in(t: &mut T, w: u32) -> bool {
        if let T::Split { kids, .. } = t {
          if let Some(i) = kids.iter().position(|(k, _)| matches!(k, T::Win(x) if *x == w)) {
            let removed = kids.remove(i).1;
            // detach_container: the split partner takes the share
            let j = i.min(kids.len() - 1);
            kids[j].1 += removed;
            if kids.len() == 1 {
              // flatten: the child takes the split's place
              *t = kids.remove(0).0;
            }
            return true;
          }
          return kids.iter_mut().any(|(k, _)| Self::remove_in(k, w));
        }
        false
      }

      pub fn remove(&mut self, w: u32) {
        match &mut self.root {
          Some(T::Win(x)) if *x == w => self.root = None,
          Some(t) => {
            Self::remove_in(t, w);
          }
          None => {}
        }
      }

      fn ratio_in(t: &mut T, w: u32, delta: f64) -> bool {
        if let T::Split { kids, .. } = t {
          if kids.len() == 2 && kids.iter().any(|(k, _)| matches!(k, T::Win(x) if *x == w)) {
            let first = share_from_ratio(ratio_from_share(kids[0].1) + delta);
            kids[0].1 = first;
            kids[1].1 = 1. - first;
            return true;
          }
          return kids.iter_mut().any(|(k, _)| Self::ratio_in(k, w, delta));
        }
        false
      }

      pub fn split_ratio(&mut self, w: u32, delta: f64) {
        if let Some(t) = &mut self.root {
          Self::ratio_in(t, w, delta);
        }
      }

      pub fn rects(&self, work: Bx) -> Vec<(u32, (i32, i32, i32, i32))> {
        let mut out: Vec<_> = self.boxes(work).into_iter().map(|(w, b)| (w, window_box(b, work, super::GAPS))).collect();
        out.sort_by_key(|(w, _)| *w);
        out
      }
    }
  }

  /// A small deterministic generator (no external crates).
  struct Rng(u64);
  impl Rng {
    fn next(&mut self) -> u64 {
      self.0 ^= self.0 << 13;
      self.0 ^= self.0 >> 7;
      self.0 ^= self.0 << 17;
      self.0
    }
    fn below(&mut self, n: u64) -> u64 {
      self.next() % n
    }
    fn f(&mut self, lo: f64, hi: f64) -> f64 {
      #[allow(clippy::cast_precision_loss)]
      let u = (self.next() % 1_000_000) as f64 / 1_000_000.;
      lo + (hi - lo) * u
    }
  }

  fn same(a: &[(u32, (i32, i32, i32, i32))], b: &[(u32, (i32, i32, i32, i32))]) -> bool {
    a.len() == b.len()
      && a.iter().zip(b).all(|((wa, ra), (wb, rb))| {
        wa == wb && (ra.0 - rb.0).abs() <= 1 && (ra.1 - rb.1).abs() <= 1 && (ra.2 - rb.2).abs() <= 1 && (ra.3 - rb.3).abs() <= 1
      })
  }

  /// Work areas: 1920x1080 under a 40px bar with gaps_out 5, a 1280x1024
  /// side monitor, a portrait 1080x1920 and an ultrawide.
  fn works() -> Vec<Bx> {
    vec![
      Bx::new(5., 45., 1910., 1030.),
      Bx::new(-1275., 59., 1270., 960.),
      Bx::new(5., 45., 1070., 1870.),
      Bx::new(5., 45., 3430., 1390.),
    ]
  }

  #[test]
  fn open_close_resize_sequences_match_hyprland() {
    let mut runs = 0;
    for (wi, work) in works().into_iter().enumerate() {
      for seed in 1..=400u64 {
        let mut rng = Rng(seed * 7919 + wi as u64 * 104_729);
        let mut h = hypr::Dwindle::default();
        let mut o = ours::Tree { root: None };
        let mut open: Vec<u32> = vec![];
        let mut next_id = 1u32;
        let mut active: Option<u32> = None;

        for _step in 0..30 {
          let mouse = (rng.f(work.x - 50., work.x + work.w + 50.), rng.f(work.y - 50., work.y + work.h + 50.));
          match rng.below(10) {
            0..=4 if open.len() < 8 => {
              let w = next_id;
              next_id += 1;
              // the focused window: one of the open ones, sometimes none (a
              // floating or other-workspace window had focus)
              let act = if rng.below(5) == 0 || open.is_empty() { None } else { Some(open[rng.below(open.len() as u64) as usize]) };
              h.add(w, act, mouse, work, None);
              o.add(w, act, mouse, work, None);
              open.push(w);
              active = Some(w);
            }
            5..=6 if !open.is_empty() => {
              let i = rng.below(open.len() as u64) as usize;
              let w = open.remove(i);
              h.remove(w, work);
              o.remove(w);
              if active == Some(w) {
                active = open.last().copied();
              }
            }
            7 if !open.is_empty() => {
              let w = open[rng.below(open.len() as u64) as usize];
              let delta = [0.1, -0.1, 0.05, -0.05][rng.below(4) as usize];
              h.split_ratio(w, delta, work);
              o.split_ratio(w, delta);
            }
            8 if open.len() > 1 => {
              // movewindow: removed, then re-added at the focal point
              let w = open[rng.below(open.len() as u64) as usize];
              let focal = (rng.f(work.x, work.x + work.w), rng.f(work.y, work.y + work.h));
              h.remove(w, work);
              o.remove(w);
              h.add(w, None, mouse, work, Some(focal));
              o.add(w, None, mouse, work, Some(focal));
            }
            _ => {}
          }
          let _ = active;
          let hr = h.rects(work);
          let or = o.rects(work);
          assert!(same(&hr, &or), "work {wi} seed {seed}\nhyprland {hr:?}\nours     {or:?}");
          runs += 1;
        }
      }
    }
    assert!(runs > 40_000);
  }

  #[test]
  fn two_windows_are_eight_pixels_apart_and_touch_the_work_area() {
    let work = Bx::new(5., 45., 1910., 1030.);
    let kids = partition(work, true, &[0.5, 0.5]);
    let a = window_box(kids[0], work, GAPS);
    let b = window_box(kids[1], work, GAPS);
    assert_eq!(a, (5, 45, 951, 1030));
    assert_eq!(b, (964, 45, 951, 1030));
    assert_eq!(b.0 - (a.0 + a.2), 8);
  }

  #[test]
  fn uneven_split_keeps_the_ratio_between_node_boxes() {
    // ratio 1.2: the first node box is 60% of the work area, then gaps
    let work = Bx::new(0., 0., 2000., 1000.);
    let kids = partition(work, true, &[share_from_ratio(1.2), 1. - share_from_ratio(1.2)]);
    assert_eq!(window_box(kids[0], work, GAPS), (0, 0, 1196, 1000));
    assert_eq!(window_box(kids[1], work, GAPS), (1204, 0, 796, 1000));
  }

  #[test]
  fn new_window_side_follows_the_point_even_outside_the_box() {
    let b = Bx::new(1000., 0., 800., 600.);
    assert!(side_by_side(b));
    assert!(new_is_first(b, (1100., 300.)));
    assert!(!new_is_first(b, (1500., 300.)));
    // left of the box: first half, as Hyprland
    assert!(new_is_first(b, (10., 300.)));
    // no cursor (managed on startup): far right, the second half
    assert!(!new_is_first(b, (f64::MAX, f64::MAX)));
    let tall = Bx::new(0., 0., 600., 800.);
    assert!(!side_by_side(tall));
    assert!(new_is_first(tall, (300., 100.)));
    assert!(!new_is_first(tall, (300., 700.)));
  }

  #[test]
  fn square_box_splits_top_and_bottom() {
    // w > h * 1 is strict: a square box splits top/bottom
    assert!(!side_by_side(Bx::new(0., 0., 500., 500.)));
  }

  #[test]
  fn closest_prefers_inside_then_distance_then_first() {
    let boxes = [Bx::new(0., 0., 100., 100.), Bx::new(200., 0., 100., 100.)];
    assert_eq!(closest((50., 50.), &boxes), Some(0));
    assert_eq!(closest((260., 50.), &boxes), Some(1));
    assert_eq!(closest((150., 50.), &boxes), Some(0));
    assert_eq!(closest((50., 50.), &[]), None);
  }
}
