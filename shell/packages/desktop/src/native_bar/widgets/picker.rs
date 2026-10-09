//! Modeless Win32 location picker. Its controls and draft live on a UI
//! thread; only an explicit Save sends a location back to the shell.
use super::{layout::Spec, location::{self, Choice, Draft, Field}, Ev, Msg};
use std::{sync::mpsc, time::Instant};
use windows::{core::{w, HSTRING, PCWSTR}, Win32::{
  Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM, RECT},
  Graphics::Gdi::*,
  System::LibraryLoader::GetModuleHandleW,
  UI::{Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED}, HiDpi::AdjustWindowRectExForDpi,
    Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus}, WindowsAndMessaging::*},
}};

const CLASS_NAME: PCWSTR = w!("LogicalLunge.WidgetLocation");
const RESULTS: usize = 110;
const RECENT: usize = 111;
const RETRY: usize = 112;
const WIDTH: i32 = 420;
const FIELD_Y: i32 = 84;
const ROW_HEIGHT: i32 = 40;
const PICKER_STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_POPUP.0 | WS_SYSMENU.0 | WS_CLIPCHILDREN.0);
type Answer = (u64, Result<Vec<Choice>, String>);

fn footer_y(recents: usize) -> i32 { if recents == 0 { 312 } else { 352 + recents.min(2) as i32 * ROW_HEIGHT } }
fn client_height(recents: usize) -> i32 { footer_y(recents) + 88 }

struct Picker {
  id: u64, token: u64, language: String, draft: Draft,
  edits: [HWND; 3], results: HWND, recent: HWND, status: HWND, save: HWND, retry: HWND,
  field_labels: [HWND;3], close: HWND,
  updating: bool, rx: mpsc::Receiver<Answer>, tx: mpsc::Sender<Answer>,
  labels: Vec<String>,
  theme: super::super::view::Theme, scale: f32, font: HFONT, background: HBRUSH, field_background: HBRUSH,
  small_font: HFONT, title_font: HFONT, popup_background: HBRUSH,
  controls: Vec<(HWND, (i32, i32, i32, i32))>,
  saved_rx: mpsc::Receiver<Result<(), String>>, saved_tx: mpsc::Sender<Result<(), String>>,
  save_busy: bool, save_error: Option<String>,
  handling: bool, ui_percent: u32,
}

fn color(c: super::super::gfx::Rgba) -> COLORREF { COLORREF(c.0 as u32 | ((c.1 as u32) << 8) | ((c.2 as u32) << 16)) }

/// Includes room for the frame; the whole layout shrinks on small monitors.
fn fit_scale(want: f32, width: i32, height: i32, recents: usize) -> f32 {
  want.min((width - 20).max(1) as f32 / WIDTH as f32).min((height - 20).max(1) as f32 / client_height(recents) as f32).max(0.1)
}

/// HWND is passed as an integer across threads; all controls belong to this thread.
pub fn open(spec: Spec, token: u64, owner: isize, language: String, labels: Vec<String>, theme: super::super::view::Theme) {
  let id = spec.id;
  let started = std::thread::Builder::new().name(format!("widget-location-{id}")).spawn(move || unsafe {
    let id = spec.id;
    let instance = match GetModuleHandleW(None) { Ok(i) => i, Err(_) => {
      super::super::send(Msg::Widgets(Ev::PickerClosed(id, token))); return;
    } };
    let class = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(),
      lpszClassName: CLASS_NAME, hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
      hbrBackground: HBRUSH::default(), ..Default::default() };
    RegisterClassW(&class);
    let (tx, rx) = mpsc::channel();
    let (saved_tx, saved_rx) = mpsc::channel();
    let mut state = Box::new(Picker { id, token, language, draft: Draft::new(&spec),
      edits: [HWND::default(); 3], results: HWND::default(), recent: HWND::default(), status: HWND::default(),
      field_labels:[HWND::default();3],close:HWND::default(),
      save: HWND::default(), retry: HWND::default(), updating: false, rx, tx, labels,
      theme, scale: 1.0, font: HFONT::default(), background: CreateSolidBrush(color(theme.layer0)),
      field_background: CreateSolidBrush(color(theme.surface_container_high)), controls: Vec::new(),
      small_font:HFONT::default(),title_font:HFONT::default(),popup_background:CreateSolidBrush(color(theme.layer1_hover)),
      saved_rx, saved_tx, save_busy: false, save_error: None, handling: false, ui_percent: super::super::scale::percent() });
    let mut rect = windows::Win32::Foundation::RECT::default();
    let owner_hwnd = HWND(owner as *mut _);
    let _ = GetWindowRect(owner_hwnd, &mut rect);
    // Keep this compact dialog within the owner's monitor work area.
    let monitors = super::monitors();
    let monitor = monitors.iter().find(|m| rect.left >= m.work.left && rect.left < m.work.right && rect.top >= m.work.top && rect.top < m.work.bottom)
      .or_else(|| monitors.iter().find(|m| m.primary)).cloned();
    let dpi = monitor.as_ref().map_or(96, |m| m.dpi);
    state.scale = monitor.as_ref().map_or(1.0, |m| fit_scale(super::super::scale::of_dpi(dpi), m.work.right - m.work.left, m.work.bottom - m.work.top,state.draft.recent.len()));
    state.font = make_font(state.scale,13.5,450); state.small_font=make_font(state.scale,11.0,400); state.title_font=make_font(state.scale,18.0,600);
    let mut size = RECT { right: (WIDTH as f32 * state.scale).round() as i32, bottom: (client_height(state.draft.recent.len()) as f32 * state.scale).round() as i32, ..Default::default() };
    let _ = AdjustWindowRectExForDpi(&mut size, PICKER_STYLE, false, WS_EX_TOOLWINDOW, dpi);
    let (width, height) = (size.right - size.left, size.bottom - size.top);
    let (x, y) = monitor.map(|m| (rect.left.min(m.work.right - width).max(m.work.left), rect.top.min(m.work.bottom - height).max(m.work.top)))
      .unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT));
    let hwnd = match CreateWindowExW(WS_EX_TOOLWINDOW, CLASS_NAME, &HSTRING::from(&state.labels[0]),
      PICKER_STYLE, x, y, width, height, None, None, instance,
      Some((&mut *state as *mut Picker).cast())) {
      Ok(hwnd) => hwnd, Err(_) => { super::super::send(Msg::Widgets(Ev::PickerClosed(id, token))); return; }
    };
    super::super::send(Msg::Widgets(Ev::PickerOpened(id, token, hwnd.0 as isize)));
    rounded_region(hwnd,width,height,(18.0*state.scale).round() as i32);
    // Do not take focus after the user has switched away during creation.
    let foreground = GetForegroundWindow();
    let mut foreground_pid = 0; let mut owner_pid = 0;
    GetWindowThreadProcessId(foreground, Some(&mut foreground_pid)); GetWindowThreadProcessId(owner_hwnd, Some(&mut owner_pid));
    let take_focus = foreground == owner_hwnd || (owner_pid != 0 && foreground_pid == owner_pid);
    let _ = ShowWindow(hwnd, if take_focus { SW_SHOW } else { SW_SHOWNOACTIVATE });
    if take_focus { let _ = SetForegroundWindow(hwnd); let _ = SetFocus(state.edits[0]); }
    SetTimer(hwnd, 1, 50, None);
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
      if msg.message == WM_LBUTTONUP && (msg.hwnd == state.results || msg.hwnd == state.recent) {
        let item = SendMessageW(msg.hwnd, LB_ITEMFROMPOINT, WPARAM(0), msg.lParam).0;
        DispatchMessageW(&msg);
        if (item >> 16) & 0xFFFF == 0 {
          let control = if msg.hwnd == state.results { RESULTS } else { RECENT };
          SendMessageW(hwnd, WM_COMMAND, WPARAM(control | ((LBN_DBLCLK as usize) << 16)), LPARAM(0));
        }
      } else if msg.message == WM_KEYDOWN && msg.wParam.0 == 0x0D && (GetFocus() == state.results || GetFocus() == state.recent) {
        let control = if GetFocus() == state.results { RESULTS } else { RECENT };
        SendMessageW(hwnd, WM_COMMAND, WPARAM(control | ((LBN_DBLCLK as usize) << 16)), LPARAM(0));
      } else if msg.message == WM_KEYDOWN && msg.wParam.0 == 0x1B {
        let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
      } else if msg.message == WM_KEYDOWN && msg.wParam.0 == 0x28 && state.edits.contains(&GetFocus()) && IsWindowVisible(state.results).as_bool() {
        let _ = SetFocus(state.results); SendMessageW(state.results, LB_SETCURSEL, WPARAM(0), LPARAM(0));
      } else if !IsDialogMessageW(hwnd, &msg).as_bool() {
        let _ = TranslateMessage(&msg); DispatchMessageW(&msg);
      }
    }
    super::super::send(Msg::Widgets(Ev::PickerClosed(id, token)));
  });
  if started.is_err() { super::super::send(Msg::Widgets(Ev::PickerClosed(id, token))); }
}

impl Drop for Picker {
  fn drop(&mut self) { unsafe { for font in [self.font,self.small_font,self.title_font] { let _=DeleteObject(font); } let _ = DeleteObject(self.background); let _ = DeleteObject(self.field_background); let _=DeleteObject(self.popup_background); } }
}

unsafe fn make_font(scale: f32, size:f32, weight:i32) -> HFONT {
  // The bundled variable WOFF fonts belong to DirectWrite; GDI controls
  // use Windows' Unicode UI fallback at the same LL body size.
  CreateFontW(-(size * scale).round() as i32, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET.0 as u32,
    OUT_DEFAULT_PRECIS.0 as u32, CLIP_DEFAULT_PRECIS.0 as u32, CLEARTYPE_QUALITY.0 as u32, 0, w!("Segoe UI"))
}

unsafe fn rounded_region(hwnd:HWND,width:i32,height:i32,radius:i32) {
  let region=CreateRoundRectRgn(0,0,width+1,height+1,radius*2,radius*2);
  if SetWindowRgn(hwnd,region,false)==0 { let _=DeleteObject(region); }
}

unsafe fn control(hwnd: HWND, class: PCWSTR, label: &str, style: WINDOW_STYLE, id: usize, r: (i32, i32, i32, i32), scale: f32, font: HFONT) -> HWND {
  let child = CreateWindowExW(WINDOW_EX_STYLE(0), class, &HSTRING::from(label), WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | style,
    (r.0 as f32 * scale).round() as i32, (r.1 as f32 * scale).round() as i32,
    (r.2 as f32 * scale).round() as i32, (r.3 as f32 * scale).round() as i32,
    hwnd, HMENU(id as *mut _), GetModuleHandleW(None).unwrap_or_default(), None).unwrap_or_default();
  SendMessageW(child, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
  child
}

unsafe fn text(hwnd: HWND) -> String {
  let len = GetWindowTextLengthW(hwnd).max(0) as usize;
  let mut buffer = vec![0u16; len + 1];
  let read = GetWindowTextW(hwnd, &mut buffer).max(0) as usize;
  String::from_utf16_lossy(&buffer[..read])
}

impl Picker {
  unsafe fn control(&mut self, hwnd: HWND, class: PCWSTR, label: &str, style: WINDOW_STYLE, id: usize, rect: (i32, i32, i32, i32)) -> HWND {
    let control = control(hwnd, class, label, style, id, rect, self.scale, self.font);
    self.controls.push((control, rect)); control
  }
  unsafe fn controls(&mut self, hwnd: HWND) {
    let labels = self.labels.clone();
    let title=self.control(hwnd,w!("STATIC"),&labels[0],WINDOW_STYLE(0),200,(22,20,326,26));
    SendMessageW(title,WM_SETFONT,WPARAM(self.title_font.0 as usize),LPARAM(1));
    self.control(hwnd,w!("STATIC"),labels.get(15).map(String::as_str).unwrap_or(""),WINDOW_STYLE(0),201,(22,48,350,18));
    self.close=self.control(hwnd,w!("BUTTON"),labels.get(16).map(String::as_str).unwrap_or("Kapat"),WS_TABSTOP|WINDOW_STYLE(BS_OWNERDRAW as u32),3,(370,18,28,28));
    for i in 0..3 {
      let y=FIELD_Y+i as i32*60;
      self.field_labels[i]=self.control(hwnd, w!("STATIC"), &labels[1 + i], WINDOW_STYLE(0), 202+i, (36,y+7,348,15));
      self.edits[i] = self.control(hwnd, w!("EDIT"), "", WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32), 100 + i, (36,y+26,348,21));
    }
    let list_style=WS_TABSTOP|WS_VSCROLL|WINDOW_STYLE((LBS_NOTIFY|LBS_OWNERDRAWFIXED|LBS_HASSTRINGS|LBS_NOINTEGRALHEIGHT) as u32);
    self.results = self.control(hwnd, w!("LISTBOX"), "", list_style, RESULTS, (22,140,376,120));
    self.status = self.control(hwnd, w!("STATIC"), "", WINDOW_STYLE(0), 205, (22,270,276,28));
    self.retry = self.control(hwnd, w!("BUTTON"), &labels[5], WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32), RETRY, (300,267,98,28));
    let recent_label=self.control(hwnd, w!("STATIC"), &labels[6], WINDOW_STYLE(0), 206, (22,310,376,18));
    self.recent = self.control(hwnd, w!("LISTBOX"), "", list_style, RECENT, (22,336,376,self.draft.recent.len().min(2).max(1) as i32*ROW_HEIGHT));
    for location in &self.draft.recent {
      let label = HSTRING::from(format!("{} · {}", location.display(), location.country));
      SendMessageW(self.recent, LB_ADDSTRING, WPARAM(0), LPARAM(label.as_ptr() as isize));
    }
    let footer=footer_y(self.draft.recent.len());
    self.control(hwnd, w!("BUTTON"), &labels[7], WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32), 2, (200,footer,94,36));
    self.save = self.control(hwnd, w!("BUTTON"), &labels[8], WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32), 1, (304,footer,94,36));
    self.control(hwnd, w!("STATIC"), "© OpenStreetMap contributors · Photon / Open-Meteo", WINDOW_STYLE(0), 207, (22,footer+54,376,18));
    for (control,_) in &self.controls {
      let id=GetDlgCtrlID(*control);
      if (201..=207).contains(&id) { SendMessageW(*control,WM_SETFONT,WPARAM(self.small_font.0 as usize),LPARAM(1)); }
    }
    for list in [self.results,self.recent] { SendMessageW(list,LB_SETITEMHEIGHT,WPARAM(0),LPARAM((ROW_HEIGHT as f32*self.scale).round() as isize)); }
    if self.draft.recent.is_empty() { let _=ShowWindow(recent_label,SW_HIDE); let _=ShowWindow(self.recent,SW_HIDE); }
    self.sync();
  }

  /// Update only dependent controls after user edits, never the active Edit.
  unsafe fn sync(&mut self) {
    self.updating = true;
    for i in 0..3 {
      if text(self.edits[i]) != self.draft.text[i] { let _ = SetWindowTextW(self.edits[i], &HSTRING::from(&self.draft.text[i])); }
    }
    let _ = EnableWindow(self.edits[0], !self.save_busy);
    let _ = EnableWindow(self.edits[1], self.draft.country.is_some() && !self.save_busy);
    let _ = EnableWindow(self.edits[2], self.draft.selected.is_some() && !self.save_busy);
    let _ = EnableWindow(self.results, !self.save_busy); let _ = EnableWindow(self.recent, !self.save_busy);
    let _ = EnableWindow(self.save, self.draft.can_save() && !self.save_busy); let _=EnableWindow(self.close,!self.save_busy);
    self.updating = false;
    self.show_results();
  }

  unsafe fn show_results(&mut self) {
    SendMessageW(self.results, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
    for choice in &self.draft.results {
      let label = match choice {
        Choice::Country(c) => format!("{} ({}) · {}", c.name, c.code, c.english_name),
        Choice::Place(p) => format!("{} · {}", p.label, p.country),
      };
      let label = HSTRING::from(label);
      SendMessageW(self.results, LB_ADDSTRING, WPARAM(0), LPARAM(label.as_ptr() as isize));
    }
    let show=!self.draft.results.is_empty() && !self.draft.text[self.draft.field as usize].trim().is_empty();
    let height=if show {self.draft.results.len().min(3) as i32*ROW_HEIGHT} else {0};
    let top=if self.draft.field==Field::District { FIELD_Y+120-4-height } else { FIELD_Y+self.draft.field as i32*60+56 };
    let rect=(22,top,376,height.max(ROW_HEIGHT));
    if let Some((_,old))=self.controls.iter_mut().find(|(control,_)| *control==self.results) { *old=rect; }
    let _=SetWindowPos(self.results,HWND_TOP,(rect.0 as f32*self.scale).round() as i32,(rect.1 as f32*self.scale).round() as i32,
      (rect.2 as f32*self.scale).round() as i32,(rect.3 as f32*self.scale).round() as i32,
      SWP_NOACTIVATE|if show { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW });
    rounded_region(self.results,(rect.2 as f32*self.scale).round() as i32,(rect.3 as f32*self.scale).round() as i32,(10.0*self.scale).round() as i32);
    self.show_status();
  }

  unsafe fn show_status(&self) {
    let status = if let Some(error) = &self.save_error { format!("{}: {error}", self.labels[14]) }
      else if let Some(error) = &self.draft.error { format!("{}: {error}", self.labels[9]) }
      else if self.draft.inflight.is_some() { self.labels[10].clone() }
      else if self.draft.can_save() { self.labels[11].clone() }
      else if self.draft.text[self.draft.field as usize].trim().is_empty() { String::new() }
      else if self.draft.results.is_empty() { self.labels[12].clone() }
      else { self.labels[13].clone() };
    let _ = SetWindowTextW(self.status, &HSTRING::from(status));
    let _ = ShowWindow(self.retry,if self.draft.error.is_some() { SW_SHOWNA } else { SW_HIDE });
    let _ = EnableWindow(self.retry, self.draft.error.is_some() && !self.save_busy);
    let _=InvalidateRect(GetParent(self.status).unwrap_or_default(),None,false);
  }

  unsafe fn paint(&self,hwnd:HWND,dc:HDC) {
    let mut bounds=RECT::default(); let _=GetClientRect(hwnd,&mut bounds);
    FillRect(dc,&bounds,self.background);
    draw_round(dc,bounds,color(self.theme.layer0),color(self.theme.outline_variant),(18.0*self.scale).round() as i32);
    let focus=GetFocus();
    for i in 0..3 {
      let active=focus==self.edits[i] || focus==self.results && self.draft.field as usize==i;
      draw_round(dc,self.rect((22,FIELD_Y+i as i32*60,376,52)),color(self.theme.surface_container_high),
        color(if active {self.theme.primary} else {self.theme.surface_container_high}),(12.0*self.scale).round() as i32);
    }
    if GetWindowLongW(self.results,GWL_STYLE) as u32&WS_VISIBLE.0!=0 {
      if let Some((_,r))=self.controls.iter().find(|(control,_)|*control==self.results) {
        // Two small depth steps plus a hairline separate the floating results
        // from the input surfaces. The child HWND supplies the opaque fill.
        let shade=|factor:f32|color(super::super::gfx::Rgba((self.theme.layer1_hover.0 as f32*factor) as u8,(self.theme.layer1_hover.1 as f32*factor) as u8,(self.theme.layer1_hover.2 as f32*factor) as u8,1.0));
        draw_round(dc,self.rect((r.0-4,r.1+2,r.2+8,r.3+6)),shade(0.65),shade(0.65),(13.0*self.scale).round() as i32);
        draw_round(dc,self.rect((r.0-2,r.1+1,r.2+4,r.3+3)),shade(0.45),shade(0.45),(11.0*self.scale).round() as i32);
        draw_round(dc,self.rect((r.0-1,r.1-1,r.2+2,r.3+2)),color(self.theme.layer1_hover),color(self.theme.outline_variant),(10.0*self.scale).round() as i32);
      }
    }
    let y=(footer_y(self.draft.recent.len()) as f32-12.0)*self.scale;
    let pen=CreatePen(PS_SOLID,1,color(self.theme.outline_variant)); let old=SelectObject(dc,pen);
    let _=MoveToEx(dc,(22.0*self.scale).round() as i32,y.round() as i32,None);
    let _=LineTo(dc,(398.0*self.scale).round() as i32,y.round() as i32);
    let _=SelectObject(dc,old); let _=DeleteObject(pen);
  }

  fn rect(&self,r:(i32,i32,i32,i32))->RECT {
    RECT {left:(r.0 as f32*self.scale).round() as i32,top:(r.1 as f32*self.scale).round() as i32,
      right:((r.0+r.2) as f32*self.scale).round() as i32,bottom:((r.1+r.3) as f32*self.scale).round() as i32}
  }

  unsafe fn draw_list(&self,item:&DRAWITEMSTRUCT) {
    let selected=item.itemState.0&ODS_SELECTED.0!=0;
    let background=if selected {self.theme.primary_container} else if item.CtlID as usize==RESULTS {self.theme.layer1_hover} else {self.theme.surface_container_high};
    let fg=if selected {self.theme.on_primary_container} else {self.theme.on_layer0};
    let brush=CreateSolidBrush(color(background)); FillRect(item.hDC,&item.rcItem,brush); let _=DeleteObject(brush);
    let (main,detail)=if item.CtlID as usize==RESULTS {
      match self.draft.results.get(item.itemID as usize) {
        Some(Choice::Country(c)) => (c.name.clone(),if c.english_name.is_empty() || c.english_name==c.name {c.code.clone()} else {format!("{} · {}",c.code,c.english_name)}),
        Some(Choice::Place(p)) => (p.name.clone(),p.label.clone()),
        None => return,
      }
    } else { match self.draft.recent.get(item.itemID as usize) {Some(p)=>(p.display(),p.country.clone()),None=>return} };
    let inset=(12.0*self.scale).round() as i32; let top=(3.0*self.scale).round() as i32;
    let primary=RECT {left:item.rcItem.left+inset,right:item.rcItem.right-inset,top:item.rcItem.top+top,bottom:item.rcItem.top+(23.0*self.scale).round() as i32};
    let secondary=RECT {top:primary.bottom-2,bottom:item.rcItem.bottom-2,..primary};
    draw_label(item.hDC,self.font,&main,primary,color(fg));
    draw_label(item.hDC,self.small_font,&detail,secondary,color(self.theme.on_surface_variant));
    if item.itemState.0&ODS_FOCUS.0!=0 { let _=DrawFocusRect(item.hDC,&item.rcItem); }
  }
}

unsafe fn draw_round(dc:HDC,rect:RECT,bg:COLORREF,edge:COLORREF,radius:i32) {
  let brush=CreateSolidBrush(bg); let pen=CreatePen(PS_SOLID,1,edge);
  let old_brush=SelectObject(dc,brush); let old_pen=SelectObject(dc,pen);
  let _=RoundRect(dc,rect.left,rect.top,rect.right,rect.bottom,radius*2,radius*2);
  let _=SelectObject(dc,old_brush); let _=SelectObject(dc,old_pen); let _=DeleteObject(brush); let _=DeleteObject(pen);
}
unsafe fn draw_label(dc:HDC,font:HFONT,label:&str,mut rect:RECT,fg:COLORREF) {
  let old=SelectObject(dc,font); SetBkMode(dc,TRANSPARENT); SetTextColor(dc,fg);
  let mut chars:Vec<u16>=label.encode_utf16().collect();
  DrawTextW(dc,&mut chars,&mut rect,DT_SINGLELINE|DT_VCENTER|DT_END_ELLIPSIS|DT_NOPREFIX);
  let _=SelectObject(dc,old);
}

unsafe extern "system" fn proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
  // a panic must not cross into Windows (it would abort the shell)
  std::panic::catch_unwind(|| proc_inner(hwnd, message, wp, lp)).unwrap_or_else(|_| {
    tracing::error!("Picker: a window message failed; ignored");
    LRESULT(0)
  })
}

unsafe fn proc_inner(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
  if message == WM_NCCREATE {
    let create = &*(lp.0 as *const CREATESTRUCTW);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
  }
  if message == WM_CLOSE { let _ = DestroyWindow(hwnd); return LRESULT(0); }
  if message == WM_DESTROY { PostQuitMessage(0); return LRESULT(0); }
  let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Picker;
  if !ptr.is_null() && message==WM_NCHITTEST {
    let mut point=windows::Win32::Foundation::POINT { x:lp.0 as i16 as i32,y:(lp.0>>16) as i16 as i32 };
    let _=ScreenToClient(hwnd,&mut point);
    let scale=(*ptr).scale;
    return LRESULT(if point.y>=0 && point.y<(70.0*scale) as i32 && point.x<(360.0*scale) as i32 {HTCAPTION} else {HTCLIENT} as isize);
  }
  if ptr.is_null() || !matches!(message, WM_CREATE | WM_COMMAND | WM_TIMER | WM_PAINT | WM_PRINTCLIENT | WM_ERASEBKGND | WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN | WM_DRAWITEM | WM_DPICHANGED | WM_DISPLAYCHANGE) { return DefWindowProcW(hwnd, message, wp, lp); }
  // SetWindowText sends synchronous EN_CHANGE; do not reborrow the state.
  if (*ptr).updating || (*ptr).handling { return DefWindowProcW(hwnd, message, wp, lp); }
  (*ptr).handling = true;
  let result = (|| {
  let state = &mut *ptr;
  match message {
    WM_CREATE => {
      state.controls(hwnd);
      if state.controls.iter().any(|(control, _)| control.0.is_null()) { return LRESULT(-1); }
    }
    WM_ERASEBKGND => {
      state.paint(hwnd,HDC(wp.0 as *mut _)); return LRESULT(1);
    }
    WM_PAINT => {
      let mut paint=PAINTSTRUCT::default(); let dc=BeginPaint(hwnd,&mut paint);
      state.paint(hwnd,dc); let _=EndPaint(hwnd,&paint); return LRESULT(0);
    }
    WM_PRINTCLIENT => { state.paint(hwnd,HDC(wp.0 as *mut _)); return LRESULT(0); }
    WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => {
      let dc = HDC(wp.0 as *mut _);
      let control=HWND(lp.0 as *mut _);
      let popup=control==state.results;
      let field = matches!(message, WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX) || state.field_labels.contains(&control) || state.edits.contains(&control);
      let muted=message==WM_CTLCOLORSTATIC && control!=GetDlgItem(hwnd,200).unwrap_or_default();
      SetTextColor(dc, color(if control==state.status && (state.save_error.is_some() || state.draft.error.is_some()) {state.theme.error} else if muted {state.theme.on_surface_variant} else {state.theme.on_layer0}));
      SetBkColor(dc, color(if popup {state.theme.layer1_hover} else if field { state.theme.surface_container_high } else { state.theme.layer0 }));
      return LRESULT(if popup {state.popup_background.0} else if field { state.field_background.0 } else { state.background.0 } as isize);
    }
    WM_DRAWITEM => {
      let item = &*(lp.0 as *const DRAWITEMSTRUCT);
      if [RESULTS,RECENT].contains(&(item.CtlID as usize)) { state.draw_list(item); return LRESULT(1); }
      let primary = item.CtlID == 1;
      let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
      let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
      let bg = if primary && !disabled { state.theme.primary } else if item.CtlID==3 || item.CtlID==2 || item.CtlID as usize==RETRY {state.theme.layer0} else { state.theme.surface_container_high };
      let fg = if disabled { state.theme.on_surface_variant } else if primary { state.theme.on_primary } else { state.theme.on_layer0 };
      FillRect(item.hDC, &item.rcItem, state.background);
      let brush = CreateSolidBrush(color(bg)); let old_brush = SelectObject(item.hDC, brush);
      let old_pen = SelectObject(item.hDC, GetStockObject(DC_PEN));
      SetDCPenColor(item.hDC, color(if item.itemState.0 & ODS_FOCUS.0 != 0 { state.theme.primary } else { bg }));
      let radius = (14.0 * state.scale).round() as i32;
      let _ = RoundRect(item.hDC, item.rcItem.left, item.rcItem.top, item.rcItem.right, item.rcItem.bottom, radius, radius);
      let _ = SelectObject(item.hDC, old_brush); let _ = SelectObject(item.hDC, old_pen); let _ = DeleteObject(brush);
      let old_font = SelectObject(item.hDC, if item.CtlID==3 {state.title_font} else {state.font});
      SetBkMode(item.hDC, TRANSPARENT); SetTextColor(item.hDC, color(fg));
      let mut text: Vec<u16> = if item.CtlID==3 {"×".into()} else {text(item.hwndItem)}.encode_utf16().collect(); let mut rect = item.rcItem;
      if pressed { rect.top += 1; }
      DrawTextW(item.hDC, &mut text, &mut rect, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
      let _ = SelectObject(item.hDC, old_font);
      return LRESULT(1);
    }
    WM_DPICHANGED | WM_DISPLAYCHANGE => {
      let mut proposed = RECT::default();
      if message == WM_DPICHANGED { proposed = *(lp.0 as *const RECT); } else { let _ = GetWindowRect(hwnd, &mut proposed); }
      let monitors = super::monitors();
      let monitor = monitors.iter().find(|m| proposed.left >= m.work.left && proposed.left < m.work.right && proposed.top >= m.work.top && proposed.top < m.work.bottom)
        .or_else(|| monitors.iter().find(|m| m.primary));
      if let Some(monitor) = monitor {
        let dpi = if message == WM_DPICHANGED { (wp.0 & 0xFFFF) as u32 } else { monitor.dpi };
        state.scale = fit_scale(super::super::scale::of_dpi(dpi), monitor.work.right - monitor.work.left, monitor.work.bottom - monitor.work.top,state.draft.recent.len());
        let old_fonts=[state.font,state.small_font,state.title_font];
        state.font=make_font(state.scale,13.5,450); state.small_font=make_font(state.scale,11.0,400); state.title_font=make_font(state.scale,18.0,600);
        for (control, r) in &state.controls {
          let _ = SetWindowPos(*control, None, (r.0 as f32 * state.scale).round() as i32, (r.1 as f32 * state.scale).round() as i32,
            (r.2 as f32 * state.scale).round() as i32, (r.3 as f32 * state.scale).round() as i32, SWP_NOZORDER | SWP_NOACTIVATE);
          let id=GetDlgCtrlID(*control);
          let font=if id==200 {state.title_font} else if (201..=207).contains(&id) {state.small_font} else {state.font};
          SendMessageW(*control, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
        for font in old_fonts { let _=DeleteObject(font); }
        for list in [state.results,state.recent] { SendMessageW(list,LB_SETITEMHEIGHT,WPARAM(0),LPARAM((ROW_HEIGHT as f32*state.scale).round() as isize)); }
        let mut size = RECT { right: (WIDTH as f32 * state.scale).round() as i32, bottom: (client_height(state.draft.recent.len()) as f32 * state.scale).round() as i32, ..Default::default() };
        let _ = AdjustWindowRectExForDpi(&mut size, PICKER_STYLE, false, WS_EX_TOOLWINDOW, dpi);
        let width = size.right - size.left; let height = size.bottom - size.top;
        let _ = SetWindowPos(hwnd, None, proposed.left.min(monitor.work.right - width).max(monitor.work.left), proposed.top.min(monitor.work.bottom - height).max(monitor.work.top), width, height, SWP_NOZORDER | SWP_NOACTIVATE);
        rounded_region(hwnd,width,height,(18.0*state.scale).round() as i32); state.show_results();
      }
    }
    WM_TIMER => {
      let percent = super::super::scale::percent();
      if percent != state.ui_percent { state.ui_percent = percent; let _ = PostMessageW(hwnd, WM_DISPLAYCHANGE, WPARAM(0), LPARAM(0)); }
      if let Ok(answer) = state.saved_rx.try_recv() {
        state.save_busy = false;
        match answer { Ok(()) => { let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)); }, Err(error) => state.save_error = Some(error) }
        state.sync();
      }
      while let Ok((generation, answer)) = state.rx.try_recv() {
        state.draft.answer(generation, answer); state.show_results();
      }
      if let Some((generation, path)) = state.draft.request(&state.language, Instant::now()) {
        let tx = state.tx.clone();
        std::thread::spawn(move || { let _ = tx.send((generation, location::fetch(&path))); });
        state.show_status();
      }
    }
    WM_COMMAND => {
      let id = wp.0 & 0xFFFF; let notification = (wp.0 >> 16) as u32;
      if (100..103).contains(&id) {
        let field = [Field::Country, Field::City, Field::District][id - 100];
        if notification == EN_CHANGE {
          state.draft.change(field, text(state.edits[id - 100])); state.sync();
        } else if notification == EN_SETFOCUS {
          state.draft.focus(field); state.show_results();
        } else if notification==EN_KILLFOCUS { let _=InvalidateRect(hwnd,None,false); }
      } else if id == RESULTS && notification == LBN_DBLCLK {
        let index = SendMessageW(state.results, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
        if index >= 0 {
          if let Some(choice) = state.draft.results.get(index as usize).cloned() {
            if state.draft.choose(choice) { state.sync(); let _ = SetFocus(state.edits[state.draft.field as usize]); }
          }
        }
      } else if id == RECENT && notification == LBN_DBLCLK {
        let index = SendMessageW(state.recent, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
        if index >= 0 { if let Some(location) = state.draft.recent.get(index as usize).cloned() { state.draft.restore(location); state.sync(); } }
      } else if id == RETRY { state.draft.retry(); state.show_results(); }
      else if id == 2 || id==3 { let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)); }
      else if id == 1 && state.draft.can_save() && !state.save_busy {
        state.save_error = None; state.save_busy = true; state.sync();
        if let Some(location) = state.draft.selected.clone() { super::super::send(Msg::Widgets(Ev::LocationSave(state.id, state.token, location, state.saved_tx.clone()))); }
      }
    }
    _ => {},
  }
  LRESULT(0)
  })();
  (*ptr).handling = false;
  result
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn picker_fits_dpi_and_ui_scale_on_small_monitors() {
    for (scale, width, height) in [(1.25, 1366, 728), (1.875, 1280, 680), (3.0, 1920, 1040)] {
      for recents in [0,1,8] {
        let fitted = fit_scale(scale, width, height,recents);
        assert!(WIDTH as f32 * fitted + 20.0 <= width as f32 + 0.1);
        assert!(client_height(recents) as f32 * fitted + 20.0 <= height as f32 + 0.1);
      }
    }
  }

  /// Exercises real EDIT/LISTBOX controls in an invisible test-only window.
  /// No foreground activation, keyboard injection or installed core is used.
  #[test]
  fn hidden_picker_controls_and_dropdown_fit() { unsafe {
    let instance=GetModuleHandleW(None).unwrap();
    RegisterClassW(&WNDCLASSW {lpfnWndProc:Some(proc),hInstance:instance.into(),lpszClassName:CLASS_NAME,..Default::default()});
    for (scale,recents) in [(1.0,0),(1.5,1),(1.0,8)] {
      let mut spec=Spec::default();
      let place=super::super::layout::Location {country_code:"TR".into(),country:"Türkiye".into(),city:"İstanbul".into(),district:"Kadıköy".into(),latitude:40.99,longitude:29.03,city_latitude:41.01,city_longitude:28.98};
      spec.recent_locations=vec![place.clone();recents];
      let (tx,rx)=mpsc::channel(); let (saved_tx,saved_rx)=mpsc::channel();
      let theme=super::super::super::view::DARK;
      let labels=["Konumu değiştir","Ülke","Şehir","İlçe (isteğe bağlı)","Sonuçlar","Tekrar dene","Son konumlar","İptal","Kaydet","Konumlar yüklenemedi","Aranıyor…","Konum seçildi · Kaydet ile uygulayın","Sonuç yok","Bir sonuç seçin","Konum kaydedilemedi","Ülke, şehir ve isteğe bağlı ilçe","Kapat"].iter().map(|s|s.to_string()).collect();
      let mut state=Box::new(Picker {id:1,token:1,language:"tr".into(),draft:Draft::new(&spec),edits:[HWND::default();3],results:HWND::default(),recent:HWND::default(),status:HWND::default(),save:HWND::default(),retry:HWND::default(),field_labels:[HWND::default();3],close:HWND::default(),updating:false,rx,tx,labels,theme,scale,
        font:make_font(scale,13.5,450),small_font:make_font(scale,11.0,400),title_font:make_font(scale,18.0,600),background:CreateSolidBrush(color(theme.layer0)),field_background:CreateSolidBrush(color(theme.surface_container_high)),popup_background:CreateSolidBrush(color(theme.layer1_hover)),controls:Vec::new(),saved_rx,saved_tx,save_busy:false,save_error:None,handling:false,ui_percent:100});
      let width=(WIDTH as f32*scale).round() as i32; let height=(client_height(recents) as f32*scale).round() as i32;
      let foreground=GetForegroundWindow();
      let hwnd=CreateWindowExW(WS_EX_TOOLWINDOW,CLASS_NAME,w!("Picker test"),PICKER_STYLE,0,0,width,height,None,None,instance,Some((&mut *state as *mut Picker).cast())).unwrap();
      assert!(!IsWindowVisible(hwnd).as_bool());
      assert_eq!(GetForegroundWindow(),foreground);
      assert_eq!(SendMessageW(state.results,LB_GETITEMHEIGHT,WPARAM(0),LPARAM(0)).0,(ROW_HEIGHT as f32*scale).round() as isize);
      assert_eq!(GetWindowLongW(state.results,GWL_STYLE) as u32&WS_VISIBLE.0,0,"no empty results box");
      for (control,_) in &state.controls {
        if GetWindowLongW(*control,GWL_STYLE) as u32&WS_VISIBLE.0==0 {continue}
        let mut r=RECT::default();GetWindowRect(*control,&mut r).unwrap();
        assert!(r.left>=0&&r.top>=0&&r.right<=width&&r.bottom<=height,"control {} outside picker: {:?}",GetDlgCtrlID(*control),r);
      }
      state.draft.change(Field::Country,"T".into());
      state.draft.results=[("TR","Türkiye","Turkey"),("TM","Türkmenistan","Turkmenistan"),("TJ","Tacikistan","Tajikistan"),("TZ","Tanzanya","Tanzania"),("TH","Tayland","Thailand"),("TW","Tayvan","Taiwan"),("TG","Togo","Togo"),("TN","Tunus","Tunisia")].into_iter()
        .map(|(code,name,english_name)|Choice::Country(location::Country {code:code.into(),name:name.into(),english_name:english_name.into()})).collect();state.sync();
      assert_eq!(text(state.edits[0]),"T");
      assert_ne!(GetWindowLongW(state.results,GWL_STYLE) as u32&WS_VISIBLE.0,0);
      let mut r=RECT::default();GetWindowRect(state.results,&mut r).unwrap();
      assert_eq!(r.top,(140.0*scale).round() as i32);assert_eq!(r.bottom-r.top,(120.0*scale).round() as i32);
      assert_eq!(GetWindow(hwnd,GW_CHILD).unwrap(),state.results,"suggestions are above all other controls");
      assert_eq!(SendMessageW(state.results,LB_GETCOUNT,WPARAM(0),LPARAM(0)).0,8,"all results remain available via scrolling");
      if let Ok(out)=std::env::var("LL_PICKER_SNAPSHOTS") { snapshot(hwnd,width,height,&std::path::Path::new(&out).join(format!("picker-{scale}-{recents}.bmp"))); }
      for field in [Field::City,Field::District] {
        state.draft.field=field;state.draft.text[field as usize]="K".into();state.sync();
        GetWindowRect(state.results,&mut r).unwrap();
        assert!(r.bottom<=height,"suggestion overlay remains inside the picker");
      }
      state.draft.restore(place);state.sync();
      let mut restored=RECT::default();GetWindowRect(hwnd,&mut restored).unwrap();assert_eq!(restored.bottom-restored.top,height,"closing results restores the compact height");
      if let Ok(out)=std::env::var("LL_PICKER_SNAPSHOTS") {snapshot(hwnd,width,height,&std::path::Path::new(&out).join(format!("picker-saved-{scale}-{recents}.bmp")));}
      assert_eq!(GetForegroundWindow(),foreground);
      DestroyWindow(hwnd).unwrap();
    }
  } }

  unsafe fn snapshot(hwnd:HWND,width:i32,height:i32,path:&std::path::Path) {
    let dc=CreateCompatibleDC(None);let mut bits=std::ptr::null_mut();
    let mut info=BITMAPINFO::default();info.bmiHeader=BITMAPINFOHEADER {biSize:std::mem::size_of::<BITMAPINFOHEADER>() as u32,biWidth:width,biHeight:-height,biPlanes:1,biBitCount:32,biCompression:BI_RGB.0,..Default::default()};
    let bitmap=CreateDIBSection(dc,&info,DIB_RGB_COLORS,&mut bits,None,0).unwrap();let old=SelectObject(dc,bitmap);
    SendMessageW(hwnd,WM_PRINTCLIENT,WPARAM(dc.0 as usize),LPARAM(PRF_CLIENT as isize));
    // Paint from the back of the HWND stack so the autocomplete overlays
    // the following fields, just as it does in a visible native window.
    let mut children=Vec::new();let mut child=GetWindow(hwnd,GW_CHILD).unwrap_or_default();
    while !child.0.is_null() {children.push(child);child=GetWindow(child,GW_HWNDNEXT).unwrap_or_default();}
    for child in children.into_iter().rev() {
      if GetWindowLongW(child,GWL_STYLE) as u32&WS_VISIBLE.0==0 {continue}
      let mut r=RECT::default();GetWindowRect(child,&mut r).unwrap();
      let saved=SaveDC(dc);let _=SetViewportOrgEx(dc,r.left,r.top,None);IntersectClipRect(dc,0,0,r.right-r.left,r.bottom-r.top);
      if GetDlgCtrlID(child)==RESULTS as i32 {
        let clip=CreateRectRgn(0,0,0,0);GetWindowRgn(child,clip);OffsetRgn(clip,r.left,r.top);SelectClipRgn(dc,clip);let _=DeleteObject(clip);
        let bounds=RECT {right:r.right-r.left,bottom:r.bottom-r.top,..Default::default()};
        let brush=CreateSolidBrush(color(super::super::super::view::DARK.layer1_hover));FillRect(dc,&bounds,brush);let _=DeleteObject(brush);
      }
      SendMessageW(child,WM_PRINT,WPARAM(dc.0 as usize),LPARAM((PRF_CLIENT|PRF_NONCLIENT|PRF_ERASEBKGND) as isize));let _=RestoreDC(dc,saved);
    }
    let _=GdiFlush();let pixels=std::slice::from_raw_parts(bits as *const u8,(width*height*4) as usize);
    let mut bmp=Vec::new();bmp.extend(b"BM");bmp.extend((54+pixels.len() as u32).to_le_bytes());bmp.extend([0u8;4]);bmp.extend(54u32.to_le_bytes());
    bmp.extend(40u32.to_le_bytes());bmp.extend(width.to_le_bytes());bmp.extend((-height).to_le_bytes());bmp.extend(1u16.to_le_bytes());bmp.extend(32u16.to_le_bytes());bmp.extend([0u8;24]);bmp.extend(pixels);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();std::fs::write(path,bmp).unwrap();
    SelectObject(dc,old);let _=DeleteObject(bitmap);let _=DeleteDC(dc);
  }
}
