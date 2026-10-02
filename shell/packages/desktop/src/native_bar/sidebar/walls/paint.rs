//! Drawing of the wallpaper page: the tabs, the galleries, the settings
//! of each tab, the removal confirmation.

use super::*;

/// Draws the page body in `r` (scrolled by `off`); returns its height.
pub(in crate::native_bar::sidebar) fn paint(cx: &mut Cx, sb: &mut Sidebar, m: &Model, minutes: &mut TextField, r: Rect, off: f32) -> anyhow::Result<f32> {
  let w = &mut sb.walls;
  // the tabs (`.nl-seg`): fixed above the gallery
  let tab_w = (r.w - 3.0 * 4.0) / 4.0;
  for (i, (label, icon)) in TABS.iter().enumerate() {
    let b = Rect::new(r.x + i as f32 * (tab_w + 4.0), r.y, tab_w, 40.0);
    seg(cx, b, &cx.tr(label), icon, w.tab == i, Hit::Walls(WHit::Tab(i)), i == 0, i == 3)?;
  }
  let top = r.y + 40.0 + 10.0;
  let area = Rect::new(r.x - 10.0, top, r.w + 20.0, r.bottom() - top);
  cx.push_clip(area);
  let mut y = top - off;
  let x = r.x;
  let width = r.w;
  if let Some((ok, text)) = w.msg.clone() {
    y += cx.message(x, y, width, ok, &text)? + 10.0;
  }
  match w.tab {
    TAB_WALL => y = paint_walls(cx, sb, x, y, width)?,
    TAB_LIVE => y = paint_live(cx, sb, x, y, width)?,
    TAB_SAVER => y = paint_saver(cx, sb, m, minutes, x, y, width)?,
    _ => y = paint_store(cx, sb, x, y, width)?,
  }
  cx.pop_clip();
  Ok(y + off - r.y)
}

/// A segmented button (`.nl-seg button`): ends rounded, the selected one a pill.
#[allow(clippy::too_many_arguments)]
fn seg(cx: &mut Cx, b: Rect, label: &str, icon: &str, sel: bool, hit: Hit, first: bool, last: bool) -> anyhow::Result<()> {
  let (bg, fg) = if sel {
    (cx.t.sec_container, cx.t.on_sec_container)
  } else if cx.hot(&hit) {
    (cx.t.layer1_hover, cx.t.on_layer1)
  } else {
    (cx.t.layer1, cx.t.on_layer1)
  };
  let radius = if sel || first || last { 20.0 } else { 8.0 };
  cx.round(b, radius, bg)?;
  let lw = cx.measure(label, st(12.5))?.min(b.w - 8.0);
  let total = 17.0 + 4.0 + lw;
  if total <= b.w - 8.0 {
    let x0 = b.x + (b.w - total) / 2.0;
    cx.icon(icon, x0 + 8.5, b.y + b.h / 2.0, 17.0, sel, fg)?;
    cx.text(label, Rect::new(x0 + 21.0, b.y, lw + 2.0, b.h), st(12.5), fg)?;
  } else {
    // narrow: the icon over a small label
    cx.icon(icon, b.x + b.w / 2.0, b.y + 13.0, 17.0, sel, fg)?;
    cx.text_center(label, Rect::new(b.x + 2.0, b.y + 21.0, b.w - 4.0, 16.0), st(10.5), fg)?;
  }
  cx.hit(b, hit);
  Ok(())
}

/// Chips that wrap (`.wall-cats`); returns the height they took.
fn chips_wrap(cx: &mut Cx, x: f32, y: f32, w: f32, chips: &[(String, Option<&str>, bool, Hit)]) -> anyhow::Result<f32> {
  let (mut cx_, mut cy) = (x, y);
  for (label, icon, on, hit) in chips {
    let cw = cx.chip_w(label, *icon)?;
    if cx_ + cw > x + w && cx_ > x {
      cx_ = x;
      cy += 32.0 + 6.0;
    }
    cx.chip(cx_, cy, 32.0, label, *icon, *on, true, hit.clone())?;
    cx_ += cw + 6.0;
  }
  Ok(cy + 32.0 - y)
}

/// A row of chips that scrolls sideways (every category row of the page):
/// the wheel (either way) and a drag move it, arrows and faded edges show
/// what is hidden, and the selected chip is brought into view when it
/// changes. Returns its height.
fn chip_row(cx: &mut Cx, sb: &mut Sidebar, row: u8, x: f32, y: f32, w: f32, chips: &[(String, Option<&str>, bool, Hit)]) -> anyhow::Result<f32> {
  const H: f32 = 32.0;
  const GAP: f32 = 6.0;
  const ARROW: f32 = 28.0;
  const FADE: f32 = 36.0;
  let id = ScrollId::Row(row);
  // the chips' places first: the offset can then keep the selected in view
  let mut spots = Vec::with_capacity(chips.len());
  let mut at = 0.0f32;
  for (label, icon, _, _) in chips {
    let cw = cx.chip_w(label, *icon)?;
    spots.push((at, cw));
    at += cw + GAP;
  }
  let content = (at - GAP).max(0.0);
  let max = (content - w).max(0.0);
  let mut off = sb.scroll.get(&id).copied().unwrap_or(0.0).clamp(0.0, max);
  if sb.walls.reveal == Some(row) {
    sb.walls.reveal = None;
    if let Some(&(cx0, cw)) = chips.iter().position(|c| c.2).and_then(|i| spots.get(i)) {
      // clear of the arrows on both sides
      if cx0 - ARROW < off {
        off = (cx0 - ARROW).max(0.0);
      } else if cx0 + cw + ARROW > off + w {
        off = (cx0 + cw + ARROW - w).min(max);
      }
    }
  }
  sb.scroll.insert(id, off);
  let band = Rect::new(x, y - 2.0, w, H + 4.0);
  cx.push_clip(band);
  for ((label, icon, on, hit), &(cx0, cw)) in chips.iter().zip(&spots) {
    let cxx = x + cx0 - off;
    if cxx > x + w || cxx + cw < x {
      continue;
    }
    cx.chip(cxx, y, H, label, *icon, *on, true, hit.clone())?;
  }
  // faded edges and arrows where chips are hidden
  let bg = cx.t.layer0;
  let clear = Rgba(bg.0, bg.1, bg.2, 0.0);
  for (left, hidden) in [(true, off > 0.5), (false, off < max - 0.5)] {
    if !hidden {
      continue;
    }
    let fade = if left { Rect::new(x, band.y, FADE, band.h) } else { Rect::new(x + w - FADE, band.y, FADE, band.h) };
    let stops = if left { [(0.0, bg), (0.55, bg), (1.0, clear)] } else { [(0.0, clear), (0.45, bg), (1.0, bg)] };
    cx.gradient(fade, 0.0, &stops)?;
    let hit = Hit::Walls(WHit::RowStep(row, if left { -1 } else { 1 }));
    let b = Rect::new(if left { x } else { x + w - ARROW }, y + (H - ARROW) / 2.0, ARROW, ARROW);
    let fill = if cx.hot(&hit) { cx.t.layer1_hover } else { cx.t.layer1 };
    cx.p.fill_circle(b.x + ARROW / 2.0, b.y + ARROW / 2.0, ARROW / 2.0, fill)?;
    cx.icon(if left { "chevron_left" } else { "chevron_right" }, b.x + ARROW / 2.0, b.y + ARROW / 2.0, 20.0, false, cx.t.on_layer1)?;
    cx.hit(b, hit);
  }
  cx.pop_clip();
  cx.region(Rect::new(x, y, w, H), id, content, true);
  Ok(H)
}

/// Monitors as on Windows' display settings, and the target chips.
fn paint_monitors(cx: &mut Cx, walls: &Walls, x: f32, y: f32, w: f32) -> anyhow::Result<f32> {
  let mons = walls.monitors();
  if mons.len() < 2 {
    return Ok(0.0);
  }
  let num = |m: &Value, k: &str| m[k].as_f64().unwrap_or(0.0) as f32;
  let min_x = mons.iter().map(|m| num(m, "x")).fold(0.0f32, f32::min);
  let min_y = mons.iter().map(|m| num(m, "y")).fold(0.0f32, f32::min);
  let max_x = mons.iter().map(|m| num(m, "x") + num(m, "w")).fold(1.0f32, f32::max);
  let max_y = mons.iter().map(|m| num(m, "y") + num(m, "h")).fold(1.0f32, f32::max);
  let sc = (360.0 / (max_x - min_x)).min(90.0 / (max_y - min_y));
  let map_w = (max_x - min_x) * sc;
  let ox = x + (w - map_w) / 2.0;
  for (i, m) in mons.iter().enumerate() {
    let id = s(&m["id"]).to_string();
    let sel = walls.target == "all" || walls.target == id;
    let r = Rect::new(ox + (num(m, "x") - min_x) * sc, y + (num(m, "y") - min_y) * sc, num(m, "w") * sc - 4.0, num(m, "h") * sc - 4.0);
    let (bg, border, fg) = if sel { (cx.t.primary_container, cx.t.primary, cx.t.on_primary_container) } else { (cx.c.layer2, cx.c.outline, cx.t.on_surface_variant) };
    cx.round(r, 6.0, bg)?;
    cx.p.stroke_round(r.inset(1.0, 1.0), 5.0, border, 2.0)?;
    cx.text_center(&(i + 1).to_string(), r, st(13.0), fg)?;
    cx.hit(r, Hit::Walls(WHit::Mon(id)));
  }
  let mut h = (max_y - min_y) * sc + 8.0 + 8.0;
  let mut chips = vec![(cx.tr("Tümü"), Some("select_all"), walls.target == "all", Hit::Walls(WHit::Target("all".into())))];
  for (i, m) in mons.iter().enumerate() {
    let id = s(&m["id"]).to_string();
    chips.push((cx.tr(&format!("Monitör {}", i + 1)), None, walls.target == id, Hit::Walls(WHit::Target(id))));
  }
  h += chips_wrap(cx, x, y + h, w, &chips)? + 10.0;
  Ok(h)
}

/// Gallery tile size: two columns of 16:9; savers three square-ish; span one wide.
pub(super) fn tile_size(g: G, w: f32) -> (f32, f32, usize) {
  match g {
    G::Span => (w, 74.0, 1),
    G::Savers => ((w - 2.0 * 8.0) / 3.0, 104.0, 3),
    _ => {
      let tw = (w - 8.0) / 2.0;
      (tw, (tw * 9.0 / 16.0).round(), 2)
    }
  }
}

/// A gallery: the add / import tile first (if any), then the items, in a
/// grid; a spinner while it loads. Returns the height.
#[allow(clippy::too_many_arguments)]
fn gallery(cx: &mut Cx, sb: &mut Sidebar, g: G, x: f32, y: f32, w: f32, custom: Option<(&str, &str)>, selected: Option<String>) -> anyhow::Result<f32> {
  let (tw, th, cols) = tile_size(g, w);
  let tiles = sb.walls.tiles(g);
  let mut cells: Vec<Option<Tile>> = Vec::new();
  if custom.is_some() {
    cells.push(None);
  }
  let loading = tiles.is_none();
  let list = tiles.unwrap_or_default();
  let empty = list.is_empty() && !loading;
  cells.extend(list.into_iter().map(Some));
  let mut i = 0;
  let rows = cells.len().div_ceil(cols) + if loading && cells.len() % cols == 0 { 1 } else { 0 };
  let row_h = th + 8.0;
  let busy = sb.walls.busy.clone();
  let progress = sb.walls.progress.clone();
  let hover = sb.walls.hover.clone();
  for cell in &cells {
    let (col, row) = ((i % cols) as f32, (i / cols) as f32);
    let r = Rect::new(x + col * (tw + 8.0), y + row * row_h, tw, th);
    match cell {
      None => {
        let (label, icon) = custom.unwrap_or(("", "add"));
        let hit = Hit::Walls(WHit::Custom(g));
        let hot = cx.hot(&hit);
        let rr = if hot { grow(r, 0.03) } else { r };
        cx.round(rr, if hot { 16.0 } else { 12.0 }, cx.t.sec_container)?;
        cx.icon(icon, rr.x + rr.w / 2.0, rr.y + rr.h / 2.0 - 10.0, 26.0, false, cx.t.on_sec_container)?;
        cx.p.text_wrapped(label, Rect::new(rr.x + 8.0, rr.y + rr.h / 2.0 + 6.0, rr.w - 16.0, 30.0), st(12.0), cx.t.on_sec_container, false)?;
        cx.hit(r, hit);
      }
      Some(_) if !cx.visible(r) => {}
      Some(t) => {
        let hit = Hit::Walls(WHit::Tile(g, t.key.clone()));
        let actions = Walls::tile_actions(g);
        let on_action = actions.iter().any(|(_, id, _)| cx.hot(&Hit::Walls(WHit::TileAct(g, t.key.clone(), id))));
        let hot = cx.hot(&hit) || on_action;
        let sel = selected.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(&t.key)) || (g == G::Videos && sb.walls.in_videos(&t.path));
        let is_busy = busy.as_deref() == Some(t.key.as_str());
        paint_tile(cx, sb, g, r, t, hot, sel, is_busy, &progress, hover.as_ref())?;
        cx.hit(r, hit);
        if hot && !is_busy {
          tile_actions(cx, g, r, &t.key, actions)?;
        }
      }
    }
    i += 1;
  }
  if loading {
    let (col, row) = ((i % cols) as f32, (i / cols) as f32);
    let r = Rect::new(x + col * (tw + 8.0), y + row * row_h, tw, th);
    cx.round(r, 12.0, cx.c.layer2)?;
    cx.spinner(r.x + r.w / 2.0, r.y + r.h / 2.0, 26.0, cx.t.on_surface_variant)?;
  }
  let mut h = rows as f32 * row_h - 8.0;
  if empty {
    let ey = y + if custom.is_some() { rows as f32 * row_h } else { 0.0 };
    cx.text_center(&cx.tr("Bir şey bulunamadı"), Rect::new(x, ey, w, 30.0), st(13.0), cx.t.on_surface_variant)?;
    h = ey + 30.0 - y;
  }
  Ok(h.max(0.0))
}

/// An item's main actions on hover: round buttons at its top right, each
/// running the right-click menu's item of the same id.
fn tile_actions(cx: &mut Cx, g: G, r: Rect, key: &str, actions: &[(&str, &'static str, &str)]) -> anyhow::Result<()> {
  const B: f32 = 30.0;
  let mut bx = r.right() - 6.0 - B;
  for (icon, id, label) in actions.iter().rev() {
    let hit = Hit::Walls(WHit::TileAct(g, key.to_string(), id));
    let hot = cx.hot(&hit);
    let c = Rect::new(bx, r.y + 6.0, B, B);
    cx.p.fill_circle(c.x + B / 2.0, c.y + B / 2.0, B / 2.0, if hot { cx.t.primary } else { Rgba(0, 0, 0, 0.6) })?;
    cx.icon(icon, c.x + B / 2.0, c.y + B / 2.0, 18.0, hot, if hot { cx.t.on_primary } else { Rgba(255, 255, 255, 1.0) })?;
    if hot {
      // what it does, under the button
      let text = cx.tr(label);
      let tw = cx.measure(&text, st(11.0))?.ceil() + 14.0;
      let tip = Rect::new((c.right() - tw).max(r.x + 4.0), c.bottom() + 4.0, tw, 20.0);
      cx.round(tip, 10.0, Rgba(0, 0, 0, 0.75))?;
      cx.text_center(&text, tip, st(11.0), Rgba(255, 255, 255, 1.0))?;
    }
    cx.hit(c, hit);
    bx -= B + 6.0;
  }
  Ok(())
}

fn grow(r: Rect, k: f32) -> Rect {
  Rect::new(r.x - r.w * k / 2.0, r.y - r.h * k / 2.0, r.w * (1.0 + k), r.h * (1.0 + k))
}

#[allow(clippy::too_many_arguments)]
fn paint_tile(cx: &mut Cx, sb: &mut Sidebar, g: G, r: Rect, t: &Tile, hot: bool, sel: bool, busy: bool, progress: &str, hover: Option<&(String, Instant)>) -> anyhow::Result<()> {
  let rr = if hot { grow(r, 0.04) } else { r };
  let radius = if hot { 16.0 } else { 12.0 };
  cx.round(rr, radius, cx.c.layer2)?;
  let saver = g == G::Savers;
  let mut drew = false;
  if !t.thumb.is_empty() {
    let (mw, mh) = if saver { (96, 96) } else { (420, 240) };
    // the store's moving preview while the pointer is on it
    let moving = hot && !t.preview.is_empty();
    if moving {
      let since = hover.filter(|(k, _)| *k == t.key).map_or(Instant::now(), |(_, at)| *at);
      sb.walls.last_gif = Some(format!("gif:{}", t.preview));
      if let Some(Img::Moving(frames)) = sb.images.get_moving(&t.preview, 360, 210) {
        if let Some(b) = frame_now(frames, since) {
          let size = unsafe { b.GetSize() };
          let img: ID2D1Image = b.cast()?;
          cx.p.image_round(&img, size.width, size.height, rr, radius, 1.0)?;
          drew = true;
          cx.busy = true;
        }
      }
    }
    if !drew {
      if let Some(Img::Still(b)) = sb.images.get(&t.thumb, mw, mh) {
        let size = unsafe { b.GetSize() };
        if saver {
          let k = (36.0 / size.width.max(1.0)).min(36.0 / size.height.max(1.0));
          let (iw, ih) = (size.width * k, size.height * k);
          cx.p.image(b, Rect::new(rr.x + (rr.w - iw) / 2.0, rr.y + (rr.h - ih) / 2.0 - 10.0, iw, ih));
        } else {
          let img: ID2D1Image = b.cast()?;
          cx.p.image_round(&img, size.width, size.height, rr, radius, 1.0)?;
        }
        drew = true;
      }
    }
  }
  if !drew {
    let icon = match g {
      G::Savers => "ambient_screen",
      G::Wall | G::Span => "image",
      _ => "movie",
    };
    cx.icon(icon, rr.x + rr.w / 2.0, rr.y + rr.h / 2.0 - if saver { 10.0 } else { 0.0 }, 28.0, false, cx.t.on_surface_variant)?;
  }
  if !t.res.is_empty() {
    let rw = cx.measure(&t.res, st(10.0))?.ceil() + 12.0;
    let b = Rect::new(rr.right() - 6.0 - rw, rr.bottom() - 5.0 - 16.0, rw, 16.0);
    cx.round(b, 8.0, Rgba(0, 0, 0, 0.55))?;
    cx.text_center(&t.res, b, st(10.0), Rgba(255, 255, 255, 1.0))?;
  }
  if !t.name.is_empty() && (saver || g != G::Wall) {
    // `.wall-name`: the name on a dark fade at the bottom
    let nb = Rect::new(rr.x, rr.bottom() - 22.0, rr.w, 22.0);
    if !saver {
      cx.push_clip(rr);
      cx.gradient_v(nb, Rgba(0, 0, 0, 0.0), Rgba(0, 0, 0, 0.7))?;
      cx.pop_clip();
    }
    let fg = if saver { cx.t.on_layer1 } else { Rgba(255, 255, 255, 1.0) };
    cx.text(&t.name, Rect::new(nb.x + 6.0, nb.y + 2.0, nb.w - 12.0, 18.0), st(11.0), fg)?;
  }
  if g == G::Videos && sb.walls.in_videos(&t.path) {
    cx.p.fill_circle(rr.right() - 14.0, rr.y + 14.0, 10.0, cx.t.primary)?;
    cx.icon("check", rr.right() - 14.0, rr.y + 14.0, 15.0, true, cx.t.on_primary)?;
  }
  if hot || sel || busy {
    cx.p.stroke_round(rr.inset(-1.0, -1.0), radius + 1.0, cx.t.primary, 2.0)?;
  }
  if busy {
    cx.round(rr, radius, Rgba(0, 0, 0, 0.45))?;
    let pw = if progress.is_empty() { 0.0 } else { cx.measure(progress, stw(12.0, 600.0))?.ceil() + 4.0 };
    let cxx = rr.x + rr.w / 2.0 - pw / 2.0;
    cx.spinner(cxx, rr.y + rr.h / 2.0, 22.0, Rgba(255, 255, 255, 1.0))?;
    if pw > 0.0 {
      cx.text(progress, Rect::new(cxx + 14.0, rr.y, pw, rr.h), stw(12.0, 600.0), Rgba(255, 255, 255, 1.0))?;
    }
  }
  Ok(())
}

fn paint_walls(cx: &mut Cx, sb: &mut Sidebar, x: f32, mut y: f32, w: f32) -> anyhow::Result<f32> {
  y += paint_monitors(cx, &sb.walls, x, y, w)?;
  let mut chips = Vec::new();
  for (k, label, icon) in WALL_CATS {
    chips.push((cx.tr(label), Some(icon), sb.walls.cat == k, Hit::Walls(WHit::Cat(k))));
  }
  y += chip_row(cx, sb, ROW_WALL, x, y, w, &chips)? + 10.0;
  let custom = cx.tr("Dosyadan seç");
  y += gallery(cx, sb, G::Wall, x, y, w, Some((&custom, "add_photo_alternate")), None)? + 14.0;
  if sb.walls.monitors().len() > 1 {
    y += cx.section_title(x, y, w, "panorama_wide_angle", &cx.tr("Superscreen · tüm monitörlere yayılan tek resim"))?;
    y += cx.hint(x, y, w, &cx.tr("Ultra geniş (32:9) öneriler; resim monitörlerin gerçek yerleşimine göre bölünür."))? + 8.0;
    y += gallery(cx, sb, G::Span, x, y, w, Some((&custom, "add_photo_alternate")), None)? + 14.0;
  }
  cx.text_center(&cx.tr("Öneriler: wallhaven.cc (yalnızca güvenli içerik)"), Rect::new(x, y, w, 16.0), st(11.0), cx.c.outline)?;
  Ok(y + 20.0)
}

fn paint_live(cx: &mut Cx, sb: &mut Sidebar, x: f32, mut y: f32, w: f32) -> anyhow::Result<f32> {
  y += cx.hint(x, y, w, &cx.tr("Video, masaüstü simgelerinin arkasında oynar. Ekran kilitliyken ve kapalıyken durur."))? + 10.0;
  y += paint_monitors(cx, &sb.walls, x, y, w)?;
  let custom = cx.tr("Video ya da paket içe aktar");
  y += gallery(cx, sb, G::Live, x, y, w, Some((&custom, "video_file")), None)? + 10.0;
  y += cx.hint(x, y, w, &cx.tr("İndirdiğin videoyu, Lively (.zip) ya da Wallpaper Engine video paketini (.zip ya da project.json) içe aktarabilirsin."))? + 12.0;
  if sb.walls.live_on() {
    let label = cx.tr("Canlı duvar kağıdını kapat");
    cx.chip(x, y, 32.0, &label, Some("stop_circle"), false, true, Hit::Walls(WHit::LiveClear))?;
    y += 32.0 + 10.0;
  }
  let opts = sb.walls.info.as_ref().map(|i| i["liveOptions"].clone()).unwrap_or(Value::Null);
  for (fullscreen, label) in [(true, "Tam ekran uygulamada duraklat"), (false, "Pille çalışırken duraklat")] {
    let on = opts[if fullscreen { "pauseFullscreen" } else { "pauseOnBattery" }].as_bool() != Some(false);
    y += switch_row(cx, x, y, w, &cx.tr(label), on, Hit::Walls(WHit::LiveOpt(fullscreen)))? + 4.0;
  }
  Ok(y)
}

/// A setting with a switch on the right; returns its height.
fn switch_row(cx: &mut Cx, x: f32, y: f32, w: f32, label: &str, on: bool, hit: Hit) -> anyhow::Result<f32> {
  let r = Rect::new(x, y, w, 40.0);
  if cx.hot(&hit) {
    cx.round(r, 14.0, cx.t.layer1)?;
  }
  cx.text(label, Rect::new(x + 8.0, y, w - 80.0, 40.0), st(13.0), cx.t.on_layer1)?;
  cx.switch(r.right() - 8.0 - 52.0, y + 4.0, on)?;
  cx.hit(r, hit);
  Ok(40.0)
}

fn paint_store(cx: &mut Cx, sb: &mut Sidebar, x: f32, mut y: f32, w: f32) -> anyhow::Result<f32> {
  y += cx.hint(x, y, w, &cx.tr("Mağazadaki her video canlı duvar kâğıdı ya da ekran koruyucu olabilir; indirilen video iki yerde de kütüphanende durur."))? + 10.0;
  match sb.walls.store_cats.clone() {
    None if sb.walls.store_failed => {
      y += cx.message(x, y, w, false, &cx.tr("Mağazaya ulaşılamadı"))? + 10.0;
      return Ok(y);
    }
    None => {
      cx.spinner(x + w / 2.0, y + 16.0, 22.0, cx.t.on_surface_variant)?;
      return Ok(y + 40.0);
    }
    Some(cats) => {
      let chips: Vec<_> = cats.iter().map(|c| (cx.tr(live_cat_name(c)), None, sb.walls.store_cat == *c, Hit::Walls(WHit::StoreCat(c.clone())))).collect();
      y += chip_row(cx, sb, ROW_STORE, x, y, w, &chips)? + 10.0;
    }
  }
  y += gallery(cx, sb, G::Store, x, y, w, None, None)? + 14.0;
  cx.text_center(&cx.tr("Videolar: Sucrose Store (yalnızca güvenli içerik)"), Rect::new(x, y, w, 16.0), st(11.0), cx.c.outline)?;
  Ok(y + 20.0)
}

fn paint_saver(cx: &mut Cx, sb: &mut Sidebar, _m: &Model, minutes: &mut TextField, x: f32, mut y: f32, w: f32) -> anyhow::Result<f32> {
  // Windows' settings, in a box (`.saver-sec`)
  let box_top = y;
  let saver = sb.walls.saver.clone();
  let inner_x = x + 14.0;
  let inner_w = w - 28.0;
  let mut iy = y + 14.0;
  let title = cx.tr("Ekran koruyucu");
  // the box behind (drawn first, height known after)
  let est = if saver.is_some() { 14.0 + 28.0 + 3.0 * 44.0 + 32.0 + 14.0 } else { 14.0 + 28.0 + 30.0 + 14.0 };
  cx.round(Rect::new(x, box_top, w, est), 17.0, cx.t.layer1)?;
  iy += cx.section_title(inner_x - 8.0, iy, inner_w, "ambient_screen", &title)?;
  if sb.walls.saving {
    cx.spinner(x + w - 14.0 - 8.0, iy - 15.0, 16.0, cx.t.primary)?;
  }
  match &saver {
    None if sb.walls.saver_err.is_empty() => {
      cx.text(&cx.tr("Windows ayarları okunuyor…"), Rect::new(inner_x, iy, inner_w, 22.0), st(12.0), cx.t.on_surface_variant)?;
      iy += 30.0;
    }
    None => {}
    Some(st_) => {
      let enabled = st_["enabled"].as_bool() == Some(true);
      let secure = st_["secure"].as_bool() == Some(true);
      iy += switch_row(cx, inner_x - 8.0, iy, inner_w + 16.0, &cx.tr("Etkin"), enabled, Hit::Walls(WHit::SaverEnabled))? + 4.0;
      // the wait in minutes: typed, saved when left or Enter
      cx.text(&cx.tr("Bekleme süresi"), Rect::new(inner_x, iy, inner_w / 2.0, 40.0), st(13.0), cx.t.on_layer1)?;
      if minutes.is_empty() && cx.focus != Some(FieldId::SaverMinutes) {
        minutes.set(&st_["minutes"].as_u64().unwrap_or(1).to_string());
      }
      let unit = cx.tr("dakika");
      let uw = cx.measure(&unit, st(13.0))?.ceil();
      let fr = Rect::new(inner_x + inner_w - (52.0 + 10.0 + uw + 10.0), iy + 3.0, 52.0 + 20.0 + uw, 34.0);
      cx.field_box(Rect::new(fr.x, fr.y, 62.0, fr.h), 12.0, minutes, FieldId::SaverMinutes, "", st(13.0), 10.0, Some(cx.c.layer2))?;
      cx.text(&unit, Rect::new(fr.x + 68.0, fr.y, uw + 2.0, fr.h), st(13.0), cx.t.on_layer1)?;
      iy += 44.0;
      iy += switch_row(cx, inner_x - 8.0, iy, inner_w + 16.0, &cx.tr("Dönüşte oturum açma iste"), secure, Hit::Walls(WHit::SaverSecure))? + 4.0;
      let has = !s(&st_["selected"]).is_empty();
      let opts = cx.tr("Seçenekler");
      let ow = cx.chip_w(&opts, Some("tune"))?;
      let prev = cx.tr("Önizle");
      let pw = cx.chip_w(&prev, Some("play_arrow"))?;
      cx.chip(inner_x + inner_w - ow, iy, 32.0, &opts, Some("tune"), false, has, Hit::Walls(WHit::SaverOptions))?;
      cx.chip(inner_x + inner_w - ow - 8.0 - pw, iy, 32.0, &prev, Some("play_arrow"), false, has, Hit::Walls(WHit::SaverPreview))?;
      iy += 32.0;
    }
  }
  if !sb.walls.saver_err.is_empty() {
    let err = sb.walls.saver_err.clone();
    iy += 8.0;
    iy += cx.message(inner_x, iy, inner_w, false, &err)?;
  }
  y = iy + 14.0 + 10.0;
  // the galleries: screen savers, our videos, the store for them
  let n = SAVER_SUBS.len();
  let sub_w = (w - (n - 1) as f32 * 4.0) / n as f32;
  for (i, (label, icon)) in SAVER_SUBS.iter().enumerate() {
    let b = Rect::new(x + i as f32 * (sub_w + 4.0), y, sub_w, 36.0);
    seg(cx, b, &cx.tr(label), icon, sb.walls.sub == i, Hit::Walls(WHit::Sub(i)), i == 0, i == n - 1)?;
  }
  y += 36.0 + 10.0;
  match sb.walls.sub {
    0 => {
      let import = cx.tr("İçe aktar");
      let selected = saver.as_ref().map(|s_| s(&s_["selected"]).to_string());
      y += gallery(cx, sb, G::Savers, x, y, w, Some((&import, "upload_file")), selected)? + 10.0;
    }
    _ => {
      y += cx.hint(x, y, w, &cx.tr("Canlı duvar kâğıdı kütüphanendeki videolar ekran koruyucu olarak oynar. Yenilerini Mağaza'dan indirebilirsin; birden çoksa karışık oynatılabilir."))? + 8.0;
      let n = sb.walls.videos["videos"].as_array().map_or(0, |a| a.len());
      if n > 1 {
        let shuffle = sb.walls.videos["shuffle"].as_bool() == Some(true);
        y += switch_row(cx, x, y, w, &cx.tr("Karışık"), shuffle, Hit::Walls(WHit::Shuffle))? + 6.0;
      }
      y += gallery(cx, sb, G::Videos, x, y, w, None, None)? + 10.0;
    }
  }
  Ok(y)
}

impl<'p, 'a> Cx<'p, 'a> {
  /// A vertical gradient (a gallery name's dark fade).
  pub fn gradient_v(&mut self, r: Rect, top: Rgba, bottom: Rgba) -> anyhow::Result<()> {
    use windows::Win32::Graphics::Direct2D::{
      Common::D2D1_GRADIENT_STOP, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT, D2D1_COLOR_SPACE_SRGB,
      D2D1_EXTEND_MODE_CLAMP, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    };
    unsafe {
      let stops = [D2D1_GRADIENT_STOP { position: 0.0, color: top.into() }, D2D1_GRADIENT_STOP { position: 1.0, color: bottom.into() }];
      let c = self.p.dc.CreateGradientStopCollection(
        &stops,
        D2D1_COLOR_SPACE_SRGB,
        D2D1_COLOR_SPACE_SRGB,
        D2D1_BUFFER_PRECISION_8BPC_UNORM,
        D2D1_EXTEND_MODE_CLAMP,
        D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
      )?;
      let brush = self.p.dc.CreateLinearGradientBrush(
        &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: crate::native_bar::gfx::pt(r.x, r.y), endPoint: crate::native_bar::gfx::pt(r.x, r.bottom()) },
        None,
        &c,
      )?;
      self.p.dc.FillRectangle(&r.d2d(), &brush);
    }
    Ok(())
  }
}
