//! What the wallpaper page does: loading its lists, applying, downloading,
//! importing, removing, the screen saver settings, the right-click menus.

use super::*;

impl Ui {
  pub(in crate::native_bar::sidebar) fn sb_walls_open(&mut self) {
    let w = &mut self.sidebar.walls;
    w.msg = None;
    w.confirm = None;
    w.busy = None;
    self.sb_walls_load_tab();
  }

  /// What the open tab shows, read once per opening (galleries of the web stay cached).
  fn sb_walls_load_tab(&mut self) {
    let w = &mut self.sidebar.walls;
    match w.tab {
      0 => {
        load_info();
        if w.cat == "local" || !w.items.contains_key(&w.cat) {
          w.items.insert(w.cat.clone(), None);
          load_cat(w.cat.clone());
        }
        if !w.items.contains_key("span") {
          w.items.insert("span".into(), None);
          load_cat("span".into());
        }
      }
      1 => {
        load_info();
        w.live = None;
        load_live();
      }
      2 => self.sb_walls_store(),
      _ => {
        w.saver_err.clear();
        load_saver();
        match w.sub {
          1 => {
            w.live = None;
            load_live();
          }
          2 => self.sb_walls_store(),
          _ => {}
        }
      }
    }
  }

  fn sb_walls_store(&mut self) {
    let w = &mut self.sidebar.walls;
    if w.store_cats.is_none() {
      w.store_failed = false;
      load_store_cats();
    }
    if !w.store.contains_key(&w.store_cat) {
      w.store.insert(w.store_cat.clone(), None);
      load_store(w.store_cat.clone());
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_walls_event(&mut self, e: WEv) {
    let w = &mut self.sidebar.walls;
    match e {
      WEv::Info(v) => {
        if v["monitors"].is_array() {
          w.info = Some(v);
        }
      }
      WEv::Items(cat, list) => {
        w.items.insert(cat, Some(list));
      }
      WEv::Live(list) => w.live = Some(list),
      WEv::StoreCats(cats) => {
        w.store_failed = cats.is_none();
        if cats.is_none() {
          w.msg = Some((false, self.model.tr("Mağazaya ulaşılamadı")));
        }
        w.store_cats = cats;
      }
      WEv::Store(cat, list) => {
        w.store.insert(cat, Some(list));
      }
      WEv::Done(ok, text, reload) => {
        w.busy = None;
        w.progress.clear();
        if !text.is_empty() {
          w.msg = Some((ok, text));
        }
        if reload {
          self.sb_walls_reload();
        }
      }
      // the file dialog closed: losing the focus closes the panel again
      WEv::Progress(p) if p == "\u{0}modal" => self.sb_modal(false),
      WEv::Progress(p) => w.progress = p,
      WEv::Saver(r) => {
        w.saving = false;
        match r {
          Ok(v) => {
            w.saver = Some(v);
            self.sidebar.field(FieldId::SaverMinutes).set("");
          }
          Err(e) => w.saver_err = e,
        }
      }
      WEv::SaverIcons(v) => {
        if v.is_object() && v["error"].is_null() {
          w.icons = v;
        }
      }
      WEv::Videos(v) => {
        if v["error"].is_string() {
          let text = self.model.tr("Ekran koruyucu videosu ayarlanamadı");
          w.msg = Some((false, text));
        } else if v.is_object() {
          w.videos = v;
        }
        w.busy = None;
        w.progress.clear();
      }
      WEv::Removed(g, path, ok) => {
        if ok {
          self.sidebar.images.forget(&path);
          let text = self.model.tr("Kütüphaneden kaldırıldı");
          self.sidebar.walls.msg = Some((true, text));
          let _ = g;
          self.sb_walls_reload();
        } else {
          let text = self.model.tr("Kaldırılamadı");
          self.sidebar.walls.msg = Some((false, text));
        }
      }
    }
    self.sb_render();
  }

  /// After a change: the lists that may have changed are read again.
  fn sb_walls_reload(&mut self) {
    let w = &mut self.sidebar.walls;
    load_info();
    if w.items.contains_key("local") {
      w.items.insert("local".into(), None);
      load_cat("local".into());
    }
    load_live();
    load_saver();
  }

  /// Every second: download progress (the core writes it to Live\.progress).
  pub(in crate::native_bar::sidebar) fn sb_walls_tick(&mut self) {
    let w = &self.sidebar.walls;
    if w.busy.as_ref().is_some_and(|k| k.contains('/')) && self.sidebar.page_open() == Some(crate::native_bar::sidebar::Page::Walls) {
      job(|| {
        let p = core_json(&["--live-progress"]);
        let got = p["got"].as_f64().unwrap_or(0.0);
        let total = p["total"].as_f64().unwrap_or(-1.0);
        WEv::Progress(if total > 0.0 {
          format!("{}%", (got * 100.0 / total).round())
        } else if got > 0.0 {
          format!("{} MB", (got / 1_048_576.0).round())
        } else {
          String::new()
        })
      });
    }
  }

  fn sb_walls_target(&self) -> String {
    self.sidebar.walls.target.clone()
  }

  /// Applies a gallery item where the target says (all, a monitor, span).
  fn sb_walls_apply(&mut self, g: G, t: Tile, mode: Option<String>) {
    let mode = mode.unwrap_or_else(|| if g == G::Span { "span".into() } else { self.sb_walls_target() });
    let w = &mut self.sidebar.walls;
    if w.busy.is_some() {
      return;
    }
    w.busy = Some(t.key.clone());
    w.progress.clear();
    w.msg = None;
    let (ok_wall, ok_live) = (self.model.tr("Duvar kağıdı ayarlandı"), self.model.tr("Canlı duvar kağıdı ayarlandı"));
    let tr_err: Vec<(String, String)> = ["type", "size", "decode", "", "dl"]
      .iter()
      .map(|k| (k.to_string(), self.model.tr(live_error(if *k == "dl" { "" } else { k }, *k == "dl"))))
      .collect();
    match g {
      G::Wall | G::Span => job(move || {
        let r = if !t.path.is_empty() { core_json(&["--wall-set", &t.path, &mode]) } else { core_json(&["--wall-get", &t.full, &mode]) };
        match r["error"].as_str() {
          Some(e) => WEv::Done(false, e.to_string(), false),
          None if r["ok"].as_bool() == Some(true) => WEv::Done(true, ok_wall, true),
          None => WEv::Done(false, tr_err[3].1.clone(), false),
        }
      }),
      G::Live | G::Videos => job(move || {
        let r = core_json(&["--live-set", &t.path, &mode]);
        if r["ok"].as_bool() == Some(true) {
          WEv::Done(true, ok_live, true)
        } else {
          let e = s(&r["error"]).to_string();
          WEv::Done(false, tr_err.iter().find(|(k, _)| *k == e).map_or(tr_err[3].1.clone(), |x| x.1.clone()), false)
        }
      }),
      G::Store | G::SaverStore => job(move || {
        let r = core_json(&["--live-get", &t.cat, &t.id, &mode]);
        if r["ok"].as_bool() == Some(true) {
          WEv::Done(true, ok_live, true)
        } else {
          let e = s(&r["error"]).to_string();
          WEv::Done(false, tr_err.iter().find(|(k, _)| *k == e && !k.is_empty()).map_or(tr_err[4].1.clone(), |x| x.1.clone()), false)
        }
      }),
      G::Savers => {}
    }
  }

  /// The store item into the library only.
  fn sb_walls_download(&mut self, g: G, t: Tile) {
    let w = &mut self.sidebar.walls;
    if w.busy.is_some() {
      return;
    }
    w.busy = Some(t.key.clone());
    let ok = self.model.tr("Kütüphaneye indirildi");
    let fail = self.model.tr("İndirilemedi");
    job(move || {
      let r = if g == G::Wall { core_json(&["--wall-download", &t.full]) } else { core_json(&["--live-get", &t.cat, &t.id, "none"]) };
      if r["ok"].as_bool() == Some(true) {
        WEv::Done(true, ok, true)
      } else {
        WEv::Done(false, fail, false)
      }
    });
  }

  /// Our video screen saver: set (only this), add, remove.
  fn sb_saver_video(&mut self, op: &'static str, path: String) {
    self.sidebar.walls.busy = Some(path.clone());
    job(move || WEv::Videos(core_json(&["--saver-video", op, &path])));
  }

  fn sb_saver_store(&mut self, t: Tile) {
    let w = &mut self.sidebar.walls;
    if w.busy.is_some() {
      return;
    }
    w.busy = Some(t.key.clone());
    job(move || {
      let v = core_json(&["--saver-store-get", &t.cat, &t.id]);
      WEv::Videos(v)
    });
    load_live();
  }

  /// Windows' screen saver settings: written at once, the real state read back.
  fn sb_saver_set(&mut self, enabled: Option<bool>, secure: Option<bool>, minutes: Option<u32>, selected: Option<String>) {
    let Some(st_) = self.sidebar.walls.saver.clone() else { return };
    let mut enabled = enabled.unwrap_or(st_["enabled"].as_bool() == Some(true));
    let secure = secure.unwrap_or(st_["secure"].as_bool() == Some(true));
    let minutes = minutes.unwrap_or(st_["minutes"].as_u64().unwrap_or(1) as u32).clamp(1, 120);
    let mut selected = selected.unwrap_or_else(|| s(&st_["selected"]).to_string());
    if enabled && selected.is_empty() {
      selected = st_["choices"][0]["path"].as_str().unwrap_or("").to_string();
      enabled = !selected.is_empty();
    }
    let w = &mut self.sidebar.walls;
    w.saving = true;
    w.saver_err.clear();
    job(move || {
      WEv::Saver(match crate::screensaver::set(enabled, minutes, secure, &selected) {
        Ok(s) => serde_json::to_value(s).map_err(|e| e.to_string()),
        Err(e) => Err(e),
      })
    });
    // shown at once; the worker's answer is the truth
    if let Some(v) = self.sidebar.walls.saver.as_mut() {
      v["secure"] = json!(secure);
      v["minutes"] = json!(minutes);
    }
  }

  fn sb_saver_run(&mut self, path: String, configure: bool) {
    job(move || match crate::screensaver::run(&path, configure) {
      Ok(()) => WEv::Progress(String::new()),
      Err(e) => WEv::Done(false, e, false),
    });
  }

  /// A file dialog of the core (pick / import); the panel stays open meanwhile.
  fn sb_walls_pick(&mut self, g: G) {
    self.sb_modal(true);
    let target = if g == G::Span { "span".to_string() } else { self.sb_walls_target() };
    let (fail, noscr, size) = (self.model.tr("İçe aktarılamadı"), self.model.tr("Pakette ekran koruyucu (.scr) yok"), self.model.tr("Paket çok büyük"));
    let added = self.model.tr("$1 ekran koruyucu eklendi");
    let ok_live = self.model.tr("Canlı duvar kağıdı ayarlandı");
    let live_errs: Vec<(String, String)> = ["type", "size", "decode"].iter().map(|k| (k.to_string(), self.model.tr(live_error(k, false)))).collect();
    let set_fail = self.model.tr(live_error("", false));
    std::thread::spawn(move || {
      let r = match g {
        G::Wall | G::Span => {
          let _ = core_api::run_core_output(&["--wall-pick", &target]);
          WEv::Done(true, String::new(), true)
        }
        G::Live => {
          let v = core_json(&["--live-pick", &target]);
          match &v {
            Value::String(p) if !p.is_empty() => WEv::Done(true, ok_live, true),
            _ if v["error"].is_string() => {
              let e = s(&v["error"]).to_string();
              WEv::Done(false, live_errs.iter().find(|(k, _)| *k == e).map_or(set_fail, |x| x.1.clone()), false)
            }
            _ => WEv::Progress(String::new()),
          }
        }
        _ => {
          let v = core_json(&["--saver-pick"]);
          match v["error"].as_str() {
            Some("noscr") => WEv::Done(false, noscr, false),
            Some("size") => WEv::Done(false, size, false),
            Some(_) => WEv::Done(false, fail, false),
            None => {
              let n = v["added"].as_array().map_or(0, |a| a.len());
              if n > 0 {
                WEv::Done(true, added.replace("$1", &n.to_string()), true)
              } else {
                WEv::Progress(String::new())
              }
            }
          }
        }
      };
      send(Msg::Sidebar(Ev::Walls(r)));
      send(Msg::Sidebar(Ev::Walls(WEv::Progress("\u{0}modal".into()))));
    });
  }

  pub(in crate::native_bar::sidebar) fn sb_walls_click(&mut self, h: WHit, button: u8, x: f32, y: f32) {
    if button == 1 {
      if let WHit::Tile(g, key) = &h {
        return self.sb_walls_menu(*g, key.clone(), x, y);
      }
      return;
    }
    if button != 0 {
      return;
    }
    let w = &mut self.sidebar.walls;
    if w.confirm.is_some() && !matches!(h, WHit::Confirm(_)) {
      return;
    }
    match h {
      WHit::Tab(i) => {
        if w.tab != i {
          w.tab = i;
          w.msg = None;
          self.sidebar.store.wall_tab = i;
          self.sidebar.save_soon();
          self.sidebar.scroll.remove(&ScrollId::Page);
          self.sb_walls_load_tab();
        }
      }
      WHit::Sub(i) => {
        if w.sub != i {
          w.sub = i;
          self.sb_walls_load_tab();
        }
      }
      WHit::Mon(id) => w.target = if w.target == id { "all".into() } else { id },
      WHit::Target(id) => w.target = id,
      WHit::Cat(c) => {
        if w.cat != c {
          w.cat = c.to_string();
          if c == "local" || !w.items.contains_key(c) {
            w.items.insert(c.to_string(), None);
            load_cat(c.to_string());
          }
          self.sidebar.store.wall_cat = Some(c.to_string());
          self.sidebar.save_soon();
        }
      }
      WHit::StoreCat(c) => {
        w.store_cat = c.clone();
        self.sidebar.store.live_cat = Some(c);
        self.sidebar.save_soon();
        self.sb_walls_store();
      }
      WHit::Tile(g, key) => {
        let Some(t) = w.tile(g, &key) else { return };
        match g {
          G::Savers => self.sb_saver_set(Some(true), None, None, Some(t.path)),
          G::Videos => self.sb_saver_video("set", t.path),
          G::SaverStore => self.sb_saver_store(t),
          _ => self.sb_walls_apply(g, t, None),
        }
      }
      WHit::Custom(g) => self.sb_walls_pick(g),
      WHit::LiveClear => {
        let target = self.sb_walls_target();
        let (ok, fail) = (self.model.tr("Canlı duvar kağıdı kapatıldı"), self.model.tr(live_error("", false)));
        job(move || {
          let r = core_json(&["--live-clear", &target]);
          if r["ok"].as_bool() == Some(true) {
            WEv::Done(true, ok, true)
          } else {
            WEv::Done(false, fail, false)
          }
        });
      }
      WHit::LiveOpt(fullscreen) => {
        let opts = w.info.as_ref().map(|i| i["liveOptions"].clone()).unwrap_or(Value::Null);
        let mut fs = opts["pauseFullscreen"].as_bool() != Some(false);
        let mut bat = opts["pauseOnBattery"].as_bool() != Some(false);
        if fullscreen {
          fs = !fs;
        } else {
          bat = !bat;
        }
        if let Some(i) = w.info.as_mut() {
          i["liveOptions"] = json!({ "pauseFullscreen": fs, "pauseOnBattery": bat });
        }
        let fail = self.model.tr("Ayar kaydedilemedi");
        job(move || {
          let r = core_json(&["--live-options", if fs { "1" } else { "0" }, if bat { "1" } else { "0" }]);
          if r["ok"].as_bool() == Some(true) {
            WEv::Info(core_json(&["--wall-info"]))
          } else {
            WEv::Done(false, fail, true)
          }
        });
      }
      WHit::SaverEnabled => {
        let on = w.saver.as_ref().is_some_and(|v| v["enabled"].as_bool() == Some(true));
        self.sb_saver_set(Some(!on), None, None, None);
      }
      WHit::SaverSecure => {
        let on = w.saver.as_ref().is_some_and(|v| v["secure"].as_bool() == Some(true));
        self.sb_saver_set(None, Some(!on), None, None);
      }
      WHit::SaverPreview | WHit::SaverOptions => {
        let sel = w.saver.as_ref().map(|v| s(&v["selected"]).to_string()).unwrap_or_default();
        if !sel.is_empty() {
          self.sb_saver_run(sel, h == WHit::SaverOptions);
        }
      }
      WHit::Shuffle => {
        let on = w.videos["shuffle"].as_bool() == Some(true);
        job(move || WEv::Videos(core_json(&["--saver-shuffle", if on { "0" } else { "1" }])));
      }
      WHit::Confirm(yes) => {
        if let Some(c) = w.confirm.take() {
          if yes {
            self.sb_walls_remove(c.g, c.path);
          }
        }
      }
    }
    self.sb_render();
  }

  /// The right-click menu of a gallery item (our native menu).
  fn sb_walls_menu(&mut self, g: G, key: String, x: f32, y: f32) {
    let Some(t) = self.sidebar.walls.tile(g, &key) else { return };
    let tr = |s: &str| self.model.tr(s);
    let mons = self.sidebar.walls.monitors();
    let per_monitor: Vec<MenuItem> = mons
      .iter()
      .enumerate()
      .map(|(i, m)| MenuItem::new(&format!("mon:{}", s(&m["id"])), Some("desktop_windows"), tr(&format!("Monitör {}", i + 1))))
      .collect();
    let local = !t.path.is_empty();
    let mut items = Vec::new();
    match g {
      G::Wall | G::Span | G::Live => {
        if !local && g != G::Live {
          items.push(MenuItem::new("download", Some("download"), tr("İndir")));
        }
        items.push(MenuItem::new("apply", Some("check"), tr(if g == G::Span { "Uygula" } else { "Uygula (tüm monitörler)" })));
        if mons.len() > 1 && g != G::Span {
          items.push(MenuItem::new("here", Some("desktop_windows"), tr("Bu monitöre uygula")).submenu(per_monitor));
        }
        if g == G::Live {
          items.push(MenuItem::new("saver", Some("ambient_screen"), tr("Ekran koruyucu yap")));
        }
        if local {
          items.push(MenuItem::sep());
          items.push(MenuItem::new("folder", Some("folder_open"), tr("Dosya konumunu aç")));
          items.push(MenuItem::new("remove", Some("delete"), tr("Kütüphaneden kaldır")));
        }
      }
      G::Store | G::SaverStore => {
        items.push(MenuItem::new("download", Some("download"), tr("İndir")));
        items.push(MenuItem::new("apply", Some("check"), tr("Uygula (tüm monitörler)")));
        if mons.len() > 1 {
          items.push(MenuItem::new("here", Some("desktop_windows"), tr("Bu monitöre uygula")).submenu(per_monitor));
        }
        items.push(MenuItem::new("saver", Some("ambient_screen"), tr("Ekran koruyucu yap")));
      }
      G::Savers => {
        let imported = core_api::core_exe().is_some() && is_imported(&t.path);
        items.push(MenuItem::new("preview", Some("play_arrow"), tr("Önizle")));
        items.push(MenuItem::new("options", Some("tune"), tr("Ayarlar")));
        items.push(MenuItem::new("choose", Some("check"), tr("Seç")));
        if imported {
          items.push(MenuItem::sep());
          items.push(MenuItem::new("remove", Some("delete"), tr("Kaldır")));
        }
      }
      G::Videos => {
        let listed = self.sidebar.walls.in_videos(&t.path);
        items.push(MenuItem::new("vset", Some("ambient_screen"), tr("Ekran koruyucu yap")));
        items.push(MenuItem::new("vadd", Some("playlist_add"), tr("Listeye ekle")).enabled(!listed));
        items.push(MenuItem::new("vremove", Some("playlist_remove"), tr("Listeden çıkar")).enabled(listed));
        items.push(MenuItem::new("vrun", Some("play_arrow"), tr("Şimdi göster")));
      }
    }
    self.sb_menu(x, y, items, move |ui: &mut Ui, id: &str| ui.sb_walls_menu_pick(g, t, id));
  }

  fn sb_walls_menu_pick(&mut self, g: G, t: Tile, id: &str) {
    match id {
      "apply" => self.sb_walls_apply(g, t, Some(if g == G::Span { "span".into() } else { "all".into() })),
      "download" => self.sb_walls_download(if g == G::Wall || g == G::Span { G::Wall } else { G::Store }, t),
      "saver" => {
        if g == G::Live {
          self.sb_saver_video("set", t.path);
        } else {
          self.sb_saver_store(t);
        }
      }
      "folder" => {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{}", t.path)).spawn();
      }
      "remove" => {
        let text = if g == G::Savers { t.name.clone() } else if t.name.is_empty() { file_name(&t.path) } else { t.name.clone() };
        self.sidebar.walls.confirm = Some(Confirm { text, g, path: t.path });
      }
      "preview" => self.sb_saver_run(t.path, false),
      "options" => self.sb_saver_run(t.path, true),
      "choose" => self.sb_saver_set(Some(true), None, None, Some(t.path)),
      "vset" => self.sb_saver_video("set", t.path),
      "vadd" => self.sb_saver_video("add", t.path),
      "vremove" => self.sb_saver_video("remove", t.path),
      "vrun" => job(|| {
        let r = core_json(&["--saver-video-run"]);
        if r["ok"].as_bool() == Some(true) {
          WEv::Progress(String::new())
        } else {
          WEv::Done(false, s(&r["error"]).to_string(), false)
        }
      }),
      other => {
        if let Some(mon) = other.strip_prefix("mon:") {
          self.sb_walls_apply(g, t, Some(mon.to_string()));
        }
      }
    }
    self.sb_render();
  }

  /// Removes a library item through the core (it checks the path).
  fn sb_walls_remove(&mut self, g: G, path: String) {
    let kind = match g {
      G::Wall | G::Span => "wall",
      G::Live | G::Videos => "live",
      G::Savers => "saver",
      _ => return,
    };
    let selected = self.sidebar.walls.saver.as_ref().map(|v| s(&v["selected"]).to_string()).unwrap_or_default();
    let (minutes, secure) = self.sidebar.walls.saver.as_ref().map_or((10, false), |v| (v["minutes"].as_u64().unwrap_or(10) as u32, v["secure"].as_bool() == Some(true)));
    job(move || {
      if kind == "live" {
        // out of the video screen saver's list first
        let _ = core_api::run_core_output(&["--saver-video", "remove", &path]);
      }
      let ok = matches!(core_api::post(&format!("/library-remove?kind={}&path={}", kind, enc(&path))), Some((204, _)));
      if ok && kind == "saver" && selected.eq_ignore_ascii_case(&path) {
        // the removed one was Windows' screen saver: it is turned off
        let _ = crate::screensaver::set(false, minutes, secure, "");
      }
      WEv::Removed(g, path, ok)
    });
  }

  pub(in crate::native_bar::sidebar) fn sb_walls_typed(&mut self, id: FieldId, t: Typed) {
    if id != FieldId::SaverMinutes {
      return;
    }
    // digits only, saved on Enter or when the field is left
    let f = self.sidebar.field(id);
    let digits: String = f.text().chars().filter(|c| c.is_ascii_digit()).collect();
    if digits != f.text() {
      f.set(&digits);
    }
    if t == Typed::Submit {
      let n = digits.parse::<u32>().unwrap_or(1).clamp(1, 120);
      self.sidebar.field(id).set(&n.to_string());
      self.sb_saver_set(None, None, Some(n), None);
      if self.sidebar.focus == Some(id) {
        self.sidebar.focus = None;
      }
    }
  }

  /// Esc on the page: the confirmation goes first.
  pub(in crate::native_bar::sidebar) fn sb_walls_escape(&mut self) -> bool {
    self.sidebar.walls.confirm.take().is_some()
  }

  /// Hover over a tile: when it changed (moving previews start from their first frame).
  pub(in crate::native_bar::sidebar) fn sb_walls_hover(&mut self, hit: Option<&Hit>) {
    let key = match hit {
      Some(Hit::Walls(WHit::Tile(_, k))) => Some(k.clone()),
      _ => None,
    };
    let w = &mut self.sidebar.walls;
    if w.hover.as_ref().map(|h| &h.0) != key.as_ref() {
      w.hover = key.map(|k| (k, Instant::now()));
      // a moving preview's frames are large: only the one under the pointer stays
      if let Some(old) = w.last_gif.take() {
        self.sidebar.images.forget(&old);
      }
    }
  }
}
