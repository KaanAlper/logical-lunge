//! The window's frame (settings.css `.win`: the page list on the left, the
//! title and the scrolled page on the right) and the pages.

use serde_json::Value;

use super::{
  is_light,
  widgets::{card_border, ink, layer3, Btn, Ctrl, Ctx, Cursor, Row, BODY, SUB},
  workspaces, Act, Fld, Hit, Key, Sel, Settings, Sl, Sw, APPLY_H, BODY_PAD, CARD_H, CARD_W, HEAD_H, M,
  NAV_W, PAGES,
};
use crate::native_bar::{
  fonts::TextStyle,
  gfx::{Rect, Rgba},
  view::{Align, Painter, Theme},
};

/// settings.html COLORS (the old #b69df8 purple first)
pub const COLORS: [(&str, &str); 7] = [
  ("Mor", "#b69df8"),
  ("Mavi", "#8ab4f8"),
  ("Camgöbeği", "#7fd4c9"),
  ("Yeşil", "#a6d189"),
  ("Pembe", "#f5a3c7"),
  ("Turuncu", "#ffb77c"),
  ("Kırmızı", "#f28b82"),
];

/// settings.html LANGS
pub const LANGS: [(&str, &str); 15] = [
  ("system", "Sistem dili"),
  ("tr", "Türkçe"),
  ("en", "English"),
  ("de", "Deutsch"),
  ("fr", "Français"),
  ("es", "Español"),
  ("it", "Italiano"),
  ("pt", "Português"),
  ("ru", "Русский"),
  ("uk", "Українська"),
  ("pl", "Polski"),
  ("ja", "日本語"),
  ("zh", "中文"),
  ("ko", "한국어"),
  ("ar", "العربية"),
];

const PART_NAMES: [(&str, &str, &str); 3] = [("core", "Çekirdek", "memory"), ("tiling", "Pencere yöneticisi", "grid_view"), ("shell", "Kabuk", "web_asset")];

pub fn slider_range(sl: Sl) -> (i32, i32, i32) {
  match sl {
    Sl::ToastInfo | Sl::ToastError => (1, 30, 1),
    Sl::NightLevel => (0, 100, 5),
  }
}

pub fn slider_value(s: &Settings, sl: Sl) -> i32 {
  match sl {
    Sl::ToastInfo => s.s["toastInfo"].as_i64().unwrap_or(3) as i32,
    Sl::ToastError => s.s["toastError"].as_i64().unwrap_or(5) as i32,
    Sl::NightLevel => s.night_level,
  }
}

pub fn seg_len(key: Key) -> usize {
  match key {
    Key::NightMode => 3,
    _ => 2,
  }
}

/// A drop-down box's items (labels, to be translated) and the current one.
pub fn list_for(s: &Settings, sel: Sel) -> (Vec<String>, usize) {
  match sel {
    Sel::Lang => {
      let lang = s.s["language"].as_str().unwrap_or("system");
      (LANGS.iter().map(|l| l.1.to_string()).collect(), LANGS.iter().position(|l| l.0 == lang).unwrap_or(0))
    }
    Sel::SegMon(seg) => (s.ws.mons.iter().map(|m| m.label()).collect(), s.ws.segment_monitor_pos(seg)),
  }
}

/// Everything, at window DIP (the card at (M, M)).
pub fn paint(p: &mut Painter, t: &Theme, s: &mut Settings, tr: &dyn Fn(&str) -> String) -> anyhow::Result<()> {
  let card = Rect::new(M, M, CARD_W, CARD_H);
  // box-shadow: 0 10px 30px rgba(0 0 0 / 45%)
  for k in 1..=8 {
    let g = k as f32 * 1.8;
    p.fill_round(Rect::new(card.x - g, card.y + 4.0 - g, card.w + 2.0 * g, card.h + 2.0 * g), 26.0 + g, Rgba(0, 0, 0, 0.035))?;
  }
  // the page list's side is layer1, the page's layer0
  p.fill_round(card, 26.0, t.layer1)?;
  p.fill_round(Rect::new(card.x + NAV_W, card.y, CARD_W - NAV_W, CARD_H), 26.0, t.layer0)?;
  p.fill(Rect::new(card.x + NAV_W, card.y, 30.0, CARD_H), t.layer0)?;
  p.stroke_round(card.inset(0.5, 0.5), 26.0, card_border(t), 1.0)?;

  let mut regions = std::mem::take(&mut s.regions);
  regions.clear();
  let typing = match s.focus {
    Some(Hit::Field(f)) => Some(f),
    _ => None,
  };
  let view = Rect::new(card.x + NAV_W, card.y + HEAD_H, CARD_W - NAV_W, s.view_h());
  let mut cx = Ctx { p, t, tr, regions: &mut regions, hover: s.hover, focus: s.focus, ring: s.focus_ring, clip: card, typing };

  nav(&mut cx, s)?;
  head(&mut cx, s)?;

  // the page, scrolled and clipped
  cx.clip = view;
  cx.push_clip(view);
  let x = view.x + BODY_PAD;
  let w = view.w - 2.0 * BODY_PAD;
  let top = view.y - s.scroll;
  let end = match s.page {
    0 => look(&mut cx, s, x, top, w),
    super::PAGE_WORKSPACES => workspaces::paint(&mut cx, s, x, top, w),
    2 => lang(&mut cx, s, x, top, w),
    3 => keys(&mut cx, x, top, w),
    super::PAGE_NIGHT => night(&mut cx, s, x, top, w),
    super::PAGE_HEALTH => health(&mut cx, s, x, top, w),
    6 => advanced(&mut cx, s, x, top, w),
    _ => about(&mut cx, s, x, top, w),
  };
  cx.pop_clip();
  let end = end?;
  s.content_h = end - top + BODY_PAD;
  scrollbar(&mut cx, s, view)?;
  cx.clip = card;

  if s.dirty {
    apply_note(&mut cx, card)?;
  }
  s.regions = regions;
  Ok(())
}

fn nav(cx: &mut Ctx, s: &Settings) -> anyhow::Result<()> {
  let t = cx.t;
  let (x, y) = (M + 12.0, M + 18.0);
  // .nav-title
  cx.p.icon("settings", x + 12.0 + 13.0, y + 6.0 + 13.0, 26.0, true, t.primary)?;
  let title = cx.tr("Ayarlar");
  cx.p.text(&title, Rect::new(x + 12.0 + 36.0, y + 6.0, NAV_W - 72.0, 26.0), TextStyle { size: 20.0, weight: 560.0 }, t.on_layer1, Align::Left, false)?;
  let mut by = y + 6.0 + 26.0 + 18.0;
  for (i, (icon, label)) in PAGES.iter().enumerate() {
    let r = Rect::new(x, by, NAV_W - 24.0, 44.0);
    let hit = Hit::Nav(i);
    let sel = s.page == i;
    let fg = if sel { t.on_sec_container } else { t.on_surface_variant };
    if sel {
      cx.p.fill_round(r, 22.0, t.sec_container)?;
    } else if cx.hot(hit) {
      cx.p.fill_round(r, 22.0, t.layer1_hover)?;
    }
    cx.p.icon(icon, r.x + 16.0 + 11.0, r.y + 22.0, 22.0, sel, fg)?;
    let label = cx.tr(label);
    let style = TextStyle { size: 14.0, weight: if sel { 560.0 } else { 450.0 } };
    cx.p.text(&label, Rect::new(r.x + 16.0 + 22.0 + 14.0, r.y, r.w - 66.0, 44.0), style, fg, Align::Left, false)?;
    // one Tab stop for the list: the open page (arrows move)
    cx.push(r, hit, sel, Cursor::Hand);
    cx.ring(r, 22.0, hit)?;
    by += 44.0 + 2.0;
  }
  Ok(())
}

fn head(cx: &mut Ctx, s: &Settings) -> anyhow::Result<()> {
  let t = cx.t;
  let x = M + NAV_W + 28.0;
  let title = cx.tr(PAGES[s.page.min(PAGES.len() - 1)].1);
  cx.p.text(&title, Rect::new(x, M, CARD_W - NAV_W - 28.0 - 72.0, HEAD_H), TextStyle { size: 22.0, weight: 520.0 }, t.on_layer0, Align::Left, false)?;
  let b = Rect::new(M + CARD_W - 16.0 - 40.0, M + 12.0, 40.0, 40.0);
  let hit = Hit::Act(Act::Close);
  if cx.hot(hit) {
    cx.p.fill_round(b, 20.0, t.layer1_hover)?;
  }
  cx.p.icon("close", b.x + 20.0, b.y + 20.0, 22.0, false, t.on_layer0)?;
  cx.push(b, hit, true, Cursor::Hand);
  cx.ring(b, 20.0, hit)
}

fn scrollbar(cx: &mut Ctx, s: &Settings, view: Rect) -> anyhow::Result<()> {
  cx.p.scrollbar(view, s.content_h, s.scroll, ink(cx.t, 0.18))?;
  Ok(())
}

/// settings.css `.apply-float`
fn apply_note(cx: &mut Ctx, card: Rect) -> anyhow::Result<()> {
  let t = cx.t;
  let r = Rect::new(card.x + NAV_W + 20.0, card.bottom() - APPLY_H + 2.0, CARD_W - NAV_W - 40.0, APPLY_H - 20.0);
  for k in 1..=5 {
    let g = k as f32 * 2.0;
    cx.p.fill_round(Rect::new(r.x - g, r.y + 4.0 - g, r.w + 2.0 * g, r.h + 2.0 * g), 18.0 + g, Rgba(0, 0, 0, 0.03))?;
  }
  cx.p.fill_round(r, 18.0, layer3(t))?;
  cx.p.stroke_round(r, 18.0, t.outline_variant, 1.0)?;
  cx.p.icon("pending_actions", r.x + 16.0 + 11.0, r.y + r.h / 2.0, 22.0, false, t.primary)?;
  let a = cx.tr("Arayüz değişiklikleri hazır");
  let b = cx.tr("Kabuğu yeniden açınca uygulanır.");
  let btn_label = cx.tr("Şimdi uygula");
  let bw = cx.ctrl_width(&Ctrl::Button { label: btn_label.clone(), icon: Some("refresh"), kind: Btn::Outline, hit: Hit::Act(Act::ApplyNow), disabled: false })?;
  let tx = r.x + 16.0 + 22.0 + 12.0;
  let tw = r.w - (tx - r.x) - bw - 24.0;
  cx.p.text(&a, Rect::new(tx, r.y + 5.0, tw, 20.0), TextStyle { size: 14.0, weight: 550.0 }, t.on_layer1, Align::Left, false)?;
  cx.p.text(&b, Rect::new(tx, r.y + 24.0, tw, 16.0), TextStyle { size: 11.0, weight: 400.0 }, t.on_surface_variant, Align::Left, false)?;
  cx.button(Rect::new(r.right() - 12.0 - bw, r.y + (r.h - 36.0) / 2.0, bw, 36.0), &btn_label, Some("refresh"), Btn::Outline, Hit::Act(Act::ApplyNow), false)
}

fn note(cx: &mut Ctx, x: f32, y: f32, w: f32, icon: &str, text: &str, c: Rgba) -> anyhow::Result<f32> {
  let text = cx.tr(text);
  let h = cx.p.measure_wrapped(&text, SUB, w - 40.0, 200.0, false)?.max(18.0);
  cx.p.icon(icon, x + 6.0 + 9.0, y + 10.0 + 9.0, 18.0, false, c)?;
  cx.p.text_wrapped(&text, Rect::new(x + 6.0 + 26.0, y + 10.0 + (18.0 - h.min(18.0)) / 2.0, w - 40.0, h + 2.0), SUB, c, false)?;
  Ok(y + 10.0 + h)
}

// ---------------------------------------------------------------- Görünüm

fn look(cx: &mut Ctx, s: &Settings, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let t = cx.t;
  let light = is_light(t);
  let tr = |cx: &Ctx, k: &str| cx.tr(k);

  let mut y = cx.sec_title(x, y, "Tema")?;
  let rows = [Row::new(if light { "light_mode" } else { "dark_mode" }, "Kabuk teması".into())
    .sub("Yalnızca Logical Lunge'u değiştirir, Windows temasına dokunmaz".into())
    .ctrl(Ctrl::Seg(Key::Theme, vec![(tr(cx, "Koyu"), Some("dark_mode")), (tr(cx, "Aydınlık"), Some("light_mode"))], light as usize))];
  y += cx.card(x, y, w, &rows, 0.0)?;

  // accent: the row, the swatches, the hex field
  y = cx.sec_title(x, y, "Vurgu rengi")?;
  let focus = s.s["focusColor"].as_str().unwrap_or("#b69df8").to_lowercase();
  let rows = [Row::new("palette", "Kabuk ve pencere kenarlıkları".into()).sub("Bar, paneller ve etkin pencerenin rengi birlikte değişir".into())];
  let extra = 4.0 + 44.0 + 16.0 + 36.0 + 16.0 + if s.hex_bad || s.color_error { 22.0 } else { 0.0 };
  let ch = cx.card(x, y, w, &rows, extra)?;
  let mut sy = y + ch - extra + 4.0;
  let mut sx = x + 18.0;
  for (i, (_, c)) in COLORS.iter().enumerate() {
    let r = Rect::new(sx, sy, 44.0, 44.0);
    let hit = Hit::Swatch(i);
    let color = Rgba::hex(u32::from_str_radix(&c[1..], 16).unwrap_or(0));
    let sel = focus == *c;
    let grow = if cx.hot(hit) && !sel { 1.3 } else { 0.0 };
    let rr = Rect::new(r.x - grow, r.y - grow, r.w + 2.0 * grow, r.h + 2.0 * grow);
    cx.p.fill_round(rr, if sel { 14.0 } else { 22.0 + grow }, color)?;
    if sel {
      cx.p.icon("check", r.x + 22.0, r.y + 22.0, 22.0, false, Rgba::hex(0x1d1b20))?;
    }
    cx.push(r, hit, true, Cursor::Hand);
    cx.ring(r, if sel { 14.0 } else { 22.0 }, hit)?;
    sx += 44.0 + 10.0;
  }
  sy += 44.0 + 16.0;
  // .hexrow
  let current = Rgba::hex(u32::from_str_radix(focus.trim_start_matches('#'), 16).unwrap_or(0xb69df8));
  cx.p.fill_circle(x + 18.0 + 12.0, sy + 18.0, 12.0, current)?;
  let custom = !COLORS.iter().any(|c| c.1 == focus);
  let placeholder = if custom { focus.clone() } else { "#rrggbb".into() };
  cx.field(Rect::new(x + 18.0 + 24.0 + 10.0, sy, 130.0, 36.0), &s.hex, &placeholder, Fld::Hex)?;
  let label = cx.tr("Özel rengi uygula");
  let bw = cx.ctrl_width(&Ctrl::Button { label: label.clone(), icon: Some("colorize"), kind: Btn::Tonal, hit: Hit::Act(Act::HexApply), disabled: false })?;
  let bx = x + 18.0 + 24.0 + 10.0 + 130.0 + 10.0;
  cx.button(Rect::new(bx, sy, bw, 36.0), &label, Some("colorize"), Btn::Tonal, Hit::Act(Act::HexApply), s.color_saving)?;
  sy += 36.0 + 8.0;
  if s.hex_bad {
    let e = cx.tr("Renk kodu #rrggbb biçiminde olmalı");
    cx.p.text(&e, Rect::new(x + 18.0, sy, w - 36.0, 18.0), SUB, t.error, Align::Left, false)?;
  } else if s.color_error {
    let e = cx.tr("Renk kaydedilemedi. Yeniden dene.");
    cx.p.text(&e, Rect::new(x + 18.0, sy, w - 36.0, 18.0), SUB, t.on_surface_variant, Align::Left, false)?;
  }
  y += ch;

  // the interface scale: every native window at the monitor's DPI times this
  y = cx.sec_title(x, y, "Boyut")?;
  let steps = crate::native_bar::scale::STEPS;
  let current = crate::native_bar::scale::from_pref(s.s["uiScale"].as_u64());
  let chosen = steps.iter().position(|p| *p == current).unwrap_or(2);
  let rows = [Row::new("format_size", "Arayüz ölçeği".into())
    .sub("Bar, paneller, menüler, bildirimler, yazılar ve widget'lar birlikte büyür ya da küçülür".into())
    .ctrl(Ctrl::Seg(Key::UiScale, steps.iter().map(|p| (format!("%{p}"), None)).collect(), chosen))];
  y += cx.card(x, y, w, &rows, 0.0)?;

  y = cx.sec_title(x, y, "Hareket")?;
  let mut rows = vec![Row::new("animation", "Animasyonlar".into())
    .sub("Workspace kaymaları, pencere açma / kapama / taşıma ve menü geçişleri".into())
    .ctrl(Ctrl::Switch(s.s["animations"].as_bool() != Some(false), Hit::Switch(Sw::Animations)))];
  // only on computers with a touchpad; the core applies it at once
  if s.s["touchpad"].as_bool() == Some(true) {
    rows.push(
      Row::new("swipe", "Dokunmatik yüzey hareketleri".into())
        .sub("3 parmak: yana workspace kaydırır, yukarı overview, aşağı sağ panel; 4 parmak: pencereyi taşır".into())
        .ctrl(Ctrl::Switch(s.s["gestures"].as_bool() != Some(false), Hit::Switch(Sw::Gestures))),
    );
  }
  y += cx.card(x, y, w, &rows, 0.0)?;

  y = cx.sec_title(x, y, "Bildirimler")?;
  let toast_row = |cx: &Ctx, icon: &'static str, label: &str, sl: Sl| {
    let v = slider_value(s, sl);
    Row::new(icon, label.into()).sub("Ekranda kalma süresi; üzerine gelince bekler".into()).ctrl(Ctrl::Slider {
      value: v,
      min: 1,
      max: 30,
      label: cx.tr(&format!("{v} sn")),
      hit: Hit::Slider(sl),
    })
  };
  let rows = [
    toast_row(cx, "notifications", "Bilgi bildirimleri", Sl::ToastInfo),
    toast_row(cx, "warning", "Uyarılar ve hatalar", Sl::ToastError),
    Row::new("notifications_active", "Windows bildirimleri".into())
      .sub("Uygulamaların bildirimleri Logical Lunge kartı olarak çıkar; Windows'un balonları kapanır, Bildirim Merkezi'nde kalırlar".into())
      .ctrl(Ctrl::Switch(s.s["winToasts"].as_bool() != Some(false), Hit::Switch(Sw::WinToasts))),
    Row::new("visibility", "Örnek göster".into()).ctrl(Ctrl::Icon("chevron_right")).click(Hit::Act(Act::SampleToasts)),
  ];
  y += cx.card(x, y, w, &rows, 0.0)?;
  Ok(y)
}

// ---------------------------------------------------------------- Dil ve saat

fn lang(cx: &mut Ctx, s: &Settings, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let code = s.s["language"].as_str().unwrap_or("system");
  let name = LANGS.iter().find(|l| l.0 == code).map_or("Sistem dili", |l| l.1);
  let name = cx.tr(name);
  let mut y = cx.sec_title(x, y, "Dil")?;
  let rows = [Row::new("translate", "Arayüz dili".into())
    .sub("Sistem dili seçiliyse Windows'un dili kullanılır".into())
    .ctrl(Ctrl::Select(name, Hit::Select(Sel::Lang), 200.0))];
  y += cx.card(x, y, w, &rows, 0.0)?;

  y = cx.sec_title(x, y, "Saat")?;
  let t24 = crate::native_bar::model::format_time("HH:mm");
  let t12 = crate::native_bar::model::format_time("h:mm tt");
  let clock12 = s.s["clock"].as_str() == Some("12");
  let rows = [Row::new("schedule", "Saat biçimi".into())
    .sub("Bar, bildirimler ve pano geçmişi".into())
    .ctrl(Ctrl::Seg(Key::Clock, vec![(cx.tr(&format!("24 saat · {t24}")), None), (cx.tr(&format!("12 saat · {t12}")), None)], clock12 as usize))];
  y += cx.card(x, y, w, &rows, 0.0)?;
  Ok(y)
}

// ---------------------------------------------------------------- Kısayollar

fn keys(cx: &mut Ctx, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let mut y = cx.sec_title(x, y, "Logical Lunge kısayolları")?;
  let rows = [Row::new("keyboard", "Kısayolları düzenle".into())
    .sub("Super, workspace geçişleri, pencere taşıma, uygulamalar: sağ paneldeki düzenleyicide".into())
    .ctrl(Ctrl::Icon("chevron_right"))
    .click(Hit::Act(Act::EditKeys))];
  y += cx.card(x, y, w, &rows, 0.0)?;
  y = cx.sec_title(x, y, "Pencere yöneticisi")?;
  let rows = [Row::new("data_object", "Pencere yöneticisi kısayolları".into())
    .sub("config.yaml dosyasının keybindings bölümünde; kaydedince hemen uygulanır".into())
    .ctrl(Ctrl::Icon("open_in_new"))
    .click(Hit::Act(Act::EditConfig))];
  y += cx.card(x, y, w, &rows, 0.0)?;
  Ok(y)
}

// ---------------------------------------------------------------- Gece ışığı

fn night(cx: &mut Ctx, s: &Settings, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let Some(n) = &s.night else { return Ok(y) };
  let mode = n["mode"].as_str().unwrap_or("manual");
  let mode_i = match mode {
    "after" => 1,
    "range" => 2,
    _ => 0,
  };
  let level = s.night_level;
  let kelvin = ((6500.0 - 46.0 * level as f32) / 100.0).round() as i32 * 100;
  let mut y = cx.sec_title(x, y, "Gece ışığı")?;
  let mut rows = vec![
    Row::new("nightlight", "Gece ışığı".into())
      .sub("Ekranın mavi ışığını azaltır (tüm monitörler)".into())
      .ctrl(Ctrl::Switch(n["on"].as_bool() == Some(true), Hit::Switch(Sw::Night))),
    Row::new("schedule", "Zamanlama".into()).ctrl(Ctrl::Seg(
      Key::NightMode,
      vec![(cx.tr("Her zaman"), Some("wb_sunny")), (cx.tr("Saatten sonra"), Some("bedtime")), (cx.tr("Saat aralığı"), Some("schedule"))],
      mode_i,
    )),
  ];
  if mode_i != 0 {
    let mut row = Row::new("more_time", "Saatler".into());
    if mode_i == 1 {
      row = row.sub("Sabah 07:00'de kapanır".into());
    }
    row = row.ctrl(Ctrl::Field { text: s.night_from.clone(), placeholder: "20:00".into(), w: 96.0, fld: Fld::NightFrom });
    if mode_i == 2 {
      row = row.ctrl(Ctrl::Field { text: s.night_to.clone(), placeholder: "07:00".into(), w: 96.0, fld: Fld::NightTo });
    }
    rows.push(row);
  }
  rows.push(Row::new("tonality", "Yoğunluk".into()).ctrl(Ctrl::Slider {
    value: level,
    min: 0,
    max: 100,
    label: format!("%{level} · {kelvin}K"),
    hit: Hit::Slider(Sl::NightLevel),
  }));
  y += cx.card(x, y, w, &rows, 0.0)?;
  Ok(y)
}

// ---------------------------------------------------------------- Sistem sağlığı

fn uptime(sec: Option<i64>) -> String {
  let Some(sec) = sec else { return "—".into() };
  let (d, h, m) = (sec / 86400, sec % 86400 / 3600, sec % 3600 / 60);
  if d > 0 {
    format!("{d}d {h}h")
  } else if h > 0 {
    format!("{h}h {m}m")
  } else {
    format!("{m}m")
  }
}

fn health(cx: &mut Ctx, s: &Settings, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let Some(h) = &s.health else { return Ok(y) };
  let t = cx.t;
  let mut y = cx.sec_title(x, y, "Parçalar")?;
  // .parts: three cards in a row
  let parts: Vec<&Value> = h["parts"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
  let pw = (w - 20.0) / 3.0;
  let ph = 14.0 + 22.0 + 6.0 + 18.0 + 6.0 + 18.0 + 6.0 + 18.0 + 14.0;
  for (i, part) in parts.iter().enumerate() {
    let px = x + (i % 3) as f32 * (pw + 10.0);
    let py = y + (i / 3) as f32 * (ph + 10.0);
    let r = Rect::new(px, py, pw, ph);
    cx.p.fill_round(r, 20.0, t.layer1)?;
    let key = part["key"].as_str().unwrap_or("");
    let (name, icon) = PART_NAMES.iter().find(|p| p.0 == key).map_or((key, "widgets"), |p| (p.1, p.2));
    cx.p.icon(icon, r.x + 16.0 + 11.0, r.y + 14.0 + 11.0, 22.0, false, t.primary)?;
    let name = cx.tr(name);
    cx.p.text(&name, Rect::new(r.x + 16.0 + 30.0, r.y + 14.0, pw - 62.0, 22.0), TextStyle { size: 14.0, weight: 560.0 }, t.on_layer1, Align::Left, false)?;
    let running = part["running"].as_bool() == Some(true);
    let state = cx.tr(if running { "Çalışıyor" } else { "Çalışmıyor" });
    cx.state(r.x + 16.0, r.y + 14.0 + 22.0 + 6.0 + 9.0, running, &state)?;
    if running {
      let mut sy = r.y + 14.0 + 22.0 + 6.0 + 18.0 + 6.0;
      let mem = part["memMB"].as_i64().map_or("—".to_string(), |m| m.to_string());
      for (label, value) in [("Açık süre", uptime(part["uptime"].as_i64())), ("Bellek", format!("{mem} MB"))] {
        let label = cx.tr(label);
        cx.p.text(&label, Rect::new(r.x + 16.0, sy, pw - 32.0, 18.0), SUB, t.on_surface_variant, Align::Left, false)?;
        let vw = cx.p.measure(&value, SUB)?;
        cx.p.text(&value, Rect::new(r.right() - 16.0 - vw - 1.0, sy, vw + 2.0, 18.0), SUB, t.on_surface_variant, Align::Left, true)?;
        sy += 18.0 + 6.0;
      }
    }
  }
  if !parts.is_empty() {
    y += ((parts.len() + 2) / 3) as f32 * (ph + 10.0) - 10.0;
  }

  y = cx.sec_title(x, y, "Durum")?;
  let elevated = h["elevated"].as_bool() == Some(true);
  let black = h["blackBox"].as_str().map(|b| match b.find(" KARA KUTU: ") {
    Some(i) => format!("{} · {}", &b[..i], &b[i + " KARA KUTU: ".len()..]),
    None => b.to_string(),
  });
  let rows = [
    Row::new("admin_panel_settings", "Yönetici haklarıyla".into())
      .sub("Kısayollar ve pencere yönetimi Görev Yöneticisi gibi yönetici pencerelerinde de çalışır".into())
      .ctrl(Ctrl::State(elevated, cx.tr(if elevated { "Evet" } else { "Hayır" }))),
    Row::new("flight", "Kara kutu".into()).sub(black.unwrap_or_else(|| "Yavaşlama kaydı yok".into())),
  ];
  y += cx.card(x, y, w, &rows, 0.0)?;

  let events: Vec<String> = h["events"].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
  if !events.is_empty() {
    y = cx.sec_title(x, y, "Son olaylar")?;
    let log = events.join("\n");
    let style = TextStyle { size: 12.0, weight: 400.0 };
    let lh = cx.p.measure_wrapped(&log, style, w - 36.0, 4000.0, true)?;
    let r = Rect::new(x, y, w, lh + 24.0);
    cx.p.fill_round(r, 20.0, t.layer1)?;
    cx.p.text_wrapped(&log, Rect::new(x + 18.0, y + 12.0, w - 36.0, lh + 2.0), style, t.on_surface_variant, true)?;
    y += r.h;
  }

  y = cx.sec_title(x, y, "İşlemler")?;
  let btns = [
    ("Masaüstünü yenile", "restart_alt", Btn::Primary, Act::RestartDesktop),
    ("Kara kutu kaydı al", "flight", Btn::Tonal, Act::BlackBox),
    ("Log klasörünü aç", "folder_open", Btn::Outline, Act::OpenLogs),
  ];
  let mut bx = x;
  for (label, icon, kind, act) in btns {
    let label = cx.tr(label);
    let bw = cx.ctrl_width(&Ctrl::Button { label: label.clone(), icon: Some(icon), kind, hit: Hit::Act(act), disabled: false })?;
    if bx + bw > x + w {
      bx = x;
      y += 44.0;
    }
    cx.button(Rect::new(bx, y, bw, 36.0), &label, Some(icon), kind, Hit::Act(act), false)?;
    bx += bw + 8.0;
  }
  Ok(y + 36.0)
}

// ---------------------------------------------------------------- Gelişmiş

fn advanced(cx: &mut Ctx, s: &Settings, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let mut y = cx.sec_title(x, y, "Ayar dosyaları")?;
  let rows = [
    Row::new("folder_open", "Ayar klasörünü aç".into())
      .sub(s.s["configDir"].as_str().unwrap_or_default().to_string())
      .ctrl(Ctrl::Icon("open_in_new"))
      .click(Hit::Act(Act::OpenConfigDir)),
    Row::new("edit_document", "config.yaml'ı düzenle".into())
      .sub("Pencere yöneticisi, kenarlıklar, kurallar ve kısayollar".into())
      .ctrl(Ctrl::Icon("open_in_new"))
      .click(Hit::Act(Act::EditConfig)),
  ];
  y += cx.card(x, y, w, &rows, 0.0)?;
  y = cx.sec_title(x, y, "Pencere yöneticisi")?;
  let rows = [
    Row::new("sync", "Pencere yöneticisi ayarlarını yenile".into())
      .sub("config.yaml'daki değişiklikleri yeniden okur".into())
      .ctrl(Ctrl::Icon("chevron_right"))
      .click(Hit::Act(Act::WmReload)),
    Row::new("refresh", "Pencereleri yeniden çiz".into())
      .sub("Yerleşim bozulduysa pencereleri yeniden yerleştirir".into())
      .ctrl(Ctrl::Icon("chevron_right"))
      .click(Hit::Act(Act::WmRedraw)),
  ];
  y += cx.card(x, y, w, &rows, 0.0)?;
  y = cx.sec_title(x, y, "Windows")?;
  let rows = [Row::new("desktop_windows", "Windows'un yerini al".into())
    .sub("Logical Lunge açıkken Windows'un görev çubuğu, yerleşim önerileri ve benzeri parçaları kapanır; kapanınca eski hâline döner".into())
    .ctrl(Ctrl::Switch(s.s["takeover"].as_bool() != Some(false), Hit::Switch(Sw::Takeover)))];
  y += cx.card(x, y, w, &rows, 0.0)?;
  Ok(y)
}

// ---------------------------------------------------------------- Hakkında

fn about(cx: &mut Ctx, s: &Settings, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let t = cx.t;
  let mut y = y + 8.0;
  // .about: the logo, the name, the version
  let r = Rect::new(x, y, w, 64.0 + 44.0);
  cx.p.fill_round(r, 20.0, t.layer1)?;
  let logo = Rect::new(x + 18.0, y + 22.0, 64.0, 64.0);
  cx.p.fill_round(logo, 18.0, Rgba::hex(0x1d1b20))?;
  for (lx, ly, lw, lh, c) in [(15.0, 13.0, 11.0, 38.0, 0xd0bcff), (15.0, 40.0, 34.0, 11.0, 0xd0bcff), (31.0, 13.0, 18.0, 22.0, 0x7f67be)] {
    cx.p.fill_round(Rect::new(logo.x + lx, logo.y + ly, lw, lh), 5.0, Rgba::hex(c))?;
  }
  let title_style = TextStyle { size: 22.0, weight: 560.0 };
  let title_x = logo.right() + 18.0;
  let title_w = cx.p.measure("Logical Lunge", title_style)?;
  cx.p.text("Logical Lunge", Rect::new(title_x, y + 30.0, title_w, 28.0), title_style, t.on_layer1, Align::Left, false)?;
  let edition_x = title_x + title_w + 12.0;
  cx.p.text("Native version", Rect::new(edition_x, y + 36.0, (r.right() - 18.0 - edition_x).max(0.0), 20.0), SUB, t.on_surface_variant, Align::Left, false)?;
  let version = s.s["version"].as_str().filter(|v| !v.is_empty()).unwrap_or("—");
  let v = cx.tr(&format!("Sürüm {version} · GPL-3.0"));
  cx.p.text(&v, Rect::new(logo.right() + 18.0, y + 60.0, w - 120.0, 20.0), BODY, t.on_surface_variant, Align::Left, false)?;
  y += r.h;

  y = cx.sec_title(x, y, "Güncellemeler")?;
  let rows = [
    Row::new("system_update_alt", "Güncellemeleri denetle".into())
      .sub("Yeni sürüm varsa sağ üstte bir kart çıkar".into())
      .ctrl(Ctrl::Icon("chevron_right"))
      .click(Hit::Act(Act::CheckUpdates)),
    Row::new("code", "Kaynak kodu".into())
      .sub("github.com/KaanAlper/logical-lunge".into())
      .ctrl(Ctrl::Icon("open_in_new"))
      .click(Hit::Act(Act::SourceCode)),
  ];
  y += cx.card(x, y, w, &rows, 0.0)?;
  Ok(y)
}

/// A line of text notes (errors, "saved") under a block.
pub fn status_note(cx: &mut Ctx, x: f32, y: f32, w: f32, icon: &str, text: &str, c: Rgba) -> anyhow::Result<f32> {
  note(cx, x, y, w, icon, text, c)
}
