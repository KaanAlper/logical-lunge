//! A quick settings tile being moved (edit mode): a long press (350 ms) or
//! a small move lifts it onto its own compositor visual, a little bigger
//! with a shadow. While it follows the pointer only that visual's offset
//! changes (nothing is redrawn under it); the panel is drawn again only
//! when the drop place changes, and the other tiles slide to their new
//! places. On release the tile lands in its slot (or back where it was,
//! after Esc or a drop outside the tiles); the order is saved.

use windows::{core::Interface, Win32::Graphics::DirectComposition::IDCompositionVisual3};

use super::*;
use crate::native_bar::{anim, gfx};

/// room around the lifted tile for its shadow (DIPs)
pub(in crate::native_bar::sidebar) const GHOST_M: f32 = 24.0;
/// the ghost surface's height (DIPs)
pub(in crate::native_bar::sidebar) const GHOST_H: f32 = CELL_H * 1.04 + 2.0 * GHOST_M;
/// lifted: 4 % bigger
const LIFT: f32 = 1.04;

impl Ui {
  /// A pointer move (or a due long press) with a tile pressed.
  pub(in crate::native_bar::sidebar) fn sb_tile_drag(&mut self, x: f32, y: f32, lift: bool) {
    let tr = |s: &str| self.model.tr(s);
    match self.sidebar.quick.drag_move(&self.model, &tr, x, y, lift) {
      Moved::Nothing => {}
      Moved::Lifted => {
        if let Err(err) = self.sb_ghost_paint() {
          tracing::warn!("Sidebar: lifted tile: {:?}", err);
        }
        self.sb_ghost_follow();
        self.sb_render();
      }
      Moved::Moved => self.sb_ghost_follow(),
      Moved::Retargeted => {
        self.sb_ghost_follow();
        self.sb_render();
      }
    }
  }

  /// The button went up (`drop`) or the drag was cancelled. True when a
  /// lifted tile was put down (no click follows).
  pub(in crate::native_bar::sidebar) fn sb_tile_drop(&mut self, drop: bool) -> bool {
    let tr = |s: &str| self.model.tr(s);
    let Some((tile, changed)) = self.sidebar.quick.release(&self.model, &tr, drop) else {
      self.sb_render();
      return false;
    };
    if changed {
      self.sidebar.store.quick_toggles = Some(self.sidebar.quick.toggles.clone());
      self.sidebar.save_soon();
    }
    // the slots as they are now, the tile itself not drawn yet
    self.sb_render();
    if !self.model.animations || self.sb_ghost_land(tile).is_err() {
      self.sidebar.quick.landing = None;
      self.sb_render();
      self.sb_ghost_hide();
    } else {
      self.sb_frames();
    }
    true
  }

  /// Every frame: a long press lifts its tile; a landing that is over
  /// gives the tile back to the panel.
  pub(in crate::native_bar::sidebar) fn sb_tiles_frame(&mut self) {
    if self.sidebar.drag == Some(crate::native_bar::sidebar::Drag::Tile) {
      if let Some((x, y)) = self.sidebar.quick.lift_due() {
        self.sb_tile_drag(x, y, true);
      }
    }
    if let Some((_, at)) = self.sidebar.quick.landing {
      if at.elapsed().as_secs_f32() * 1000.0 >= SLIDE_MS + 10.0 {
        // the tile is drawn in its slot first, then the ghost goes: no
        // frame without either
        self.sidebar.quick.landing = None;
        self.sb_render();
        self.sb_ghost_hide();
      }
    }
  }

  /// Draws the lifted tile once onto the ghost surface and shows it.
  fn sb_ghost_paint(&mut self) -> anyhow::Result<()> {
    let theme = self.theme();
    let Ui { gfx, fonts, res, icons, model, sidebar, .. } = self;
    let Some(w) = &sidebar.win else { return Ok(()) };
    let Some(d) = &sidebar.quick.drag else { return Ok(()) };
    let r = Rect::new(GHOST_M, GHOST_M, d.w * LIFT, d.h * LIFT);
    let (surface, scale) = (w.ghost.surface.clone(), w.scale);
    let tr = |s: &str| model.tr(s);
    let mut requests = Vec::new();
    let (mut hits, mut regions) = (Vec::new(), Vec::new());
    gfx::draw_surface(&surface, scale, |dc| {
      let mut p = crate::native_bar::view::Painter { dc, gfx, fonts, res, icons, requests: &mut requests };
      let mut cx = Cx::new(&mut p, theme, &mut hits, &mut regions, None, None, None, &tr, 0.0);
      if let Err(err) = sidebar.quick.paint_ghost(&mut cx, model, r) {
        tracing::warn!("Sidebar: ghost: {:?}", err);
      }
      Ok(())
    })?;
    unsafe {
      w.ghost_scale.SetScaleX2(1.0)?;
      w.ghost_scale.SetScaleY2(1.0)?;
      let v: IDCompositionVisual3 = w.ghost.visual.cast()?;
      v.SetOpacity2(1.0)?;
    }
    Ok(())
  }

  /// The ghost's top left (pixels) for the pointer, keeping the grab offset.
  fn sb_ghost_at(&self) -> Option<(f32, f32)> {
    let w = self.sidebar.win.as_ref()?;
    let d = self.sidebar.quick.drag.as_ref()?;
    let grow_x = d.w * (LIFT - 1.0) / 2.0;
    let grow_y = d.h * (LIFT - 1.0) / 2.0;
    Some((((d.x - d.dx - grow_x - GHOST_M) * w.scale).round(), ((d.y - d.dy - grow_y - GHOST_M) * w.scale).round()))
  }

  /// Moves the ghost with the pointer: an offset and a commit, no drawing.
  fn sb_ghost_follow(&mut self) {
    let Some((x, y)) = self.sb_ghost_at() else { return };
    let Some(w) = &self.sidebar.win else { return };
    unsafe {
      let _ = w.ghost.visual.SetOffsetX2(x);
      let _ = w.ghost.visual.SetOffsetY2(y);
      let _ = self.gfx.dcomp.Commit();
    }
    self.sidebar.ghost_at = (x, y);
  }

  /// Animates the ghost into the tile's slot, shrinking back to its size.
  fn sb_ghost_land(&mut self, tile: Tile) -> windows::core::Result<()> {
    let Some(w) = &self.sidebar.win else { return Ok(()) };
    let Some(slot) = self.sidebar.quick.tile_rect(tile) else {
      return Err(windows::core::Error::from(windows::Win32::Foundation::E_FAIL));
    };
    let dcomp = &self.gfx.dcomp;
    let s = w.scale;
    let (fx, fy) = self.sidebar.ghost_at;
    let (tx, ty) = (((slot.x - GHOST_M) * s).round(), ((slot.y - GHOST_M) * s).round());
    unsafe {
      w.ghost.visual.SetOffsetX(&anim::build(dcomp, fx, tx, SLIDE_MS, anim::POP_IN)?)?;
      w.ghost.visual.SetOffsetY(&anim::build(dcomp, fy, ty, SLIDE_MS, anim::POP_IN)?)?;
      // the lifted size back to the slot's, from the tile's top left
      w.ghost_scale.SetCenterX2(GHOST_M * s)?;
      w.ghost_scale.SetCenterY2(GHOST_M * s)?;
      let k = slot.w / (slot.w * LIFT).max(1.0);
      w.ghost_scale.SetScaleX(&anim::build(dcomp, 1.0, k, SLIDE_MS, anim::POP_IN)?)?;
      w.ghost_scale.SetScaleY(&anim::build(dcomp, 1.0, k, SLIDE_MS, anim::POP_IN)?)?;
      dcomp.Commit()
    }
  }

  fn sb_ghost_hide(&mut self) {
    let Some(w) = &self.sidebar.win else { return };
    unsafe {
      if let Ok(v) = w.ghost.visual.cast::<IDCompositionVisual3>() {
        let _ = v.SetOpacity2(0.0);
      }
      let _ = self.gfx.dcomp.Commit();
    }
  }
}
