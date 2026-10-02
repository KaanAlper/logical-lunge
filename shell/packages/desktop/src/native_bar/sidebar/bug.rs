//! The issue report (the web panel's bug-report.jsx): a dialog over the
//! dimmed panel with the kind, when it happened, a description, the logs to
//! attach (the core's black box, core / shell / window manager logs:
//! `--bug-report-file`), the device (`--bug-report-device`), sent to the
//! project's Supabase table (insert only) with upload progress.

use std::time::Duration;

use serde_json::{json, Value};

use super::{
  super::{
    core_api,
    dialog::{Kind, Spec},
    gfx::{Rect, Rgba},
    model::Model,
    send, Msg, Ui,
  },
  kit::{st, stw, Cx},
  text::{TextField, Typed},
  Ev, FieldId, Hit, Sidebar,
};

const SUPABASE_URL: &str = "https://bygirolbhyziitvnaxln.supabase.co";
// A publishable key is safe in a shipped client: the table allows INSERT
// through RLS and denies public SELECT, UPDATE and DELETE.
const SUPABASE_KEY: &str = "sb_publishable_Tn0PWa_PME7JLTu3xHtd2w_b4ywwiAO";

/// (kind, file name, icon)
const FILES: [(&str, &str, &str); 4] = [
  ("blackbox", "blackbox_record.txt", "monitor_heart"),
  ("core", "core.log", "description"),
  ("shell", "shell.log", "description"),
  ("tiling", "tiling.log", "view_quilt"),
];
const KINDS: [(&str, &str, &str); 5] = [("hata", "Hata", "Bug"), ("cokme", "Çökme", "Crash"), ("performans", "Performans", "Performance"), ("istek", "İstek", "Request"), ("oneri", "Öneri", "Suggestion")];

#[derive(Clone, Debug, PartialEq)]
pub(super) enum BugHit {
  Close,
  Kind(&'static str),
  File(usize),
  Retry(usize),
  Send,
}

pub(in crate::native_bar) enum BugEv {
  Device(Option<Value>),
  File(usize, Result<(String, usize), String>),
  Progress(u32),
  Sent(Result<(), String>),
}

#[derive(Clone)]
enum Phase {
  Loading,
  Ready(String, usize),
  Error,
}

pub(super) struct Bug {
  english: bool,
  kind: &'static str,
  files: Vec<Phase>,
  selected: Vec<bool>,
  device: Option<Value>,
  device_error: bool,
  sending: bool,
  progress: Option<u32>,
  pub(super) sent: bool,
  error: String,
}

impl Bug {
  fn t(&self, tr: &'static str, en: &'static str) -> &'static str {
    if self.english {
      en
    } else {
      tr
    }
  }

  fn loading(&self) -> bool {
    self.files.iter().zip(&self.selected).any(|(f, s)| *s && matches!(f, Phase::Loading))
  }
}

/// "2026-10-02 14:05" now (local time)
fn now_local() -> String {
  use windows::Win32::{
    Foundation::{FILETIME, SYSTEMTIME},
    System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
  };
  let ms = super::notifs::now_ms().max(0) as u64;
  let ticks = (ms / 1000 + 11_644_473_600) * 10_000_000;
  let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
  let (mut utc, mut l) = (SYSTEMTIME::default(), SYSTEMTIME::default());
  unsafe {
    if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut l).is_err() {
      return String::new();
    }
  }
  format!("{:04}-{:02}-{:02} {:02}:{:02}", l.wYear, l.wMonth, l.wDay, l.wHour, l.wMinute)
}

/// "YYYY-MM-DD HH:MM" (or with a T) -> its parts, None when it is no date.
pub(super) fn parse_local(s: &str) -> Option<(u16, u16, u16, u16, u16)> {
  let s = s.trim().replace('T', " ");
  let (d, t) = s.split_once(' ')?;
  let mut dp = d.split('-');
  let (y, mo, da) = (dp.next()?.parse().ok()?, dp.next()?.parse().ok()?, dp.next()?.parse().ok()?);
  let (h, mi) = t.trim().split_once(':')?;
  let (h, mi): (u16, u16) = (h.parse().ok()?, mi.parse().ok()?);
  let ok = (1..=12).contains(&mo) && (1..=31).contains(&da) && h < 24 && mi < 60 && (2000..=2100).contains(&y);
  ok.then_some((y, mo, da, h, mi))
}

/// A local "YYYY-MM-DD HH:MM" as UTC ISO 8601.
fn to_iso(s: &str) -> Option<String> {
  use windows::Win32::{Foundation::SYSTEMTIME, System::Time::TzSpecificLocalTimeToSystemTime};
  let (y, mo, d, h, mi) = parse_local(s)?;
  let local = SYSTEMTIME { wYear: y, wMonth: mo, wDay: d, wHour: h, wMinute: mi, ..Default::default() };
  let mut utc = SYSTEMTIME::default();
  unsafe { TzSpecificLocalTimeToSystemTime(None, &local, &mut utc).ok()? };
  Some(format!("{:04}-{:02}-{:02}T{:02}:{:02}:00.000Z", utc.wYear, utc.wMonth, utc.wDay, utc.wHour, utc.wMinute))
}

pub(super) fn paint(cx: &mut Cx, sb: &mut Sidebar, _m: &Model, panel: Rect) -> anyhow::Result<()> {
  // `.bug-overlay`: the panel dimmed, the dialog over it
  cx.round(panel, 19.0, Rgba(7, 6, 10, 0.76))?;
  cx.hit(panel, Hit::Bug(BugHit::Close));
  let Some(b) = sb.bug.as_ref() else { return Ok(()) };
  let english = b.english;
  let t = |tr: &'static str, en: &'static str| if english { en } else { tr };
  let d = Rect::new(panel.x + 12.0, panel.y + 12.0, panel.w - 24.0, panel.h - 24.0);
  cx.shadow(d, 24.0, 1.2)?;
  cx.round(d, 24.0, cx.t.layer0)?;
  cx.p.stroke_round(d.inset(0.5, 0.5), 24.0, cx.t.outline_variant, 1.0)?;
  cx.hit(d, Hit::Panel);
  cx.push_clip(d);
  // head
  let hi = Rect::new(d.x + 16.0, d.y + 16.0, 40.0, 40.0);
  cx.round(hi, 14.0, cx.t.primary_container)?;
  cx.icon("bug_report", hi.x + 20.0, hi.y + 20.0, 22.0, false, cx.t.on_primary_container)?;
  cx.text(t("Hata bildirimi", "Issue report"), Rect::new(hi.right() + 12.0, d.y + 16.0, d.w - 130.0, 22.0), stw(17.0, 650.0), cx.t.on_layer1)?;
  cx.text(
    t("Ne oldu? İlgili günlükleri ekleyerek anlat.", "Describe what happened and attach the relevant logs."),
    Rect::new(hi.right() + 12.0, d.y + 38.0, d.w - 130.0, 16.0),
    st(11.0),
    cx.t.on_surface_variant,
  )?;
  if !b.sending {
    cx.round_btn(Rect::new(d.right() - 16.0 - 32.0, d.y + 20.0, 32.0, 32.0), "close", 20.0, false, None, cx.t.on_layer1, Hit::Bug(BugHit::Close))?;
  }
  let head_b = d.y + 16.0 + 40.0 + 14.0;
  cx.round(Rect::new(d.x, head_b, d.w, 1.0), 0.0, cx.c.border0)?;
  if b.sent {
    let cy = head_b + 35.0;
    cx.icon("check_circle", d.x + d.w / 2.0, cy + 21.0, 42.0, false, Rgba::hex(0x81d69e))?;
    cx.text_center(t("Rapor gönderildi", "Report sent"), Rect::new(d.x, cy + 50.0, d.w, 24.0), stw(17.0, 600.0), cx.t.on_layer1)?;
    cx.text_center(
      t("Seçtiğin ekler rapora eklendi.", "The selected attachments were included."),
      Rect::new(d.x, cy + 78.0, d.w, 18.0),
      st(12.0),
      cx.t.on_surface_variant,
    )?;
    let btn = Rect::new(d.x + d.w / 2.0 - 70.0, cy + 110.0, 140.0, 42.0);
    submit_button(cx, btn, t("Kapat", "Close"), None, false, true, Hit::Bug(BugHit::Close))?;
    cx.pop_clip();
    return Ok(());
  }
  let footer_h = 12.0 + 30.0 + 10.0 + 42.0 + 16.0;
  let foot_y = d.bottom() - footer_h;
  let mut y = head_b + 16.0;
  let x = d.x + 16.0;
  let w = d.w - 32.0;
  // kinds
  let mut cx_ = x;
  for (k, tr_, en) in KINDS {
    let label = if english { en } else { tr_ };
    let lw = cx.measure(label, st(12.0))?.ceil() + 24.0;
    if cx_ + lw > x + w {
      cx_ = x;
      y += 32.0 + 6.0;
    }
    let r = Rect::new(cx_, y, lw, 32.0);
    let on = b.kind == k;
    let hit = Hit::Bug(BugHit::Kind(k));
    if on {
      cx.round(r, 16.0, cx.t.primary_container)?;
      cx.p.stroke_round(r.inset(0.5, 0.5), 16.0, cx.t.primary, 1.0)?;
    } else {
      if cx.hot(&hit) {
        cx.round(r, 16.0, cx.t.layer1_hover)?;
      }
      cx.p.stroke_round(r.inset(0.5, 0.5), 16.0, cx.t.outline_variant, 1.0)?;
    }
    cx.text_center(label, r, st(12.0), if on { cx.t.on_primary_container } else { cx.t.on_surface_variant })?;
    if !b.sending {
      cx.hit(r, hit);
    }
    cx_ += lw + 6.0;
  }
  y += 32.0 + 16.0;
  // when
  let half = (w - 10.0) / 2.0;
  for (i, (id, label)) in [(FieldId::BugStart, t("Başlangıç", "Started")), (FieldId::BugEnd, t("Bitiş", "Ended"))].into_iter().enumerate() {
    let fx = x + i as f32 * (half + 10.0);
    cx.text(label, Rect::new(fx, y, half, 16.0), stw(12.0, 550.0), cx.t.on_surface_variant)?;
    let mut f = sb.fields.remove(&id).unwrap_or_else(|| TextField::new(false));
    cx.field_box(Rect::new(fx, y + 22.0, half, 38.0), 12.0, &mut f, id, "2026-01-31 18:30", st(12.0), 11.0, Some(cx.t.layer1))?;
    sb.fields.insert(id, f);
  }
  y += 22.0 + 38.0 + 16.0;
  cx.text(t("Hatayı açıkla", "Describe the issue"), Rect::new(x, y, w, 16.0), stw(12.0, 550.0), cx.t.on_surface_variant)?;
  let mut f = sb.fields.remove(&FieldId::BugText).unwrap_or_else(|| TextField::new(true));
  let ph = t("Ne yapıyordun, ne bekliyordun, ne oldu?", "What were you doing, what did you expect, and what happened?");
  let text_h = ((foot_y - (y + 22.0)) - 16.0 - 24.0 - 4.0 * 48.0 - 20.0 - 60.0).clamp(92.0, 220.0);
  cx.field_box(Rect::new(x, y + 22.0, w, text_h), 12.0, &mut f, FieldId::BugText, ph, st(12.0), 11.0, Some(cx.t.layer1))?;
  sb.fields.insert(FieldId::BugText, f);
  y += 22.0 + text_h + 16.0;
  let b = sb.bug.as_ref().unwrap();
  cx.text(t("Tanı ekleri", "Diagnostic attachments"), Rect::new(x, y, w, 18.0), stw(13.0, 600.0), cx.t.on_layer1)?;
  cx.text(t("Son günlük kesitleri gönderilir.", "Recent log excerpts are sent."), Rect::new(x, y + 18.0, w, 14.0), st(11.0), cx.t.on_surface_variant)?;
  y += 38.0;
  for (i, (_, name, icon)) in FILES.iter().enumerate() {
    let r = Rect::new(x, y, w, 44.0);
    let included = b.selected[i];
    let a = if included { 1.0 } else { 0.52 };
    cx.round(r, 12.0, cx.t.layer1.alpha(a))?;
    cx.p.stroke_round(r.inset(0.5, 0.5), 12.0, cx.c.border0.alpha(cx.c.border0.3 * a), 1.0)?;
    cx.checkbox(r.x + 9.0, r.y + 14.5, 15.0, included)?;
    if !b.sending {
      cx.hit(Rect::new(r.x, r.y, 34.0, r.h), Hit::Bug(BugHit::File(i)));
    }
    cx.icon(icon, r.x + 34.0 + 9.5, r.y + 22.0, 19.0, false, cx.t.primary.alpha(a))?;
    let tx = r.x + 34.0 + 19.0 + 8.0;
    cx.text(name, Rect::new(tx, r.y + 6.0, r.w - (tx - r.x) - 90.0, 16.0), stw(12.0, 550.0), cx.t.on_layer1.alpha(a))?;
    let status = match &b.files[i] {
      Phase::Loading => t("Toplanıyor…", "Collecting…").to_string(),
      Phase::Error => t("Bulunamadı veya okunamadı", "Missing or unreadable").to_string(),
      Phase::Ready(_, bytes) => {
        let state = if b.sent {
          t("Gönderildi", "Sent").to_string()
        } else if b.sending && included {
          format!("{}{}", t("Gönderiliyor", "Uploading"), b.progress.map_or("…".to_string(), |p| format!(" {p}%")))
        } else {
          t("Hazır", "Ready").to_string()
        };
        format!("{} KB · {}", bytes.div_ceil(1024), state)
      }
    };
    cx.text(&status, Rect::new(tx, r.y + 23.0, r.w - (tx - r.x) - 90.0, 14.0), st(10.0), cx.t.on_surface_variant.alpha(a))?;
    let right = r.right() - 10.0;
    match &b.files[i] {
      Phase::Loading if included => cx.spinner(right - 9.0, r.y + 22.0, 18.0, cx.t.primary)?,
      Phase::Error => {
        let rl = t("Tekrar dene", "Retry");
        let rw = cx.measure(rl, st(11.0))?.ceil() + 14.0;
        let rb = Rect::new(right - rw, r.y + 9.0, rw, 26.0);
        let hit = Hit::Bug(BugHit::Retry(i));
        if cx.hot(&hit) {
          cx.round(rb, 8.0, Rgba(242, 170, 162, 0.12))?;
        }
        cx.text_center(rl, rb, st(11.0), Rgba::hex(0xf2aaa2))?;
        if !b.sending {
          cx.hit(rb, hit);
        }
        cx.icon("cancel", rb.x - 12.0, r.y + 22.0, 19.0, false, Rgba::hex(0xf2aaa2))?;
      }
      Phase::Ready(..) if b.sending && included => match b.progress {
        Some(p) => {
          cx.p.ring(right - 9.0, r.y + 22.0, 7.0, 3.0, p as f32 / 100.0, cx.t.outline_variant, cx.t.primary)?;
        }
        None => cx.spinner(right - 9.0, r.y + 22.0, 18.0, cx.t.primary)?,
      },
      Phase::Ready(..) => cx.icon(if b.sent { "check_circle" } else { "check" }, right - 9.0, r.y + 22.0, 19.0, false, Rgba::hex(0x81d69e))?,
      _ => {}
    }
    y += 48.0;
  }
  // device
  let dev = &b.device;
  let line1 = dev.as_ref().and_then(|d| d["os"].as_str().map(str::to_string)).unwrap_or_else(|| t("Alınıyor…", "Loading…").to_string());
  let line2 = dev
    .as_ref()
    .map(|d| ["cpu", "gpu", "ram"].iter().filter_map(|k| d[*k].as_str().filter(|s| !s.is_empty()).map(str::to_string)).collect::<Vec<_>>().join(" · "))
    .unwrap_or_default();
  let dh = 10.0 + 16.0 + 16.0 + if line2.is_empty() { 0.0 } else { 16.0 } + if b.device_error { 16.0 } else { 0.0 } + 10.0;
  let dr = Rect::new(x, y + 4.0, w, dh);
  if dr.bottom() < foot_y {
    cx.round(dr, 12.0, cx.t.layer1)?;
    let mut ly = dr.y + 10.0;
    cx.text(t("Cihaz", "Device"), Rect::new(dr.x + 12.0, ly, dr.w - 24.0, 16.0), stw(12.0, 600.0), cx.t.on_layer1)?;
    ly += 16.0;
    if b.device_error {
      cx.text(
        t("Cihaz bilgisi okunamadı; rapor yine gönderilebilir.", "Device details unavailable; the report can still be sent."),
        Rect::new(dr.x + 12.0, ly, dr.w - 24.0, 16.0),
        st(11.0),
        cx.t.on_surface_variant,
      )?;
      ly += 16.0;
    }
    cx.text(&line1, Rect::new(dr.x + 12.0, ly, dr.w - 24.0, 16.0), st(11.0), cx.t.on_surface_variant)?;
    ly += 16.0;
    if !line2.is_empty() {
      cx.text(&line2, Rect::new(dr.x + 12.0, ly, dr.w - 24.0, 16.0), st(11.0), cx.t.on_surface_variant)?;
    }
    y = dr.bottom() + 10.0;
  }
  if !b.error.is_empty() && y + 30.0 < foot_y {
    let eh = cx.wrapped_h(&b.error, st(12.0), w - 50.0, foot_y - y - 20.0)?.max(18.0) + 20.0;
    let er = Rect::new(x, y, w, eh);
    cx.round(er, 12.0, Rgba(242, 170, 162, 0.13))?;
    cx.icon("error", er.x + 12.0 + 9.0, er.y + 19.0, 18.0, false, Rgba::hex(0xf2aaa2))?;
    cx.p.text_wrapped(&b.error, Rect::new(er.x + 38.0, er.y + 10.0, er.w - 50.0, eh - 18.0), st(12.0), Rgba::hex(0xf2aaa2), false)?;
  }
  // footer
  cx.round(Rect::new(d.x, foot_y, d.w, 1.0), 0.0, cx.c.border0)?;
  cx.p.text_wrapped(
    t("Seçtiğin günlüklerde pencere adları ve dosya yolları bulunabilir.", "Selected logs may contain window titles and file paths."),
    Rect::new(x, foot_y + 12.0, w, 30.0),
    st(10.0),
    cx.t.on_surface_variant,
    false,
  )?;
  let btn = Rect::new(x, d.bottom() - 16.0 - 42.0, w, 42.0);
  let enabled = !b.sending && !b.loading();
  let (label, icon) = if b.sending { (t("Gönderiliyor…", "Sending…"), None) } else { (t("Raporu gönder", "Send report"), Some("send")) };
  submit_button(cx, btn, label, icon, b.sending, enabled, Hit::Bug(BugHit::Send))?;
  cx.pop_clip();
  Ok(())
}

fn submit_button(cx: &mut Cx, r: Rect, label: &str, icon: Option<&str>, spinning: bool, enabled: bool, hit: Hit) -> anyhow::Result<()> {
  let a = if enabled || spinning { 1.0 } else { 0.55 };
  let bg = if enabled && cx.hot(&hit) { super::kit::blend(cx.t.primary, Rgba(255, 255, 255, 1.0), 0.08) } else { cx.t.primary };
  cx.round(r, 13.0, bg.alpha(a))?;
  let lw = cx.measure(label, stw(13.0, 600.0))?.ceil();
  let extra = if icon.is_some() || spinning { 19.0 + 9.0 } else { 0.0 };
  let x0 = r.x + (r.w - lw - extra) / 2.0;
  if spinning {
    cx.spinner(x0 + 9.0, r.y + r.h / 2.0, 18.0, cx.t.on_primary)?;
  } else if let Some(i) = icon {
    cx.icon(i, x0 + 9.5, r.y + r.h / 2.0, 19.0, false, cx.t.on_primary.alpha(a))?;
  }
  cx.text(label, Rect::new(x0 + extra, r.y, lw + 2.0, r.h), stw(13.0, 600.0), cx.t.on_primary.alpha(a))?;
  if enabled {
    cx.hit(r, hit);
  }
  Ok(())
}

// ------------------------------------------------------------------ workers

fn ev(e: BugEv) {
  send(Msg::Sidebar(Ev::Bug(e)));
}

fn collect(i: usize, english: bool) {
  let kind = FILES[i].0;
  std::thread::spawn(move || {
    let out = core_api::run_core_output(&["--bug-report-file", kind]).and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let r = match out {
      Some(v) if v["ok"].as_bool() == Some(true) && v["text"].as_str().is_some_and(|t| !t.trim().is_empty()) => {
        let text = v["text"].as_str().unwrap_or("").to_string();
        let bytes = v["bytes"].as_u64().map_or(text.len(), |b| b as usize);
        Ok((text, bytes))
      }
      Some(v) => Err(v["error"].as_str().map(str::to_string).unwrap_or_else(|| if english { "Log could not be read." } else { "Günlük okunamadı." }.to_string())),
      None => Err(String::new()),
    };
    ev(BugEv::File(i, r));
  });
}

/// HTTPS POST with the upload progress (WinHTTP); the status and body.
fn post(url_path: &str, headers: &str, body: &[u8], progress: impl Fn(u32)) -> Result<(u32, String), String> {
  use windows::{
    core::{w, HSTRING, PCWSTR},
    Win32::Networking::WinHttp::*,
  };
  let host = SUPABASE_URL.trim_start_matches("https://").trim_end_matches('/').to_string();
  unsafe {
    let session = WinHttpOpen(w!("LogicalLunge"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
    if session.is_null() {
      return Err("session".into());
    }
    let _ = WinHttpSetTimeouts(session, 15_000, 15_000, 45_000, 45_000);
    let result = (|| -> Result<(u32, String), String> {
      let conn = WinHttpConnect(session, &HSTRING::from(host.as_str()), INTERNET_DEFAULT_HTTPS_PORT as u16, 0);
      if conn.is_null() {
        return Err("connect".into());
      }
      let req = WinHttpOpenRequest(conn, w!("POST"), &HSTRING::from(url_path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE);
      if req.is_null() {
        let _ = WinHttpCloseHandle(conn);
        return Err("request".into());
      }
      let done = (|| -> Result<(u32, String), String> {
        let h: Vec<u16> = headers.encode_utf16().collect();
        WinHttpSendRequest(req, Some(&h), None, 0, body.len() as u32, 0).map_err(|e| e.message())?;
        let mut sent = 0usize;
        for chunk in body.chunks(16 * 1024) {
          let mut wrote = 0u32;
          WinHttpWriteData(req, Some(chunk.as_ptr().cast()), chunk.len() as u32, &mut wrote).map_err(|e| e.message())?;
          sent += wrote as usize;
          // all bytes out is not yet the server's answer
          progress(((sent * 100) / body.len().max(1)).min(99) as u32);
        }
        WinHttpReceiveResponse(req, std::ptr::null_mut()).map_err(|e| e.message())?;
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
          req,
          WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
          PCWSTR::null(),
          Some((&mut status as *mut u32).cast()),
          &mut len,
          std::ptr::null_mut(),
        )
        .map_err(|e| e.message())?;
        let mut out = Vec::new();
        loop {
          let mut avail = 0u32;
          if WinHttpQueryDataAvailable(req, &mut avail).is_err() || avail == 0 || out.len() > 64 * 1024 {
            break;
          }
          let mut buf = vec![0u8; avail as usize];
          let mut read = 0u32;
          if WinHttpReadData(req, buf.as_mut_ptr().cast(), avail, &mut read).is_err() || read == 0 {
            break;
          }
          out.extend_from_slice(&buf[..read as usize]);
        }
        Ok((status, String::from_utf8_lossy(&out).to_string()))
      })();
      let _ = WinHttpCloseHandle(req);
      let _ = WinHttpCloseHandle(conn);
      done
    })();
    let _ = WinHttpCloseHandle(session);
    result
  }
}

/// Sends the report; the old two-column table refuses the separate log
/// columns without inserting anything: then once more with its combined one.
fn upload(base: Value, logs: [(String, Option<String>); 3], english: bool) -> Result<(), String> {
  let headers = format!("apikey: {SUPABASE_KEY}\r\nContent-Type: application/json\r\nPrefer: return=minimal\r\n");
  let progress = |p: u32| ev(BugEv::Progress(p));
  let mut first = base.clone();
  first["core_log"] = json!(logs[0].1);
  first["shell_log"] = json!(logs[1].1);
  first["tiling_log"] = json!(logs[2].1);
  first["system_log"] = Value::Null;
  let net = |e: String| format!("{}{}", if english { "Network connection failed. " } else { "Ağ bağlantısı kurulamadı. " }, e);
  let (status, body) = post("/rest/v1/bug_reports", &headers, first.to_string().as_bytes(), progress).map_err(net)?;
  if (200..300).contains(&status) {
    return Ok(());
  }
  let detail: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
  let legacy = status == 400
    && detail["code"].as_str() == Some("PGRST204")
    && ["core_log", "shell_log", "tiling_log"].iter().any(|c| detail["message"].as_str().unwrap_or("").contains(c));
  if !legacy {
    return Err(format!("{}: {}", status, body.chars().take(240).collect::<String>()));
  }
  let sections: Vec<String> = logs.iter().filter_map(|(name, text)| text.as_ref().map(|t| format!("===== {name} =====\n{t}"))).collect();
  let mut second = base;
  second["system_log"] = if sections.is_empty() { Value::Null } else { json!(sections.join("\n\n")) };
  let (status, body) = post("/rest/v1/bug_reports", &headers, second.to_string().as_bytes(), |p| ev(BugEv::Progress(p))).map_err(net)?;
  if (200..300).contains(&status) {
    Ok(())
  } else {
    Err(format!("{}: {}", status, body.chars().take(240).collect::<String>()))
  }
}

impl Ui {
  pub(super) fn sb_bug_open(&mut self) {
    // an unsent report (the panel closed while it was written) is kept
    if self.sidebar.bug.as_ref().is_some_and(|b| !b.sent) {
      self.sidebar.focus = Some(FieldId::BugText);
      return;
    }
    let english = !self.model.locale().to_lowercase().starts_with("tr");
    self.sidebar.bug = Some(Bug {
      english,
      kind: "hata",
      files: vec![Phase::Loading; FILES.len()],
      selected: vec![true; FILES.len()],
      device: None,
      device_error: false,
      sending: false,
      progress: None,
      sent: false,
      error: String::new(),
    });
    let now = now_local();
    self.sidebar.field(FieldId::BugStart).set(&now);
    self.sidebar.field(FieldId::BugEnd).set(&now);
    let draft = self.sidebar.store.bug_draft.clone();
    self.sidebar.field(FieldId::BugText).set(&draft);
    self.sidebar.focus = Some(FieldId::BugText);
    std::thread::spawn(move || {
      let device = core_api::run_core_output(&["--bug-report-device"]).and_then(|s| serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object);
      ev(BugEv::Device(device));
      // the core log should hold the black box record made first
      let out = core_api::run_core_output(&["--bug-report-file", "blackbox"]).and_then(|s| serde_json::from_str::<Value>(&s).ok());
      let r = match out {
        Some(v) if v["ok"].as_bool() == Some(true) && v["text"].as_str().is_some_and(|t| !t.trim().is_empty()) => {
          let text = v["text"].as_str().unwrap_or("").to_string();
          let bytes = v["bytes"].as_u64().map_or(text.len(), |b| b as usize);
          Ok((text, bytes))
        }
        _ => Err(String::new()),
      };
      ev(BugEv::File(0, r));
      for i in 1..FILES.len() {
        collect(i, english);
      }
    });
  }

  pub(super) fn sb_bug_event(&mut self, e: BugEv) {
    let Some(b) = self.sidebar.bug.as_mut() else { return };
    match e {
      BugEv::Device(d) => {
        b.device_error = d.is_none();
        b.device = d;
      }
      BugEv::File(i, r) => {
        if let Some(f) = b.files.get_mut(i) {
          *f = match r {
            Ok((text, bytes)) => Phase::Ready(text, bytes),
            Err(_) => Phase::Error,
          };
        }
      }
      BugEv::Progress(p) => b.progress = Some(p),
      BugEv::Sent(r) => {
        b.sending = false;
        b.progress = None;
        match r {
          Ok(()) => {
            b.sent = true;
            let (title, body) = (b.t("Rapor gönderildi", "Report sent"), b.t("Hata raporu kaydedildi.", "The issue report was saved."));
            self.sidebar.store.bug_draft.clear();
            self.sidebar.save_soon();
            self.toast_add(json!({ "kind": "ok", "title": title, "body": body, "icon": "check_circle" }));
          }
          Err(e) => b.error = format!("{}{}", b.t("Gönderilemedi: ", "Could not send: "), e),
        }
      }
    }
    self.sb_render();
  }

  fn sb_bug_send(&mut self, allow_missing: bool) {
    let start = self.sidebar.field(FieldId::BugStart).text();
    let end = self.sidebar.field(FieldId::BugEnd).text();
    let text = self.sidebar.field(FieldId::BugText).text();
    let Some(b) = self.sidebar.bug.as_mut() else { return };
    if b.sending {
      return;
    }
    if text.trim().is_empty() {
      b.error = b.t("Hatayı kısaca açıklayın.", "Please describe the issue.").into();
      return;
    }
    let (Some(start_iso), end_iso) = (to_iso(&start), to_iso(&end)) else {
      b.error = b.t("Geçerli bir zaman aralığı seçin.", "Choose a valid time range.").into();
      return;
    };
    if !end.trim().is_empty() && end_iso.as_ref().map_or(true, |e| e < &start_iso) {
      b.error = b.t("Geçerli bir zaman aralığı seçin.", "Choose a valid time range.").into();
      return;
    }
    if b.loading() {
      return;
    }
    let missing: Vec<&str> = FILES.iter().enumerate().filter(|(i, _)| b.selected[*i] && matches!(b.files[*i], Phase::Error)).map(|(_, f)| f.1).collect();
    if !missing.is_empty() && !allow_missing {
      let english = b.english;
      let t = |tr: &'static str, en: &'static str| if english { en } else { tr };
      let spec = Spec::new(
        Kind::Warning,
        t("Bazı günlükler eklenemedi", "Some logs could not be attached"),
        t("Kırmızı işaretli ekleri yeniden deneyebilir veya raporu onlar olmadan gönderebilirsin.", "Retry the marked attachments or send the report without them."),
        vec![t("Eksik günlüklerle gönder", "Send without missing logs").to_string(), t("Geri dön", "Go back").to_string()],
      )
      .cancel(1)
      .default_button(1);
      self.sb_modal(true);
      self.dialog_open(spec, |ui: &mut Ui, answer| {
        ui.sb_modal(false);
        if answer.button == Some(0) {
          ui.sb_bug_send(true);
        }
        ui.sb_render();
      });
      return;
    }
    b.error.clear();
    b.sending = true;
    b.progress = None;
    let included = |i: usize| match (&b.files[i], b.selected[i]) {
      (Phase::Ready(t, _), true) => Some(t.clone()),
      _ => None,
    };
    let device = b.device.clone().unwrap_or(Value::Null);
    let (mon, _) = super::super::toast::primary();
    let tz = timezone();
    let base = json!({
      "bug_type": b.kind,
      "incident_start": start_iso,
      "incident_end": end_iso,
      "description": text.trim(),
      "os_version": device["os"],
      "cpu": device["cpu"],
      "gpu": device["gpu"],
      "ram": device["ram"],
      "blackbox_log": included(0),
      "device_info": {
        "appVersion": device["appVersion"],
        "resolution": format!("{}x{}", mon.right - mon.left, mon.bottom - mon.top),
        "uptimeMs": sysinfo::System::uptime() * 1000,
        "timezone": tz,
        "missingLogs": missing,
      },
    });
    let logs = [("core.log".to_string(), included(1)), ("shell.log".to_string(), included(2)), ("tiling.log".to_string(), included(3))];
    let english = b.english;
    std::thread::spawn(move || {
      let r = upload(base, logs, english);
      std::thread::sleep(Duration::from_millis(50));
      ev(BugEv::Sent(r));
    });
  }

  pub(super) fn sb_bug_click(&mut self, h: BugHit, button: u8) {
    if button != 0 {
      return;
    }
    let sending = self.sidebar.bug.as_ref().is_some_and(|b| b.sending);
    match h {
      BugHit::Close if !sending => self.sb_page_close(),
      BugHit::Kind(k) => {
        if let Some(b) = self.sidebar.bug.as_mut() {
          b.kind = k;
        }
      }
      BugHit::File(i) => {
        if let Some(b) = self.sidebar.bug.as_mut() {
          b.selected[i] = !b.selected[i];
        }
      }
      BugHit::Retry(i) => {
        if let Some(b) = self.sidebar.bug.as_mut() {
          b.files[i] = Phase::Loading;
          collect(i, b.english);
        }
      }
      BugHit::Send => self.sb_bug_send(false),
      _ => {}
    }
    self.sb_render();
  }

  pub(super) fn sb_bug_typed(&mut self, id: FieldId, t: Typed) {
    if let Some(b) = self.sidebar.bug.as_mut() {
      if t == Typed::Changed {
        b.error.clear();
      }
    }
    if t == Typed::Submit && id != FieldId::BugText {
      self.sidebar.focus = Some(FieldId::BugText);
    }
  }
}

/// Windows' time zone name ("Turkey Standard Time").
fn timezone() -> String {
  use windows::Win32::System::Time::{GetDynamicTimeZoneInformation, DYNAMIC_TIME_ZONE_INFORMATION};
  let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
  unsafe {
    GetDynamicTimeZoneInformation(&mut info);
  }
  let n = info.TimeZoneKeyName.iter().position(|&c| c == 0).unwrap_or(info.TimeZoneKeyName.len());
  String::from_utf16_lossy(&info.TimeZoneKeyName[..n])
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn report_times_are_read() {
    assert_eq!(parse_local("2026-10-02 14:05"), Some((2026, 10, 2, 14, 5)));
    assert_eq!(parse_local("2026-10-02T14:05"), Some((2026, 10, 2, 14, 5)));
    assert_eq!(parse_local("2026-13-02 14:05"), None);
    assert_eq!(parse_local("dün"), None);
  }
}
