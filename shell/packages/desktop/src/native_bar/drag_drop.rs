//! OLE drag target for the native workspace dots. The dragged data remains
//! owned by the source app; the bar only changes the displayed workspace.

use std::cell::Cell;
use windows::{
  core::{implement, Result},
  Win32::{
    Foundation::{HWND, LPARAM, POINT, POINTL, WPARAM},
    Graphics::Gdi::ScreenToClient,
    System::{
      Com::IDataObject,
      Ole::{
        IDropTarget, IDropTarget_Impl, RegisterDragDrop, RevokeDragDrop,
        DROPEFFECT, DROPEFFECT_NONE,
      },
      SystemServices::MODIFIERKEYS_FLAGS,
    },
    UI::WindowsAndMessaging::PostMessageW,
  },
};

use super::{WM_APP_DRAG_HOVER, WM_APP_DRAG_LEAVE};

#[implement(IDropTarget)]
struct Target {
  hwnd: HWND,
  last: Cell<Option<(i32, i32)>>,
}

impl Target {
  fn hover(&self, screen: &POINTL) {
    if self.last.get() == Some((screen.x, screen.y)) {
      return;
    }
    self.last.set(Some((screen.x, screen.y)));
    let mut point = POINT {
      x: screen.x,
      y: screen.y,
    };
    if !unsafe { ScreenToClient(self.hwnd, &mut point) }.as_bool() {
      return;
    }
    let packed = (point.x as u16 as u32) | ((point.y as u16 as u32) << 16);
    let _ = unsafe {
      PostMessageW(
        self.hwnd,
        WM_APP_DRAG_HOVER,
        WPARAM(0),
        LPARAM(packed as isize),
      )
    };
  }

  fn leave(&self) {
    self.last.set(None);
    let _ = unsafe {
      PostMessageW(self.hwnd, WM_APP_DRAG_LEAVE, WPARAM(0), LPARAM(0))
    };
  }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for Target_Impl {
  fn DragEnter(
    &self,
    _data: Option<&IDataObject>,
    _keys: MODIFIERKEYS_FLAGS,
    point: &POINTL,
    effect: *mut DROPEFFECT,
  ) -> Result<()> {
    unsafe {
      *effect = DROPEFFECT_NONE;
    }
    self.hover(point);
    Ok(())
  }

  fn DragOver(
    &self,
    _keys: MODIFIERKEYS_FLAGS,
    point: &POINTL,
    effect: *mut DROPEFFECT,
  ) -> Result<()> {
    unsafe {
      *effect = DROPEFFECT_NONE;
    }
    self.hover(point);
    Ok(())
  }

  fn DragLeave(&self) -> Result<()> {
    self.leave();
    Ok(())
  }

  fn Drop(
    &self,
    _data: Option<&IDataObject>,
    _keys: MODIFIERKEYS_FLAGS,
    _point: &POINTL,
    effect: *mut DROPEFFECT,
  ) -> Result<()> {
    unsafe {
      *effect = DROPEFFECT_NONE;
    }
    self.leave();
    Ok(())
  }
}

pub(super) fn register(hwnd: HWND) -> Result<IDropTarget> {
  let target: IDropTarget = Target {
    hwnd,
    last: Cell::new(None),
  }
  .into();
  unsafe {
    RegisterDragDrop(hwnd, &target)?;
  }
  Ok(target)
}

pub(super) unsafe fn revoke(hwnd: HWND) -> Result<()> {
  RevokeDragDrop(hwnd)
}
