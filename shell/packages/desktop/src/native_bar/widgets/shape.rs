//! Pure geometry shared by painting, content placement, resizing and the
//! native window region. Coordinates are widget-local DIPs.
use super::{layout::{self, Kind}, super::gfx::Rect};
use std::f32::consts::{PI, TAU};

pub const SHAPES: [(&str, &str); 8] = [
  ("card", "Kart"), ("capsule", "Kapsül"), ("circle", "Daire"), ("ticket", "Bilet"),
  ("bubble", "Konuşma balonu"), ("hexagon", "Altıgen"), ("polaroid", "Polaroid"), ("split", "Ayrık paneller"),
];

#[derive(Clone, Debug)]
pub struct Plan {
  pub contours: Vec<Vec<(f32, f32)>>,
  pub inner: Rect,
  pub lead: Option<Rect>,
  pub caption: Option<Rect>,
  pub settings: Rect,
  pub grip: Rect,
  pub vertical: bool,
  pub divider: Option<f32>,
}

pub fn minimum(kind: Kind, shape: &str) -> (f32, f32) {
  let (w, h) = kind.min_size();
  match shape {
    "circle" => { let side = w.max(if kind == Kind::Agenda { 264.0 } else { 224.0 }); (side, side) }
    "capsule" => ((w + 96.0).max((h + 48.0)*1.9), h + 48.0),
    "ticket" => (w + 104.0, h + 32.0),
    "bubble" => (w + 32.0, h + 56.0),
    "hexagon" => (w / 0.64 + 12.0, h / 0.64 + 32.0),
    "polaroid" => (w + 36.0, (h + 90.0).max(if kind == Kind::Media { 252.0 } else { 224.0 })),
    "split" => (w + 116.0, h + 32.0),
    _ => (w, h),
  }
}

pub fn preferred(kind: Kind, form: &str) -> (f32,f32) {
  if kind == Kind::Weather {
    match form {
      "capsule" => (320.0,144.0), "ticket" => (320.0,168.0), "bubble" => (264.0,168.0),
      "circle" => (248.0,248.0), "hexagon" => (288.0,240.0), "polaroid" => (248.0,288.0), "split" => (320.0,168.0),
      _ => kind.default_size(),
    }
  } else { kind.default_size() }
}

pub fn clamp(kind: Kind, shape: &str, r: (f32, f32, f32, f32), aw: f32, ah: f32) -> (f32, f32, f32, f32) {
  if shape == "card" { return layout::clamp(kind, r, aw, ah); }
  let (mw, mh) = minimum(kind, shape);
  let mut w = r.2.max(mw).min(aw.max(1.0)); let mut h = r.3.max(mh).min(ah.max(1.0));
  if shape == "circle" { let side = r.2.max(r.3).max(mw).min(aw.min(ah).max(1.0)); w = side; h = side; }
  if shape == "capsule" { w = w.max(h * 1.9).min(aw.max(1.0)); h = h.min(w / 1.9); }
  (r.0.max(0.0).min((aw - w).max(0.0)), r.1.max(0.0).min((ah - h).max(0.0)), w, h)
}

pub fn circle_resize(side: f32, dx: f32, dy: f32) -> f32 {
  layout::snap(side + if dx.abs() >= dy.abs() { dx } else { dy })
}

fn rounded(r: Rect, radius: f32) -> Vec<(f32, f32)> {
  let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
  let mut points = Vec::new();
  for (cx, cy, start) in [(r.right()-radius, r.y+radius, -PI/2.0), (r.right()-radius, r.bottom()-radius, 0.0),
    (r.x+radius, r.bottom()-radius, PI/2.0), (r.x+radius, r.y+radius, PI)] {
    for i in 0..=12 { let a = start + i as f32 * PI / 24.0; points.push((cx + radius*a.cos(), cy + radius*a.sin())); }
  }
  points
}

pub fn plan(shape: &str, w: f32, h: f32) -> Plan {
  let card = Rect::new(0.0, 0.0, w, h);
  let mut plan = Plan { contours: vec![rounded(card, 24.0)], inner: card.inset(16.0, 14.0), lead: None,
    caption: None, settings: Rect::new(w-40.0, 8.0, 24.0, 24.0), grip: Rect::new(w-34.0, h-32.0, 18.0, 18.0),
    vertical: false, divider: None };
  match shape {
    "capsule" => {
      plan.contours = vec![rounded(card, h/2.0)]; plan.inner = card.inset(h*0.25, (h*0.18).max(18.0));
      plan.settings = Rect::new(w-h*0.25-24.0, h*0.18, 24.0, 24.0);
      plan.grip = Rect::new(w-h*0.25-18.0, h*0.82-18.0, 18.0, 18.0);
    }
    "circle" => {
      plan.contours = vec![(0..128).map(|i| { let a = i as f32*TAU/128.0; (w/2.0+w/2.0*a.cos(), h/2.0+h/2.0*a.sin()) }).collect()];
      plan.inner = card.inset(w*0.16, h*0.16); plan.vertical = true;
      plan.settings = Rect::new(w*0.5-12.0, h*0.065, 24.0, 24.0);
      plan.grip = Rect::new(w*0.74-9.0, h*0.77-9.0, 18.0, 18.0);
    }
    "ticket" => {
      let radius = 14.0f32.min(h*0.12); let cy = h*0.5;
      let mut points = vec![(0.0, 0.0), (w, 0.0), (w, cy-radius)];
      for i in 0..=32 { let a = -PI/2.0 + i as f32*PI/32.0; points.push((w-radius*a.cos(), cy+radius*a.sin())); }
      points.extend([(w, h), (0.0, h), (0.0, cy+radius)]);
      for i in 0..=32 { let a = PI/2.0-i as f32*PI/32.0; points.push((radius*a.cos(), cy+radius*a.sin())); }
      plan.contours = vec![points]; plan.divider = Some(w*0.30);
      plan.lead = Some(Rect::new(24.0, 18.0, w*0.30-36.0, h-36.0));
      plan.inner = Rect::new(w*0.30+16.0, 18.0, w*0.70-40.0, h-36.0);
    }
    "bubble" => {
      let body = Rect::new(0.0, 0.0, w, h-26.0);
      plan.contours = vec![rounded(body, 24.0), vec![(w*0.24, h-30.0), (w*0.42, h-30.0), (w*0.28, h)]];
      plan.inner = body.inset(18.0, 18.0); plan.grip.y = body.bottom()-24.0;
    }
    "hexagon" => {
      plan.contours = vec![vec![(w*0.14, 0.0), (w*0.86, 0.0), (w, h*0.5), (w*0.86, h), (w*0.14, h), (0.0, h*0.5)]];
      plan.inner = card.inset(w*0.18, h*0.18); plan.vertical = true;
      plan.settings = Rect::new(w*0.5-12.0, 5.0, 24.0, 24.0);
      plan.grip = Rect::new(w*0.78-9.0, h*0.84-9.0, 18.0, 18.0);
    }
    "polaroid" => {
      plan.contours = vec![rounded(Rect::new(0.0, 18.0, w, h-18.0), 3.0),
        vec![(w*0.35, 3.0), (w*0.63, 0.0), (w*0.65, 28.0), (w*0.37, 31.0)]];
      plan.inner = Rect::new(16.0, 36.0, w-32.0, h-92.0);
      plan.caption = Some(Rect::new(18.0, h-44.0, w-36.0, 24.0)); plan.vertical = true;
      plan.settings.y = 22.0;
    }
    "split" => {
      let left = Rect::new(0.0, 0.0, w*0.28, h); let right = Rect::new(w*0.28+12.0, 0.0, w*0.72-12.0, h);
      plan.contours = vec![rounded(left, 18.0), rounded(right, 18.0)];
      plan.lead = Some(left.inset(12.0, 14.0)); plan.inner = right.inset(14.0, 14.0);
    }
    _ => {},
  }
  plan
}

impl Plan {
  pub fn contains(&self, x: f32, y: f32) -> bool {
    self.contours.iter().any(|points| {
      let mut inside = false; let mut j = points.len()-1;
      for i in 0..points.len() {
        let (ax, ay) = points[i]; let (bx, by) = points[j];
        if (ay > y) != (by > y) && x < (bx-ax)*(y-ay)/(by-ay)+ax { inside = !inside; }
        j = i;
      }
      inside
    })
  }
  pub fn contains_rect(&self, r: Rect) -> bool {
    [(r.x+0.1, r.y+0.1), (r.right()-0.1, r.y+0.1), (r.x+0.1, r.bottom()-0.1), (r.right()-0.1, r.bottom()-0.1)]
      .into_iter().all(|(x,y)| self.contains(x,y))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn switching_shapes_does_not_accumulate_size_and_keeps_manual_resize() {
    for kind in layout::KINDS {
      let mut spec = layout::Spec::new(1,kind,"");
      let original = (spec.w,spec.h);
      for _ in 0..5 {
        for (form,_) in SHAPES { spec.select_shape(form,1920.0,1080.0); }
        spec.select_shape("card",1920.0,1080.0);
        assert_eq!((spec.w,spec.h),original,"{kind:?} grew while browsing shapes");
      }
      spec.select_shape("capsule",1920.0,1080.0);
      (spec.w,spec.h) = (640.0,192.0);
      spec.select_shape("circle",1920.0,1080.0);
      spec = layout::Store::parse(&serde_json::to_string(&layout::Store { version:1,widgets:vec![spec] }).unwrap()).unwrap().widgets.remove(0);
      spec.select_shape("capsule",1920.0,1080.0);
      assert_eq!((spec.w,spec.h),(640.0,192.0));
    }
  }
  #[test]
  fn safe_content_and_controls_are_inside_every_contour_for_all_kinds() {
    for kind in layout::KINDS { for (shape, _) in SHAPES {
      let (w,h) = minimum(kind,shape); let p = plan(shape,w,h);
      for rect in [Some(p.inner), p.lead, p.caption, Some(p.settings), Some(p.grip)].into_iter().flatten() {
        assert!(rect.w > 0.0 && rect.h > 0.0 && p.contains_rect(rect), "{kind:?}/{shape}: {rect:?}");
      }
    } }
  }
  #[test]
  fn contour_holes_do_not_intercept_the_desktop() {
    assert!(!plan("circle",224.0,224.0).contains(5.0,5.0));
    assert!(!plan("ticket",320.0,144.0).contains(3.0,72.0));
    assert!(!plan("split",400.0,144.0).contains(117.0,72.0));
    assert!(!plan("bubble",280.0,180.0).contains(240.0,176.0));
    assert!(plan("bubble",280.0,180.0).contains(78.0,175.0));
    assert!(!plan("polaroid",280.0,224.0).contains(10.0,5.0));
    assert!(plan("polaroid",280.0,224.0).contains(140.0,8.0));
  }
  #[test]
  fn circle_selection_and_resizing_stay_square_inside_monitor() {
    let r = clamp(Kind::Weather,"circle",(900.0,700.0,248.0,128.0),1024.0,768.0);
    assert_eq!(r.2,r.3); assert_eq!(r.2,248.0); assert!(r.0+r.2 <= 1024.0 && r.1+r.3 <= 768.0);
    assert_eq!(circle_resize(248.0,-32.0,0.0),216.0);
    let side = circle_resize(248.0,40.0,80.0); assert_eq!(side,328.0);
    let small = clamp(Kind::Media,"circle",(0.0,0.0,side,side),180.0,150.0);
    assert_eq!((small.2,small.3),(150.0,150.0));
  }
}
