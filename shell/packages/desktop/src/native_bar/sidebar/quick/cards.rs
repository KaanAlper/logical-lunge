//! The cards under the quick settings' wide tiles (sidebar.html WifiMenu,
//! EthernetMenu, BluetoothMenu, AudioMenu, NightLightMenu) and what they do.

use super::*;

fn signal_icon(s: i64) -> &'static str {
  if s >= 75 {
    "signal_wifi_4_bar"
  } else if s >= 50 {
    "network_wifi_3_bar"
  } else if s >= 25 {
    "network_wifi_2_bar"
  } else {
    "network_wifi_1_bar"
  }
}

impl Quick {
  /// Draws the card (an overlay over the panel). `wifi_field` is the
  /// password field.
  #[allow(clippy::too_many_arguments)]
  pub fn paint_card(&mut self, cx: &mut Cx, m: &Model, scroll: &mut HashMap<ScrollId, f32>, pw: &mut TextField, from: &mut TextField, to: &mut TextField, animations: bool) -> anyhow::Result<()> {
    self.card_rect = None;
    let Some(card) = &self.card else { return Ok(()) };
    let (tile, top, at, closing) = (card.tile, card.top, card.at, card.closing);
    let ms = cx.now.duration_since(at).as_secs_f32() * 1000.0;
    let k = if !animations {
      1.0
    } else if closing {
      (ms / CARD_OUT_MS).min(1.0)
    } else {
      (ms / CARD_IN_MS).min(1.0)
    };
    if closing && (k >= 1.0 || !animations) {
      self.card = None;
      return Ok(());
    }
    if k < 1.0 {
      cx.busy = true;
    }
    let shown = if closing { 1.0 - POP_OUT.at(k) } else { POP_IN.at(k) };
    let x = self.area.x + 6.0;
    let w = self.area.w - 12.0;
    let h = self.card_height(cx, m, tile, w - 16.0)?;
    let full = Rect::new(x, top, w, h);
    // opens from the top: slides down 12 DIP while its bottom edge is revealed
    let dy = -12.0 * (1.0 - shown);
    let r = Rect::new(full.x, full.y + dy, full.w, full.h);
    let opacity = if closing { shown } else { (shown * 2.0).min(1.0) };
    let reveal = Rect::new(r.x - 30.0, r.y - 30.0, r.w + 60.0, (r.h + 40.0) * shown + 30.0);
    cx.push_clip(reveal);
    cx.shadow(r, 17.0, opacity)?;
    cx.round(r, 17.0, cx.c.layer2.alpha(opacity))?;
    cx.p.stroke_round(r.inset(0.5, 0.5), 17.0, cx.c.border0.alpha(cx.c.border0.3 * opacity), 1.0)?;
    cx.hit(r, Hit::Quick(QHit::Card));
    let inner = Rect::new(r.x + 8.0, r.y + 8.0, r.w - 16.0, r.h - 16.0);
    match tile {
      Tile::Wifi => self.paint_wifi(cx, inner, scroll, pw)?,
      Tile::Ethernet => self.paint_eth(cx, inner)?,
      Tile::Bluetooth => self.paint_bt(cx, inner, scroll)?,
      Tile::Audio => self.paint_audio(cx, m, inner, true, scroll)?,
      Tile::Mic => self.paint_audio(cx, m, inner, false, scroll)?,
      Tile::NightLight => self.paint_night(cx, inner, from, to)?,
      _ => {}
    }
    cx.pop_clip();
    self.card_rect = Some(r);
    Ok(())
  }

  fn card_height(&self, cx: &mut Cx, m: &Model, tile: Tile, w: f32) -> anyhow::Result<f32> {
    let foot = 6.0 + 32.0;
    let title = 24.0;
    let gap = 4.0;
    let h = match tile {
      Tile::Wifi => {
        let mut h = title + gap;
        if !self.wifi_on() || self.wifi.is_none() {
          h += 30.0 + gap;
        } else {
          h += self.wifi_list_h().min(240.0) + gap;
        }
        if !self.wifi_err.is_empty() {
          h += cx.wrapped_h(&self.wifi_err, st(13.0), w - 20.0, 80.0)? + 8.0 + gap;
        }
        h + foot
      }
      Tile::Ethernet => title + gap + 4.0 * (29.0 + gap) + foot,
      Tile::Bluetooth => {
        if !self.bt_on() {
          30.0 + gap + foot
        } else {
          let devs = self.hw.bt["devices"].as_array().cloned().unwrap_or_default();
          let conn = devs.iter().filter(|d| d["connected"].as_bool() == Some(true)).count();
          let paired = devs.len() - conn;
          let mut h = 0.0;
          if conn > 0 {
            h += title + gap + (conn as f32 * 46.0).min(240.0) + gap;
          }
          h += title + gap + if paired > 0 { (paired as f32 * 46.0).min(240.0) } else { 30.0 } + gap;
          h + foot
        }
      }
      Tile::Audio | Tile::Mic => {
        let out = tile == Tile::Audio;
        let devs = audio_devices(m, out);
        let has_def = devs.iter().any(|d| if out { d.is_default_playback } else { d.is_default_recording });
        let mut h = title + gap;
        if has_def {
          h += 46.0 + gap;
        }
        h += if devs.is_empty() { 30.0 } else { (devs.len() as f32 * 46.0).min(240.0) } + gap;
        h + foot
      }
      Tile::NightLight => {
        let mode = s(&self.hw.night["mode"]);
        let mut h = 48.0 + gap + title + gap + 38.0 + gap;
        if !mode.is_empty() && mode != "manual" {
          h += 4.0 + 18.0 + 38.0 + 4.0 + gap;
        }
        h + title + gap + 44.0 + gap + 34.0
      }
      _ => 0.0,
    };
    Ok(h + 16.0)
  }

  fn wifi_list_h(&self) -> f32 {
    let Some(data) = &self.wifi else { return 0.0 };
    let n = data["networks"].as_array().map_or(0, |a| a.len());
    let mut h = n as f32 * 46.0;
    if self.wifi_ask.is_some() {
      h += 52.0;
    }
    if n == 0 {
      h = 30.0;
    }
    h
  }

  fn card_title(cx: &mut Cx, r: Rect, y: f32, text: &str, busy: bool) -> anyhow::Result<f32> {
    let w = cx.text(text, Rect::new(r.x + 8.0, y + 4.0, r.w - 40.0, 18.0), st(13.0), cx.t.on_surface_variant)?;
    if busy {
      cx.spinner(r.x + 8.0 + w + 6.0 + 8.0, y + 13.0, 16.0, cx.t.on_surface_variant)?;
    }
    Ok(24.0)
  }

  fn empty_line(cx: &mut Cx, r: Rect, y: f32, text: &str) -> anyhow::Result<f32> {
    cx.text(text, Rect::new(r.x + 12.0, y + 6.0, r.w - 24.0, 18.0), st(13.0), cx.t.on_surface_variant)?;
    Ok(30.0)
  }

  /// `.qm-foot`: Scan (when given) and "More" (Windows settings).
  fn footer(cx: &mut Cx, r: Rect, y: f32, scan: Option<bool>, settings: &'static str) -> anyhow::Result<f32> {
    let more = cx.tr("Daha fazla");
    let mw = cx.chip_w(&more, Some("open_in_new"))?;
    let mx = r.right() - mw;
    cx.chip(mx, y + 6.0, 32.0, &more, Some("open_in_new"), false, true, Hit::Quick(QHit::More(settings)))?;
    if let Some(busy) = scan {
      let label = if busy { cx.tr("Taranıyor…") } else { cx.tr("Tara") };
      let sw = cx.chip_w(&label, Some("refresh"))?;
      let sx = mx - 6.0 - sw;
      cx.chip(sx, y + 6.0, 32.0, &label, if busy { None } else { Some("refresh") }, false, !busy, Hit::Quick(QHit::Scan))?;
      if busy {
        cx.spinner(sx + 12.0 + 9.0, y + 6.0 + 16.0, 18.0, cx.t.on_layer1)?;
      }
    }
    Ok(38.0)
  }

  /// A list row (`.net-row`): icon, name, the right side; current: tinted.
  #[allow(clippy::too_many_arguments)]
  fn net_row(cx: &mut Cx, r: Rect, icon: &str, cur: bool, name: &str, right: Option<&str>, right_icon: Option<&str>, busy: bool, hit: Option<Hit>) -> anyhow::Result<()> {
    let hot = hit.as_ref().is_some_and(|h| cx.hot(h));
    let (bg, fg) = if cur {
      (Some(cx.t.sec_container), cx.t.on_sec_container)
    } else if hot {
      (Some(cx.c.layer3), cx.t.on_layer1)
    } else {
      (None, cx.t.on_layer1)
    };
    if let Some(c) = bg {
      cx.round(r, 17.0, c)?;
    }
    cx.icon(icon, r.x + 12.0 + 10.0, r.y + r.h / 2.0, 20.0, cur, fg)?;
    let mut right_w = 0.0;
    if busy {
      cx.spinner(r.right() - 12.0 - 10.0, r.y + r.h / 2.0, 20.0, fg)?;
      right_w = 24.0;
    } else if let Some(t) = right {
      right_w = cx.text_right(t, Rect::new(r.x, r.y, r.w - 12.0, r.h), st(12.0), fg)? + 8.0;
    } else if let Some(i) = right_icon {
      cx.icon(i, r.right() - 12.0 - 10.0, r.y + r.h / 2.0, 20.0, false, fg)?;
      right_w = 24.0;
    }
    let nx = r.x + 12.0 + 20.0 + 12.0;
    cx.text(name, Rect::new(nx, r.y, r.right() - 12.0 - right_w - nx, r.h), st(14.0), fg)?;
    if let Some(h) = hit {
      cx.hit(r, h);
    }
    Ok(())
  }

  fn list_area(cx: &mut Cx, r: Rect, y: f32, content: f32, id: ScrollId, scroll: &mut HashMap<ScrollId, f32>) -> (Rect, f32) {
    let h = content.min(240.0);
    let area = Rect::new(r.x, y, r.w, h);
    let max = (content - h).max(0.0);
    let off = scroll.get(&id).copied().unwrap_or(0.0).clamp(0.0, max);
    scroll.insert(id, off);
    cx.region(area, id, content, false);
    (area, off)
  }

  fn paint_wifi(&mut self, cx: &mut Cx, r: Rect, scroll: &mut HashMap<ScrollId, f32>, pw: &mut TextField) -> anyhow::Result<()> {
    let mut y = r.y;
    let on = self.wifi_on();
    y += Self::card_title(cx, r, y, &cx.tr("Kullanılabilir ağlar"), self.wifi_scanning)? + 4.0;
    if !on {
      y += Self::empty_line(cx, r, y, &cx.tr("Wi-Fi kapalı"))? + 4.0;
    } else if self.wifi.is_none() {
      y += Self::empty_line(cx, r, y, &cx.tr("Taranıyor…"))? + 4.0;
    } else {
      let data = self.wifi.clone().unwrap_or(Value::Null);
      let nets = data["networks"].as_array().cloned().unwrap_or_default();
      let connected = s(&data["connected"]).to_string();
      if nets.is_empty() {
        y += Self::empty_line(cx, r, y, &cx.tr("Bir şey bulunamadı"))? + 4.0;
      } else {
        let content = self.wifi_list_h();
        let (area, off) = Self::list_area(cx, r, y, content, ScrollId::Card, scroll);
        cx.push_clip(area);
        let mut ry = area.y - off;
        for n in &nets {
          let ssid = s(&n["ssid"]).to_string();
          let cur = ssid == connected && !ssid.is_empty();
          let busy = self.wifi_busy.as_deref() == Some(ssid.as_str());
          let secure = n["secure"].as_bool() != Some(false);
          let state = cx.tr("Bağlı");
          Self::net_row(
            cx,
            Rect::new(r.x, ry, r.w, 44.0),
            signal_icon(n["signal"].as_i64().unwrap_or(0)),
            cur,
            &ssid,
            if cur { Some(state.as_str()) } else { None },
            if !cur && secure { Some("lock") } else { None },
            busy,
            if self.wifi_busy.is_some() { None } else { Some(Hit::Quick(QHit::Net(ssid.clone()))) },
          )?;
          ry += 46.0;
          if self.wifi_ask.as_deref() == Some(ssid.as_str()) {
            // `.net-pw`: the password and the arrow
            let fr = Rect::new(r.x + 12.0, ry + 4.0, r.w - 24.0 - 42.0, 36.0);
            let ph = cx.tr("Şifre");
            cx.field_box(fr, 18.0, pw, FieldId::WifiPw, &ph, st(14.0), 14.0, None)?;
            let fab = Rect::new(fr.right() + 6.0, ry + 4.0, 36.0, 36.0);
            let ok = !pw.is_empty() && self.wifi_busy.is_none();
            cx.round(fab, 12.0, cx.t.primary_container.alpha(if ok { 1.0 } else { 0.5 }))?;
            cx.icon("arrow_forward", fab.x + 18.0, fab.y + 18.0, 22.0, false, cx.t.on_primary_container)?;
            if ok {
              cx.hit(fab, Hit::Quick(QHit::NetSubmit));
            }
            ry += 52.0;
          }
        }
        cx.pop_clip();
        y += area.h + 4.0;
      }
    }
    if !self.wifi_err.is_empty() {
      let h = cx.wrapped_h(&self.wifi_err, st(13.0), r.w - 20.0, 80.0)?;
      cx.p.text_wrapped(&self.wifi_err, Rect::new(r.x + 10.0, y + 4.0, r.w - 20.0, h + 2.0), st(13.0), Rgba::hex(0xffb4ab), false)?;
      y += h + 8.0 + 4.0;
    }
    Self::footer(cx, r, y, on.then_some(self.wifi_scanning), "ms-settings:network-wifi")?;
    Ok(())
  }

  fn paint_eth(&self, cx: &mut Cx, r: Rect) -> anyhow::Result<()> {
    let mut y = r.y;
    y += Self::card_title(cx, r, y, "Ethernet", false)? + 4.0;
    let e = &self.hw.eth;
    let state = match s(&e["state"]) {
      "up" => cx.tr("Bağlı"),
      "disabled" => cx.tr("Devre dışı"),
      _ => cx.tr("Kablo takılı değil"),
    };
    let rows = [
      (cx.tr("Durum"), state),
      (cx.tr("Kart"), s(&e["desc"]).to_string()),
      (cx.tr("Hız"), s(&e["speed"]).to_string()),
      (cx.tr("IP adresi"), s(&e["ip"]).to_string()),
    ];
    for (k, v) in rows {
      let row = Rect::new(r.x + 12.0, y, r.w - 24.0, 29.0);
      let kw = cx.text(&k, row, st(13.0), cx.t.on_surface_variant)?;
      let v = if v.is_empty() { "—".to_string() } else { v };
      cx.text_right(&v, Rect::new(row.x + kw + 12.0, row.y, row.w - kw - 12.0, row.h), st(13.0), cx.t.on_layer1)?;
      y += 29.0 + 4.0;
    }
    Self::footer(cx, r, y, None, "ms-settings:network-ethernet")?;
    Ok(())
  }

  fn paint_bt(&self, cx: &mut Cx, r: Rect, scroll: &mut HashMap<ScrollId, f32>) -> anyhow::Result<()> {
    let mut y = r.y;
    let on = self.bt_on();
    if !on {
      y += Self::empty_line(cx, r, y, &cx.tr("Bluetooth kapalı"))? + 4.0;
    } else {
      let devs = self.hw.bt["devices"].as_array().cloned().unwrap_or_default();
      let (conn, paired): (Vec<&Value>, Vec<&Value>) = devs.iter().partition(|d| d["connected"].as_bool() == Some(true));
      let state = cx.tr("Bağlı");
      for (list, title, id) in [(conn, cx.tr("Bağlı cihazlar"), ScrollId::Card), (paired, cx.tr("Eşleşmiş cihazlar"), ScrollId::Card2)] {
        let is_conn = id == ScrollId::Card;
        if is_conn && list.is_empty() {
          continue;
        }
        y += Self::card_title(cx, r, y, &title, false)? + 4.0;
        if list.is_empty() {
          y += Self::empty_line(cx, r, y, &cx.tr("Eşleşmiş başka cihaz yok"))? + 4.0;
          continue;
        }
        let (area, off) = Self::list_area(cx, r, y, list.len() as f32 * 46.0, id, scroll);
        cx.push_clip(area);
        let mut ry = area.y - off;
        for d in list {
          let kind = s(&d["kind"]);
          Self::net_row(
            cx,
            Rect::new(r.x, ry, r.w, 44.0),
            if kind.is_empty() { "bluetooth" } else { kind },
            is_conn,
            s(&d["name"]),
            if is_conn { Some(state.as_str()) } else { None },
            None,
            false,
            None,
          )?;
          ry += 46.0;
        }
        cx.pop_clip();
        y += area.h + 4.0;
      }
    }
    Self::footer(cx, r, y, on.then_some(self.scanning), "ms-settings:bluetooth")?;
    Ok(())
  }

  fn paint_audio(&mut self, cx: &mut Cx, m: &Model, r: Rect, out: bool, scroll: &mut HashMap<ScrollId, f32>) -> anyhow::Result<()> {
    let mut y = r.y;
    let devs = audio_devices(m, out);
    let def = devs.iter().find(|d| if out { d.is_default_playback } else { d.is_default_recording }).cloned();
    y += Self::card_title(cx, r, y, &cx.tr(if out { "Çıkış cihazı" } else { "Giriş cihazı" }), false)? + 4.0;
    if let Some(d) = &def {
      let vol = match &self.audio_drag {
        Some((id, v, _)) if *id == d.device_id => *v,
        _ => d.volume,
      };
      let row = Rect::new(r.x + 6.0, y + 4.0, r.w - 12.0, 36.0);
      let mute = Rect::new(row.x, row.y, 36.0, 36.0);
      let (bg, fg) = if d.is_muted { (Rgba::hex(0x93000a), Rgba::hex(0xffdad6)) } else { (cx.t.sec_container, cx.t.on_sec_container) };
      cx.round(mute, if d.is_muted { 18.0 } else { 12.0 }, bg)?;
      let icon = if out {
        if d.is_muted || vol == 0 { "volume_off" } else if vol < 50 { "volume_down" } else { "volume_up" }
      } else if d.is_muted {
        "mic_off"
      } else {
        "mic"
      };
      cx.icon(icon, mute.x + 18.0, mute.y + 18.0, 20.0, true, fg)?;
      cx.hit(mute, Hit::Quick(QHit::AudioMute));
      let val = format!("{}", vol);
      let track = Rect::new(mute.right() + 10.0, row.y + 11.0, row.w - 36.0 - 10.0 - 10.0 - 28.0, 14.0);
      self.audio_track = track;
      cx.slider(track, vol as f32 / 100.0, cx.t.primary, cx.c.layer3, Hit::Quick(QHit::AudioSlider))?;
      cx.p.text(&val, Rect::new(track.right() + 10.0, row.y, 28.0, 36.0), st(13.0), cx.t.on_surface_variant, crate::native_bar::view::Align::Center, true)?;
      y += 46.0 + 4.0;
    }
    if devs.is_empty() {
      y += Self::empty_line(cx, r, y, &cx.tr("Bir şey bulunamadı"))? + 4.0;
    } else {
      let (area, off) = Self::list_area(cx, r, y, devs.len() as f32 * 46.0, ScrollId::Card, scroll);
      cx.push_clip(area);
      let mut ry = area.y - off;
      for d in &devs {
        let is_def = if out { d.is_default_playback } else { d.is_default_recording };
        let lower = d.name.to_lowercase();
        let icon = if !out {
          "mic"
        } else if ["head", "kulak", "buds", "airpods"].iter().any(|k| lower.contains(k)) {
          "headphones"
        } else {
          "speaker"
        };
        let busy = self.audio_busy.as_deref() == Some(d.device_id.as_str());
        Self::net_row(
          cx,
          Rect::new(r.x, ry, r.w, 44.0),
          icon,
          is_def,
          &d.name,
          None,
          if is_def { Some("check") } else { None },
          busy,
          Some(Hit::Quick(QHit::AudioDev(d.device_id.clone()))),
        )?;
        ry += 46.0;
      }
      cx.pop_clip();
      y += area.h + 4.0;
    }
    Self::footer(cx, r, y, None, "ms-settings:sound")?;
    Ok(())
  }

  fn paint_night(&mut self, cx: &mut Cx, r: Rect, from: &mut TextField, to: &mut TextField) -> anyhow::Result<()> {
    let n = self.hw.night.clone();
    let mut y = r.y;
    let on = n["on"].as_bool() == Some(true);
    // the switch row
    let row = Rect::new(r.x, y, r.w, 48.0);
    let hit = Hit::Quick(QHit::NightToggle);
    if cx.hot(&hit) {
      cx.round(row, 17.0, cx.c.layer3)?;
    }
    cx.icon("nightlight", row.x + 12.0 + 10.0, row.y + 24.0, 20.0, on, cx.t.on_layer1)?;
    cx.text(&cx.tr("Gece ışığı"), Rect::new(row.x + 44.0, row.y, row.w - 44.0 - 70.0, 48.0), st(14.0), cx.t.on_layer1)?;
    cx.switch(row.right() - 12.0 - 52.0, row.y + 8.0, on)?;
    cx.hit(row, hit);
    y += 48.0 + 4.0;
    y += Self::card_title(cx, r, y, &cx.tr("Zamanlama"), false)? + 4.0;
    // `.nl-seg`: three joined buttons
    let mode = match s(&n["mode"]) {
      "" => "manual",
      m => m,
    }
    .to_string();
    let modes: [(&'static str, &str, &str); 3] = [("manual", "Her zaman", "wb_sunny"), ("after", "Saatten sonra", "bedtime"), ("range", "Saat aralığı", "schedule")];
    let seg_w = (r.w - 8.0 - 8.0) / 3.0;
    for (i, (k, label, icon)) in modes.iter().enumerate() {
      let b = Rect::new(r.x + 4.0 + i as f32 * (seg_w + 4.0), y, seg_w, 38.0);
      let sel = mode == *k;
      let hit = Hit::Quick(QHit::NightMode(k));
      let (bg, fg) = if sel {
        (cx.t.sec_container, cx.t.on_sec_container)
      } else if cx.hot(&hit) {
        (cx.t.layer1_hover, cx.t.on_layer1)
      } else {
        (cx.t.layer1, cx.t.on_layer1)
      };
      cx.round(b, if sel { 19.0 } else { 10.0 }, bg)?;
      let label = cx.tr(label);
      let lw = cx.measure(&label, st(13.0))?.min(b.w - 30.0);
      let cxx = b.x + (b.w - lw - 17.0 - 6.0) / 2.0;
      cx.icon(icon, cxx + 8.5, b.y + 19.0, 17.0, sel, fg)?;
      cx.text(&label, Rect::new(cxx + 23.0, b.y, lw + 2.0, 38.0), st(13.0), fg)?;
      cx.hit(b, hit);
    }
    y += 38.0 + 4.0;
    if mode != "manual" {
      // `.nl-times`: start (and end), HH:MM fields
      let y0 = y + 4.0;
      let half = (r.w - 8.0 - 8.0) / 2.0;
      let lbl = cx.tr("Başlangıç");
      cx.text(&lbl, Rect::new(r.x + 4.0, y0, half, 16.0), st(12.0), cx.t.on_surface_variant)?;
      if from.is_empty() && cx.focus != Some(FieldId::NightFrom) {
        from.set(n["from"].as_str().unwrap_or("20:00"));
      }
      let fr = Rect::new(r.x + 4.0, y0 + 18.0, half, 38.0);
      cx.field_box(fr, 12.0, from, FieldId::NightFrom, "20:00", st(15.0), 10.0, Some(cx.t.layer1))?;
      let x2 = r.x + 4.0 + half + 8.0;
      if mode == "range" {
        cx.text(&cx.tr("Bitiş"), Rect::new(x2, y0, half, 16.0), st(12.0), cx.t.on_surface_variant)?;
        if to.is_empty() && cx.focus != Some(FieldId::NightTo) {
          to.set(n["to"].as_str().unwrap_or("07:00"));
        }
        cx.field_box(Rect::new(x2, y0 + 18.0, half, 38.0), 12.0, to, FieldId::NightTo, "07:00", st(15.0), 10.0, Some(cx.t.layer1))?;
      } else {
        let note = cx.tr("Sabah 07:00'de kapanır");
        cx.icon("wb_twilight", x2 + 8.0, y0 + 18.0 + 27.0, 16.0, false, cx.t.on_surface_variant)?;
        cx.text(&note, Rect::new(x2 + 22.0, y0 + 18.0 + 18.0, half - 22.0, 18.0), st(12.0), cx.t.on_surface_variant)?;
      }
      y += 4.0 + 18.0 + 38.0 + 4.0 + 4.0;
    }
    // intensity: title with "%60 · 3740K"
    let level = self.night_level.unwrap_or(n["level"].as_u64().unwrap_or(50) as u32);
    let kelvin = ((6500.0 - 46.0 * level as f32) / 100.0).round() * 100.0;
    let t_w = cx.text(&cx.tr("Yoğunluk"), Rect::new(r.x + 8.0, y + 4.0, r.w / 2.0, 18.0), st(13.0), cx.t.on_surface_variant)?;
    let _ = t_w;
    cx.p.text(
      &format!("%{} · {}K", level, kelvin as i32),
      Rect::new(r.x + r.w / 2.0, y + 4.0, r.w / 2.0 - 8.0, 18.0),
      st(13.0),
      cx.t.primary,
      crate::native_bar::view::Align::Left,
      true,
    )
    .map(|_| ())?;
    y += 24.0 + 4.0;
    // the warmth track: white to orange, ticks at a quarter, half, three quarters
    let track = Rect::new(r.x + 8.0, y + 14.0, r.w - 16.0, 16.0);
    self.night_track = track;
    cx.gradient(track, 8.0, &[(0.0, Rgba::hex(0xfff4e8).alpha(0.95)), (0.5, Rgba::hex(0xffc98a).alpha(0.95)), (1.0, Rgba::hex(0xff8a3d).alpha(0.95))])?;
    for tk in [0.25, 0.5, 0.75] {
      cx.p.fill_circle(track.x + track.w * tk, track.y + 8.0, 1.5, Rgba(0, 0, 0, 0.35))?;
    }
    cx.thumb(Rect::new(track.x, track.y - 7.0, track.w, 30.0), level as f32 / 100.0)?;
    cx.hit(Rect::new(track.x - 6.0, track.y - 10.0, track.w + 12.0, 36.0), Hit::Quick(QHit::NightSlider));
    y += 44.0 + 4.0;
    // presets
    let pw = (r.w - 8.0 - 18.0) / 4.0;
    for (i, v) in [25u32, 50, 75, 100].iter().enumerate() {
      let b = Rect::new(r.x + 4.0 + i as f32 * (pw + 6.0), y, pw, 32.0);
      cx.chip_in(b, &format!("%{v}"), None, level == *v, true, Hit::Quick(QHit::NightPreset(*v)))?;
    }
    Ok(())
  }

}

impl Ui {
  pub(in crate::native_bar::sidebar) fn sb_wifi_scan(&mut self) {
    let q = &mut self.sidebar.quick;
    if q.wifi_scanning {
      return;
    }
    q.wifi_scanning = true;
    spawn(|| ev(QEv::Wifi(wifi("list", "", None))));
  }

  pub(in crate::native_bar::sidebar) fn sb_toggle_menu(&mut self, tile: Tile) {
    let open = self.sidebar.quick.menu != Some(tile);
    if !open {
      self.sidebar.quick.set_menu(None, 0.0);
      return;
    }
    let top = self.sidebar.quick.tile_rect(tile).map_or(0.0, |r| r.bottom() + 6.0);
    self.sidebar.quick.set_menu(Some(tile), top);
    match tile {
      Tile::Wifi => {
        if self.sidebar.quick.wifi_on() {
          self.sb_wifi_scan();
        }
      }
      Tile::NightLight => {
        self.sidebar.field(FieldId::NightFrom).set("");
        self.sidebar.field(FieldId::NightTo).set("");
      }
      _ => {}
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_scan(&mut self) {
    match self.sidebar.quick.menu {
      Some(Tile::Wifi) => self.sb_wifi_scan(),
      Some(Tile::Bluetooth) => {
        self.sidebar.quick.scanning = true;
        spawn(|| {
          if let Some(v) = qs("bt") {
            ev(QEv::Bt(v));
          }
          ev(QEv::ScanDone);
        });
      }
      _ => {}
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_wifi_row(&mut self, ssid: String) {
    let q = &mut self.sidebar.quick;
    let data = q.wifi.clone().unwrap_or(Value::Null);
    let cur = s(&data["connected"]) == ssid;
    if cur {
      q.wifi_busy = Some(ssid);
      spawn(|| {
        let _ = wifi("disconnect", "", None);
        ev(QEv::Wifi(wifi("list", "", None)));
      });
      // the list read clears the busy mark through WifiConnected
      let s2 = q.wifi_busy.clone().unwrap_or_default();
      spawn(move || ev(QEv::WifiConnected(s2, json!({ "ok": true }))));
      return;
    }
    let net = data["networks"].as_array().and_then(|a| a.iter().find(|n| s(&n["ssid"]) == ssid)).cloned().unwrap_or(Value::Null);
    let known = net["known"].as_bool() == Some(true);
    let secure = net["secure"].as_bool() != Some(false);
    if known || !secure {
      self.sb_wifi_connect(ssid, None);
    } else {
      let ask = if q.wifi_ask.as_deref() == Some(ssid.as_str()) { None } else { Some(ssid) };
      q.wifi_ask = ask.clone();
      q.wifi_err.clear();
      self.sidebar.field(FieldId::WifiPw).set("");
      self.sidebar.focus = ask.map(|_| FieldId::WifiPw);
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_wifi_connect(&mut self, ssid: String, password: Option<String>) {
    let q = &mut self.sidebar.quick;
    q.wifi_err.clear();
    q.wifi_busy = Some(ssid.clone());
    spawn(move || {
      let r = wifi("connect", &ssid, password.as_deref()).unwrap_or(Value::Null);
      ev(QEv::WifiConnected(ssid, r));
    });
  }

  pub(in crate::native_bar::sidebar) fn sb_wifi_submit(&mut self) {
    let pw = self.sidebar.field(FieldId::WifiPw).text();
    let Some(ssid) = self.sidebar.quick.wifi_ask.clone() else { return };
    if pw.is_empty() || self.sidebar.quick.wifi_busy.is_some() {
      return;
    }
    self.sb_wifi_connect(ssid, Some(pw));
  }

  pub(in crate::native_bar::sidebar) fn sb_night_set(&mut self, key: &'static str, value: String) {
    spawn(move || {
      if let Some(v) = core_json(&["--nightlight-set", key, &value]) {
        ev(QEv::Night(v));
      }
    });
  }

  /// A night light time typed: saved when it is a valid HH:MM.
  pub(in crate::native_bar::sidebar) fn sb_night_time(&mut self, id: FieldId) {
    let text = self.sidebar.field(id).text();
    let ok = valid_time(&text);
    let key = if id == FieldId::NightFrom { "from" } else { "to" };
    if let Some(t) = ok {
      self.sb_night_set(key, t);
    } else {
      // back to what is saved
      let saved = self.sidebar.quick.hw.night[key].as_str().unwrap_or("").to_string();
      self.sidebar.field(id).set(&saved);
    }
  }

  /// Slider presses and drags (audio volume, night light warmth).
  pub(in crate::native_bar::sidebar) fn sb_quick_slide(&mut self, which: &QHit, x: f32, done: bool) {
    match which {
      QHit::AudioSlider => {
        let out = self.sidebar.quick.menu == Some(Tile::Audio);
        let Some(d) = audio_devices(&self.model, out).into_iter().find(|d| if out { d.is_default_playback } else { d.is_default_recording }) else { return };
        let t = self.sidebar.quick.audio_track;
        let v = (((x - t.x) / t.w).clamp(0.0, 1.0) * 100.0).round() as u32;
        let due = self.sidebar.quick.audio_drag.as_ref().map_or(true, |(_, _, at)| at.elapsed() >= Duration::from_millis(40));
        if due || done {
          self.provider("audio", ProviderFunction::Audio(AudioFunction::SetVolume(SetVolumeArgs { volume: v as f32, device_id: Some(d.device_id.clone()) })));
          self.sidebar.quick.audio_drag = Some((d.device_id, v, Instant::now()));
        } else if let Some(a) = self.sidebar.quick.audio_drag.as_mut() {
          a.1 = v;
        }
        if done {
          // the provider's next report takes over
          let id = self.sidebar.quick.audio_drag.as_ref().map(|a| a.0.clone()).unwrap_or_default();
          let _ = id;
          self.sidebar.quick.audio_drag = None;
        }
      }
      QHit::NightSlider => {
        let t = self.sidebar.quick.night_track;
        let v = ((((x - t.x) / t.w).clamp(0.0, 1.0) * 100.0 / 5.0).round() * 5.0) as u32;
        let changed = self.sidebar.quick.night_level != Some(v);
        self.sidebar.quick.night_level = Some(v);
        self.sidebar.quick.night_drag = !done;
        let due = self.sidebar.quick.night_sent.map_or(true, |at| at.elapsed() >= Duration::from_millis(60));
        if (changed && due) || done {
          self.sidebar.quick.night_sent = Some(Instant::now());
          self.sb_night_set("level", v.to_string());
        }
      }
      _ => {}
    }
    self.sb_render();
  }

  /// The wheel on the volume slider: two steps of one percent.
  pub(in crate::native_bar::sidebar) fn sb_audio_wheel(&mut self, up: bool) {
    let out = self.sidebar.quick.menu == Some(Tile::Audio);
    let Some(d) = audio_devices(&self.model, out).into_iter().find(|d| if out { d.is_default_playback } else { d.is_default_recording }) else { return };
    let v = (d.volume as i32 + if up { 2 } else { -2 }).clamp(0, 100);
    self.provider("audio", ProviderFunction::Audio(AudioFunction::SetVolume(SetVolumeArgs { volume: v as f32, device_id: Some(d.device_id) })));
  }

  pub(in crate::native_bar::sidebar) fn sb_quick_key(&mut self, id: FieldId, t: Typed) {
    match (id, t) {
      (FieldId::WifiPw, Typed::Submit) => self.sb_wifi_submit(),
      (FieldId::NightFrom | FieldId::NightTo, Typed::Submit) => {
        self.sb_night_time(id);
        self.sidebar.focus = None;
      }
      _ => {}
    }
  }
}
