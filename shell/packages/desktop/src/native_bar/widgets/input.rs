//! A widget's mouse and menu: moving and resizing on the grid (onto
//! another monitor too), the media buttons, the right-click menu and the
//! settings it changes.

use super::*;

impl Ui {
  // ------------------------------------------------------------ input

  pub(in crate::native_bar) fn widgets_msg(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Option<LRESULT>> {
    let wi = self.widgets.wins.iter().position(|w| w.hwnd == hwnd)?;
    let id = self.widgets.wins[wi].id;
    if msg != 0 && msg == self.widgets.taskbar_created {
      // Explorer started again: a new desktop window to sit above
      place_above_desktop(hwnd);
      return Some(None);
    }
    let point = |lp: LPARAM, scale: f32| ((lp.0 & 0xFFFF) as i16 as f32 / scale, ((lp.0 >> 16) & 0xFFFF) as i16 as f32 / scale);
    let hit_at = |w: &Win, x: f32, y: f32| w.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| *h);
    match msg {
      WM_PAINT => {
        unsafe {
          let _ = ValidateRect(hwnd, None);
        }
        Some(Some(LRESULT(0)))
      }
      WM_MOUSEACTIVATE => {
        let editing = self.widgets.editor.as_ref().is_some_and(|e| e.id == id);
        Some(Some(LRESULT(if editing { MA_ACTIVATE as isize } else { MA_NOACTIVATE as isize })))
      }
      WM_MOUSEMOVE => {
        if self.widgets.wins[wi].drag.is_some() {
          self.widget_drag(wi);
          return Some(Some(LRESULT(0)));
        }
        let w = &mut self.widgets.wins[wi];
        if !w.tracking {
          let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
          unsafe {
            let _ = TrackMouseEvent(&mut tme);
          }
          w.tracking = true;
        }
        let (x, y) = point(lp, w.scale);
        w.hot = hit_at(&*w, x, y);
        let changed = !w.hover;
        w.hover = true;
        if changed {
          self.widget_paint(id, false);
        }
        Some(Some(LRESULT(0)))
      }
      WM_MOUSELEAVE => {
        let w = &mut self.widgets.wins[wi];
        w.tracking = false;
        w.hot = None;
        if w.drag.is_none() {
          w.hover = false;
          self.widget_paint(id, false);
        }
        Some(Some(LRESULT(0)))
      }
      WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
        let w = &self.widgets.wins[wi];
        let editing = self.widgets.editor.as_ref().is_some_and(|e| e.id == id);
        let shape = match w.hot {
          Some(Hit::Grip) => IDC_SIZENWSE,
          Some(Hit::Prev | Hit::Play | Hit::Next) => IDC_HAND,
          Some(Hit::Text) if editing || self.spec(id).is_some_and(|s| s.kind == Kind::Note) => IDC_IBEAM,
          _ if w.drag.as_ref().is_some_and(|d| d.moved && !d.resize) => IDC_SIZEALL,
          _ => IDC_ARROW,
        };
        unsafe {
          if let Ok(c) = LoadCursorW(None, shape) {
            SetCursor(c);
          }
        }
        Some(Some(LRESULT(1)))
      }
      WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
        let (x, y) = point(lp, self.widgets.wins[wi].scale);
        let hit = hit_at(&self.widgets.wins[wi], x, y);
        match hit {
          Some(Hit::Prev) => self.widget_media(MediaFunction::Previous),
          Some(Hit::Play) => self.widget_media(MediaFunction::TogglePlayPause),
          Some(Hit::Next) => self.widget_media(MediaFunction::Next),
          Some(Hit::Text) => self.widget_text_click(id, x, y, msg == WM_LBUTTONDBLCLK),
          other => {
            // anything else moves it; the corner resizes it
            if self.widgets.editor.as_ref().is_some_and(|e| e.id == id) {
              self.widgets_end_edit(true);
            }
            let Some(s) = self.spec(id) else { return Some(Some(LRESULT(0))) };
            let start = s.rect();
            let w = &mut self.widgets.wins[wi];
            w.drag = Some(Drag { resize: other == Some(Hit::Grip), from: cursor(), start, moved: false });
            unsafe {
              SetCapture(hwnd);
            }
          }
        }
        Some(Some(LRESULT(0)))
      }
      WM_LBUTTONUP => {
        if self.widgets.wins[wi].drag.take().is_some() {
          unsafe {
            let _ = ReleaseCapture();
          }
          self.widgets_save_soon();
          self.widget_paint(id, false);
        }
        Some(Some(LRESULT(0)))
      }
      WM_CAPTURECHANGED => {
        if self.widgets.wins[wi].drag.take().is_some() {
          self.widgets_save_soon();
          self.widget_paint(id, false);
        }
        Some(Some(LRESULT(0)))
      }
      WM_RBUTTONUP => {
        self.widget_menu(id, cursor());
        Some(Some(LRESULT(0)))
      }
      WM_KEYDOWN | WM_SYSKEYDOWN if self.widgets.editor.as_ref().is_some_and(|e| e.id == id) => {
        if self.widget_key_down(id, wp.0 as u16) {
          Some(Some(LRESULT(0)))
        } else {
          Some(None)
        }
      }
      WM_CHAR if self.widgets.editor.as_ref().is_some_and(|e| e.id == id) => {
        self.widget_char(id, wp.0 as u16);
        Some(Some(LRESULT(0)))
      }
      WM_KILLFOCUS if self.widgets.editor.as_ref().is_some_and(|e| e.id == id) => {
        self.widgets_end_edit(true);
        Some(Some(LRESULT(0)))
      }
      _ => None,
    }
  }

  pub(super) fn widget_media(&self, f: fn(MediaControlArgs) -> MediaFunction) {
    let session_id = self.model.media.as_ref().and_then(|m| m.current_session.as_ref()).map(|s| s.session_id.clone());
    self.provider("media", ProviderFunction::Media(f(MediaControlArgs { session_id })));
  }

  /// Moving or resizing: follows the cursor on the grid, onto whichever
  /// monitor it is over, kept on screen.
  pub(super) fn widget_drag(&mut self, wi: usize) {
    let id = self.widgets.wins[wi].id;
    let Some(kind) = self.spec(id).map(|s| s.kind) else { return };
    let (d_from, d_start, resize) = match &self.widgets.wins[wi].drag {
      Some(d) => (d.from, d.start, d.resize),
      None => return,
    };
    let now = cursor();
    let mons = monitors();
    let here = mon_for(&mons, &self.widgets.wins[wi].device);
    let Some(cur) = here else { return };
    let scale = crate::native_bar::scale::of_dpi(cur.dpi);
    let (dx, dy) = ((now.x - d_from.x) as f32 / scale, (now.y - d_from.y) as f32 / scale);
    if dx.abs() < 2.0 && dy.abs() < 2.0 && !self.widgets.wins[wi].drag.as_ref().is_some_and(|d| d.moved) {
      return;
    }
    if let Some(d) = self.widgets.wins[wi].drag.as_mut() {
      d.moved = true;
    }
    let (mut target, mut rect) = (cur.clone(), d_start);
    if resize {
      rect.2 = layout::snap(d_start.2 + dx);
      rect.3 = layout::snap(d_start.3 + dy);
    } else {
      rect.0 = layout::snap(d_start.0 + dx);
      rect.1 = layout::snap(d_start.1 + dy);
      // dragged onto another monitor: it moves there (in that monitor's DIPs)
      let over = unsafe { MonitorFromPoint(now, MONITOR_DEFAULTTONEAREST) };
      let mut info = MONITORINFOEXW::default();
      info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
      if unsafe { GetMonitorInfoW(over, &mut info as *mut _ as *mut _) }.as_bool() {
        let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        let device = String::from_utf16_lossy(&info.szDevice[..end]);
        if !device.eq_ignore_ascii_case(&cur.device) {
          if let Some(m) = mon_for(&mons, &device) {
            let s2 = crate::native_bar::scale::of_dpi(m.dpi);
            // the grab point stays under the cursor
            let gx = (d_from.x - cur.work.left) as f32 / scale - d_start.0;
            let gy = (d_from.y - cur.work.top) as f32 / scale - d_start.1;
            rect.0 = layout::snap((now.x - m.work.left) as f32 / s2 - gx);
            rect.1 = layout::snap((now.y - m.work.top) as f32 / s2 - gy);
            target = m;
          }
        }
      }
    }
    let (aw, ah) = area_dip(&target);
    let clamped = layout::clamp(kind, rect, aw, ah);
    let moved_monitor = !target.device.eq_ignore_ascii_case(&cur.device);
    if let Some(s) = self.widgets.store.widgets.iter_mut().find(|s| s.id == id) {
      (s.x, s.y, s.w, s.h) = clamped;
      s.monitor = target.device.clone();
    }
    if moved_monitor {
      // the drag goes on from the new monitor's coordinates
      if let Some(d) = self.widgets.wins[wi].drag.as_mut() {
        d.from = now;
        d.start = clamped;
      }
    }
    self.widget_apply(wi, &target);
  }

  /// Puts widget window `wi` where its spec says (on monitor `m`); a new size
  /// gets a new surface and a draw.
  pub(super) fn widget_apply(&mut self, wi: usize, m: &Mon) {
    let id = self.widgets.wins[wi].id;
    let Some((x, y, w, h)) = self.spec(id).map(|s| s.rect()) else { return };
    let scale = crate::native_bar::scale::of_dpi(m.dpi);
    let (px, py) = (m.work.left + (x * scale).round() as i32, m.work.top + (y * scale).round() as i32);
    let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
    let win = &mut self.widgets.wins[wi];
    let resized = (pw, ph) != win.px || (scale - win.scale).abs() > 0.001;
    win.scale = scale;
    win.device = m.device.clone();
    win.work = m.work;
    unsafe {
      let _ = SetWindowPos(win.hwnd, None, px, py, pw as i32, ph as i32, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    if resized {
      match Layer::new(&self.gfx, pw.max(1), ph.max(1)) {
        Ok(layer) => {
          let win = &mut self.widgets.wins[wi];
          unsafe {
            let _ = win._root.RemoveAllVisuals();
            let _ = win._root.AddVisual(&layer.visual, false, None);
          }
          win.layer = layer;
          win.px = (pw, ph);
          self.widget_paint(id, true);
        }
        Err(err) => tracing::warn!("Desktop widget: surface: {:?}", err),
      }
    }
  }

  pub(super) fn widget_menu(&mut self, id: u64, at: POINT) {
    let Some(s) = self.spec(id).cloned() else { return };
    let tr = |t: &str| self.model.tr(t);
    let mut settings: Vec<MenuItem> = Vec::new();
    match s.kind {
      Kind::Clock => {
        settings.extend([
          MenuItem::new("style:digital", None, tr("Dijital")).checked(s.clock == ClockStyle::Digital),
          MenuItem::new("style:large", None, tr("Büyük")).checked(s.clock == ClockStyle::Large),
          MenuItem::new("style:analog", None, tr("Analog")).checked(s.clock == ClockStyle::Analog),
          MenuItem::sep(),
          MenuItem::new("seconds", None, tr("Saniyeleri göster")).checked(s.seconds),
          MenuItem::new("date", None, tr("Tarihi göster")).checked(s.date),
        ]);
      }
      Kind::System => settings.push(MenuItem::new("temps", None, tr("Sıcaklıkları göster")).checked(s.temps)),
      Kind::Weather => {
        settings.extend([
          MenuItem::new("city:auto", None, tr("Konumu saat diliminden al")).checked(s.city.is_empty()),
          MenuItem::new("city:edit", Some("edit_location"), tr("Konumu değiştir…")),
          MenuItem::sep(),
          MenuItem::new("fahrenheit", None, "°F").checked(s.fahrenheit),
          MenuItem::new("refresh", Some("refresh"), tr("Yenile")),
        ]);
      }
      Kind::Note => settings.push(MenuItem::new("edit", Some("edit_note"), tr("Düzenle"))),
      Kind::Media | Kind::Agenda => {}
    }
    let mut items = Vec::new();
    if !settings.is_empty() {
      items.push(MenuItem::new("settings", Some("tune"), tr("Ayarlar")).submenu(settings));
    }
    items.push(MenuItem::new("add", Some("widgets"), tr("Widget ekle")).submenu(self.widgets_add_menu()));
    items.push(MenuItem::sep());
    items.push(MenuItem::new("remove", Some("delete"), tr("Widget’ı kaldır")));
    self.menu_open(at, MenuFocus::Take, items, move |ui: &mut Ui, choice: &str| {
      if ui.widgets_pick(choice, at) {
        return;
      }
      ui.widget_setting(id, choice);
    });
  }

  pub(super) fn widget_setting(&mut self, id: u64, choice: &str) {
    if choice == "remove" {
      self.widget_remove(id);
      return;
    }
    if choice == "city:edit" {
      self.widgets_begin_edit(id, Target::City);
      return;
    }
    if choice == "edit" {
      self.widgets_begin_edit(id, Target::Note);
      return;
    }
    let mut refetch = false;
    let Some(s) = self.widgets.store.widgets.iter_mut().find(|s| s.id == id) else { return };
    match choice {
      "style:digital" => s.clock = ClockStyle::Digital,
      "style:large" => s.clock = ClockStyle::Large,
      "style:analog" => s.clock = ClockStyle::Analog,
      "seconds" => s.seconds = !s.seconds,
      "date" => s.date = !s.date,
      "temps" => s.temps = !s.temps,
      "city:auto" => {
        s.city.clear();
        refetch = true;
      }
      "fahrenheit" => {
        s.fahrenheit = !s.fahrenheit;
        refetch = true;
      }
      "refresh" => refetch = true,
      _ => return,
    }
    save(&self.widgets.store);
    if refetch {
      self.widgets.weather.remove(&id);
      self.widget_weather(id);
    }
    self.widgets_schedule();
    self.widget_paint(id, true);
  }
}
