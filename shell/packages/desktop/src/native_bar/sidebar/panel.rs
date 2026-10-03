//! Drawing of the panel and of the pages over it.

use super::*;

/// The panel: shadow and box, the top row, quick settings, notifications,
/// the bottom group, then what floats over them (a tile's card, the dragged
/// tile).
pub(super) fn paint_main(cx: &mut Cx, sb: &mut Sidebar, m: &crate::native_bar::model::Model, h: f32, animations: bool) -> anyhow::Result<()> {
  let panel = Rect::new(INSET, INSET, WIN_W - 2.0 * INSET, h - 2.0 * INSET);
  // box-shadow: 0 2px 14px rgba(0 0 0 / 45%)
  for k in 1..=7 {
    let g = k as f32 * 2.0;
    cx.p.fill_round(Rect::new(panel.x - g, panel.y + 2.0 - g, panel.w + 2.0 * g, panel.h + 2.0 * g), PANEL_R + g, crate::native_bar::gfx::Rgba(0, 0, 0, 0.06))?;
  }
  cx.round(panel, PANEL_R, cx.t.layer0)?;
  cx.p.stroke_round(panel.inset(0.5, 0.5), PANEL_R, cx.c.border0, 1.0)?;
  cx.hit(panel, Hit::Panel);
  let x = panel.x + 10.0;
  let w = panel.w - 20.0;
  let mut y = panel.y + 10.0 + 5.0;
  paint_sys(cx, x, y, w)?;
  y += 42.0 + 10.0;
  let tr = cx.tr;
  let qh = sb.quick.height(m, &|s| tr(s));
  let dash = sb.dash.clone();
  sb.quick.paint(cx, m, Rect::new(x, y, w, qh), animations, dash.as_ref())?;
  y += qh + 10.0;
  let (bh, folding) = sb.bottom.height(sb.store.collapsed, cx.now, animations);
  if folding {
    cx.busy = true;
  }
  let bottom = Rect::new(x, panel.bottom() - 10.0 - bh, w, bh);
  let center = Rect::new(x, y, w, (bottom.y - 10.0 - y).max(60.0));
  let day = |ms: i64| {
    let (yy, mm, dd) = crate::native_bar::model::local_day(Some(ms));
    m.format_day(yy, mm, dd, "d MMMM")
  };
  let dismissed = sb.store.notif_dismissed.clone();
  sb.notifs.paint(cx, center, &dismissed, m.dnd, &mut sb.images, &mut sb.scroll, &day, animations)?;
  let mut todo = sb.fields.remove(&FieldId::Todo).unwrap_or_else(|| TextField::new(false));
  let store = sb.store.clone();
  sb.bottom.paint(cx, m, bottom, &store, &mut todo, &mut sb.scroll, animations)?;
  sb.fields.insert(FieldId::Todo, todo);
  // floating over the rest
  let mut pw = sb.fields.remove(&FieldId::WifiPw).unwrap_or_else(|| {
    let mut f = TextField::new(false);
    f.password = true;
    f
  });
  let mut from = sb.fields.remove(&FieldId::NightFrom).unwrap_or_else(|| TextField::new(false));
  let mut to = sb.fields.remove(&FieldId::NightTo).unwrap_or_else(|| TextField::new(false));
  sb.quick.paint_card(cx, m, &mut sb.scroll, &mut pw, &mut from, &mut to, animations)?;
  sb.fields.insert(FieldId::WifiPw, pw);
  sb.fields.insert(FieldId::NightFrom, from);
  sb.fields.insert(FieldId::NightTo, to);
  Ok(())
}

/// The top row: the uptime pill and the round buttons.
pub(super) fn paint_sys(cx: &mut Cx, x: f32, y: f32, w: f32) -> anyhow::Result<()> {
  let buttons: [(Sys, &str, &str); 6] = [
    (Sys::Bug, "bug_report", "Hata Bildir (Bug Report)"),
    (Sys::Update, "system_update_alt", "Güncellemeleri denetle"),
    (Sys::Walls, "wallpaper", "Duvar kağıtları"),
    (Sys::Keys, "keyboard", "Kısayollar"),
    (Sys::Settings, "settings", "Ayarlar"),
    (Sys::Session, "power_settings_new", "Oturum"),
  ];
  let bw = 42.0;
  let group_w = buttons.len() as f32 * bw + (buttons.len() as f32 - 1.0) * 8.0;
  let mut bx = x + w - group_w;
  for (s, icon, _tip) in buttons {
    let r = Rect::new(bx, y, bw, bw);
    let hit = Hit::Sys(s);
    let hot = cx.hot(&hit);
    let (bg, fg) = match (s, hot) {
      (Sys::Bug, true) => (cx.t.primary_container, cx.t.on_primary_container),
      (Sys::Bug, false) => (cx.t.layer1, cx.t.primary),
      (Sys::Session, true) => (cx.t.error_container, cx.t.on_error_container),
      (_, true) => (cx.t.layer1_hover, cx.t.on_layer1),
      _ => (cx.t.layer1, cx.t.on_layer1),
    };
    let pressed = cx.down(&hit);
    let rr = if pressed { Rect::new(r.x + 1.3, r.y + 1.3, r.w - 2.6, r.h - 2.6) } else { r };
    cx.round(rr, if hot { 15.0 } else { 21.0 }, bg)?;
    cx.icon(icon, r.x + 21.0, r.y + 21.0, 22.0, true, fg)?;
    cx.hit(r, hit);
    bx += bw + 8.0;
  }
  // the uptime: as wide as its text
  let value = uptime(sysinfo::System::uptime() * 1000);
  let label = cx.tr("Çalışma süresi");
  let tw = cx.measure(&label, st(11.0))?.max(cx.measure(&value, stw(16.0, 560.0))?).ceil();
  let pw = (12.0 + 22.0 + 10.0 + tw + 18.0).min(x + w - group_w - 8.0 - x);
  let pill = Rect::new(x, y, pw, 42.0);
  cx.round(pill, 21.0, cx.t.layer1)?;
  cx.windows_logo(pill.x + 12.0, pill.y + 10.0, 22.0, cx.t.on_layer1.alpha(0.9))?;
  let tx = pill.x + 12.0 + 22.0 + 10.0;
  cx.text(&label, Rect::new(tx, pill.y + 5.0, pill.right() - tx - 8.0, 14.0), st(11.0), cx.t.subtext)?;
  cx.text(&value, Rect::new(tx, pill.y + 18.0, pill.right() - tx - 8.0, 20.0), stw(16.0, 560.0), cx.t.on_layer1)?;
  Ok(())
}

/// A page over the panel: its box, the head (back, icon, title, actions)
/// and its body.
pub(super) fn paint_page(cx: &mut Cx, sb: &mut Sidebar, m: &crate::native_bar::model::Model, page: Page, h: f32) -> anyhow::Result<()> {
  let panel = Rect::new(INSET, INSET, WIN_W - 2.0 * INSET, h - 2.0 * INSET);
  if page == Page::Bug {
    return bug::paint(cx, sb, m, panel);
  }
  cx.hit(panel, Hit::Panel);
  cx.round(panel, PANEL_R, cx.t.layer0)?;
  cx.push_clip(panel);
  let (icon, title) = match page {
    Page::Keys => ("keyboard", "Kısayollar"),
    _ => ("wallpaper", "Duvar kağıtları"),
  };
  let head_y = panel.y + 10.0;
  cx.round_btn(Rect::new(panel.x + 10.0, head_y, 40.0, 40.0), "arrow_back", 22.0, false, None, cx.t.on_layer1, Hit::Back)?;
  cx.icon(icon, panel.x + 10.0 + 40.0 + 8.0 + 10.0, head_y + 20.0, 20.0, false, cx.t.primary)?;
  let tx = panel.x + 10.0 + 40.0 + 8.0 + 20.0 + 8.0;
  cx.text(&cx.tr(title), Rect::new(tx, head_y, 200.0, 40.0), stw(18.0, 550.0), cx.t.on_layer1)?;
  let actions_right = panel.right() - 10.0;
  if page == Page::Keys {
    keys::paint_actions(cx, &sb.keys, actions_right, head_y + 4.0)?;
  }
  // the shortcut editor keeps its save bar under the scrolling body
  let footer = if page == Page::Keys { sb.keys.footer_h() } else { 0.0 };
  let body = Rect::new(panel.x, head_y + 40.0 + 6.0, panel.w, panel.bottom() - footer - (head_y + 46.0));
  let off = sb.scroll.get(&ScrollId::Page).copied().unwrap_or(0.0);
  cx.push_clip(body);
  let content = match page {
    Page::Keys => {
      let mut f = sb.fields.remove(&FieldId::KeysSearch).unwrap_or_else(|| TextField::new(false));
      let r = keys::paint(cx, &mut sb.keys, &mut f, Rect::new(body.x + 10.0, body.y + 4.0 - off, body.w - 20.0, 0.0));
      sb.fields.insert(FieldId::KeysSearch, f);
      r?
    }
    _ => {
      let mut f = sb.fields.remove(&FieldId::SaverMinutes).unwrap_or_else(|| TextField::new(false));
      let r = walls::paint(cx, sb, m, &mut f, Rect::new(body.x + 10.0, body.y + 4.0, body.w - 20.0, body.h - 4.0), off);
      sb.fields.insert(FieldId::SaverMinutes, f);
      r?
    }
  };
  cx.pop_clip();
  let max = (content + 16.0 - body.h).max(0.0);
  sb.scroll.insert(ScrollId::Page, off.clamp(0.0, max));
  cx.region(body, ScrollId::Page, content + 16.0, false);
  if page == Page::Keys {
    keys::paint_footer(cx, &sb.keys, Rect::new(panel.x, body.bottom(), panel.w, footer))?;
    let mut f = sb.fields.remove(&FieldId::KeysApp).unwrap_or_else(|| TextField::new(false));
    let off = sb.scroll.get(&ScrollId::KeysApps).copied().unwrap_or(0.0);
    let r = keys::paint_overlay(cx, &mut sb.keys, &mut f, off, panel);
    sb.fields.insert(FieldId::KeysApp, f);
    let max = r?;
    sb.scroll.insert(ScrollId::KeysApps, off.clamp(0.0, max));
  }
  cx.pop_clip();
  Ok(())
}

