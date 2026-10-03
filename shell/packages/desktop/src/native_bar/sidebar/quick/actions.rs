//! What the quick settings' tiles do and what the workers bring back.

use super::*;

impl Ui {
  /// Reads every tile's state (at most every 30 s unless `force`).
  pub(in crate::native_bar::sidebar) fn sb_qs_refresh(&mut self, force: bool) {
    let q = &mut self.sidebar.quick;
    if !force && q.last_refresh.is_some_and(|t| t.elapsed() < REFRESH_EVERY) {
      return;
    }
    q.last_refresh = Some(Instant::now());
    spawn(|| {
      if let Some(v) = qs("radios") {
        ev(QEv::Radios(v));
      }
    });
    spawn(|| {
      if let Some(v) = qs("eth") {
        ev(QEv::Eth(v));
      }
    });
    spawn(|| {
      if let Some(v) = qs("bt") {
        ev(QEv::Bt(v));
      }
    });
    spawn(|| {
      if let Some(v) = core_json(&["--nightlight", "status"]) {
        ev(QEv::Night(v));
      }
    });
    spawn(|| {
      if let Some(v) = qs("status") {
        ev(QEv::Status(v));
      }
    });
    spawn(|| {
      let v = core_json(&["--mic", "status"]);
      ev(QEv::Mic(v.and_then(|v| v["muted"].as_bool()).map(|m| !m)));
    });
  }

  pub(in crate::native_bar::sidebar) fn sb_quick_event(&mut self, e: QEv) {
    let q = &mut self.sidebar.quick;
    match e {
      QEv::Radios(v) => q.hw.radios = v,
      QEv::Eth(v) => q.hw.eth = v,
      QEv::Bt(v) => q.hw.bt = v,
      QEv::Night(v) => {
        if v.is_object() {
          q.hw.night = v;
          q.night_level = None;
        }
      }
      QEv::Status(v) => {
        let mut awake = v["awake"].as_bool() == Some(true);
        // after a restart: keep-awake that was left on comes back
        if !awake && self.sidebar.store.awake_want && !q.awake_restarted {
          q.awake_restarted = true;
          core_api::post_async("/qs/awake?v=1".into());
          awake = true;
        }
        q.hw.awake = awake;
      }
      QEv::Mic(v) => {
        if v.is_some() {
          q.hw.mic = v;
        }
      }
      QEv::Wifi(v) => {
        q.wifi_scanning = false;
        q.wifi_scanned = Some(Instant::now());
        if let Some(v) = v.filter(|v| v["networks"].is_array()) {
          q.wifi = Some(v);
        }
      }
      QEv::WifiConnected(ssid, r) => {
        q.wifi_busy = None;
        if r["needPassword"].as_bool() == Some(true) {
          q.wifi_ask = Some(ssid);
          self.sidebar.field(FieldId::WifiPw).set("");
          self.sidebar.focus = Some(FieldId::WifiPw);
        } else if r["ok"].as_bool() != Some(true) {
          q.wifi_err = r["error"].as_str().map(str::to_string).unwrap_or_else(|| self.model.tr("Bağlanılamadı"));
        } else {
          q.wifi_ask = None;
          self.sb_wifi_scan();
        }
      }
      QEv::EthToggled(r) => {
        if r["ok"].as_bool() == Some(false) {
          let body = self.model.tr("Yönetici görevi bulunamadı: scripts\\setup-eth-tasks.ps1 bir kez yönetici olarak çalıştırılmalı.");
          self.toast_add(json!({ "kind": "error", "title": "Ethernet", "body": body, "icon": "lan" }));
        }
        let q = &mut self.sidebar.quick;
        q.last_refresh = None;
        std::thread::spawn(|| {
          std::thread::sleep(Duration::from_millis(2500));
          if let Some(v) = qs("eth") {
            ev(QEv::Eth(v));
          }
        });
      }
      QEv::AudioDefault(id) => {
        if q.audio_busy.as_deref() == Some(id.as_str()) {
          q.audio_busy = None;
        }
      }
      QEv::ScanDone => q.scanning = false,
    }
    self.sidebar.store.qs_cache = self.sidebar.quick.cache();
    self.sidebar.save_soon();
    self.sb_render();
  }

  /// Every second while open: the Wi-Fi list again every 10 s while its card shows.
  pub(in crate::native_bar::sidebar) fn sb_quick_tick(&mut self) {
    let q = &self.sidebar.quick;
    if q.menu == Some(Tile::Wifi) && q.wifi_on() && !q.wifi_scanning && q.wifi_scanned.map_or(true, |t| t.elapsed() >= Duration::from_secs(10)) {
      self.sb_wifi_scan();
    }
  }

  pub(in crate::native_bar::sidebar) fn sb_audio_out(&self) -> Option<crate::providers::audio::AudioDevice> {
    self.model.audio.as_ref().and_then(|a| a.default_playback_device.clone())
  }

  pub(in crate::native_bar::sidebar) fn sb_toast(&mut self, title: &str, body: &str, icon: &str) {
    let (t, b) = (self.model.tr(title), self.model.tr(body));
    self.toast_add(json!({ "kind": "error", "title": t, "body": b, "icon": icon }));
  }

  /// The tile's main action (sidebar.html `def.action`).
  pub(in crate::native_bar::sidebar) fn sb_tile_action(&mut self, tile: Tile) {
    let q = &mut self.sidebar.quick;
    match tile {
      Tile::Wifi => {
        let next = if q.wifi_on() { "Off" } else { "On" };
        q.hw.radios["wifi"] = json!(next);
        spawn(move || {
          let _ = qs(&format!("radio?kind=wifi&state={next}"));
          if let Some(v) = qs("radios") {
            ev(QEv::Radios(v));
          }
        });
      }
      Tile::Ethernet => {
        let disabled = s(&q.hw.eth["state"]) == "disabled";
        if q.hw.eth.is_object() {
          q.hw.eth["state"] = json!(if disabled { "up" } else { "disabled" });
        }
        spawn(|| {
          if let Some(v) = qs("eth-toggle") {
            ev(QEv::EthToggled(v));
          }
        });
      }
      Tile::Bluetooth => {
        if q.bt_no_adapter() {
          self.sb_toast("Bluetooth", "Bluetooth adaptörü bulunamadı.", "bluetooth_disabled");
          return;
        }
        let next = if q.bt_on() { "Off" } else { "On" };
        q.hw.radios["bluetooth"] = json!(next);
        spawn(move || {
          let _ = qs(&format!("radio?kind=bluetooth&state={next}"));
          if let Some(v) = qs("radios") {
            ev(QEv::Radios(v));
          }
          if let Some(v) = qs("bt") {
            ev(QEv::Bt(v));
          }
        });
      }
      Tile::IdleInhibitor => {
        let next = !q.hw.awake;
        q.hw.awake = next;
        self.sidebar.store.awake_want = next;
        self.sidebar.save_soon();
        core_api::post_async(format!("/qs/awake?v={}", next as u8));
      }
      Tile::Mic => {
        q.hw.mic = Some(q.hw.mic == Some(false));
        spawn(|| {
          let v = core_json(&["--mic", "toggle"]);
          ev(QEv::Mic(v.and_then(|v| v["muted"].as_bool()).map(|m| !m)));
        });
      }
      Tile::Audio => {
        if let Some(d) = self.sb_audio_out() {
          self.provider("audio", ProviderFunction::Audio(AudioFunction::SetMute(SetMuteArgs { mute: !d.is_muted, device_id: Some(d.device_id) })));
        }
      }
      Tile::NightLight => {
        let on = q.hw.night["on"].as_bool() == Some(true);
        if q.hw.night.is_object() {
          q.hw.night["on"] = json!(!on);
        } else {
          q.hw.night = json!({ "on": !on });
        }
        spawn(|| {
          if let Some(v) = core_json(&["--nightlight", "toggle"]) {
            ev(QEv::Night(v));
          }
        });
      }
      Tile::DarkMode => {
        let light = !self.model.light;
        self.set_light(light);
        core_api::set_pref("theme", if light { "light" } else { "dark" });
      }
      Tile::ScreenSnip => {
        self.sidebar_close();
        core_api::run_core(&["--snip", "350"]);
      }
      Tile::OnScreenKeyboard => crate::bus::publish(crate::bus::Event::OskToggle),
      Tile::Notifications => self.sb_set_dnd(!self.model.dnd),
    }
    self.sb_render();
  }

  pub(in crate::native_bar::sidebar) fn sb_set_dnd(&mut self, dnd: bool) {
    self.model.dnd = dnd;
    core_api::set_pref("dnd", if dnd { "true" } else { "false" });
    self.sb_render();
  }

  pub(in crate::native_bar::sidebar) fn sb_tile_has_menu(&self, tile: Tile) -> bool {
    matches!(tile, Tile::Wifi | Tile::Ethernet | Tile::Bluetooth | Tile::Mic | Tile::Audio | Tile::NightLight)
  }

  /// Tile menus as sidebar.html's `def.menu` (Bluetooth without adapter warns).
  pub(in crate::native_bar::sidebar) fn sb_tile_menu(&mut self, tile: Tile) {
    if tile == Tile::Bluetooth && self.sidebar.quick.bt_no_adapter() {
      self.sb_toast("Bluetooth", "Bluetooth adaptörü bulunamadı.", "bluetooth_disabled");
      return;
    }
    self.sb_toggle_menu(tile);
  }

  /// button: 0 left, 1 right, 2 middle
  pub(in crate::native_bar::sidebar) fn sb_quick_click(&mut self, h: QHit, button: u8) {
    let edit = self.sidebar.quick.edit;
    match (h, button) {
      (QHit::Tile(t), 0) if edit => {
        if self.sidebar.quick.take_click() {
          self.sidebar.quick.edit_click(t);
          self.sb_quick_changed();
        }
      }
      (QHit::Tile(t), 1) if edit => {
        self.sidebar.quick.edit_size(t);
        self.sb_quick_changed();
      }
      (QHit::Tile(t), 0) => {
        let size = self.sidebar.quick.toggles.iter().find(|x| x.tile == t).map_or(1, |x| x.size);
        if size == 2 && self.sb_tile_has_menu(t) {
          self.sb_tile_menu(t);
        } else {
          self.sb_tile_action(t);
        }
      }
      (QHit::Tile(t), 1) if self.sb_tile_has_menu(t) => self.sb_tile_menu(t),
      (QHit::TileIcon(t), 0) => self.sb_tile_action(t),
      (QHit::Edit, 0) => {
        let q = &mut self.sidebar.quick;
        q.edit = !q.edit;
        q.set_menu(None, 0.0);
      }
      (QHit::Net(ssid), 0) => self.sb_wifi_row(ssid),
      (QHit::NetSubmit, 0) => self.sb_wifi_submit(),
      (QHit::Scan, 0) => self.sb_scan(),
      (QHit::More(uri), 0) => {
        let _ = std::process::Command::new("explorer").arg(uri).spawn();
      }
      (QHit::AudioMute, 0) => {
        let out = self.sidebar.quick.menu == Some(Tile::Audio);
        if let Some(d) = audio_devices(&self.model, out).into_iter().find(|d| if out { d.is_default_playback } else { d.is_default_recording }) {
          self.provider("audio", ProviderFunction::Audio(AudioFunction::SetMute(SetMuteArgs { mute: !d.is_muted, device_id: Some(d.device_id) })));
        }
      }
      (QHit::AudioDev(id), 0) => {
        let out = self.sidebar.quick.menu == Some(Tile::Audio);
        let is_def = audio_devices(&self.model, out).iter().any(|d| d.device_id == id && if out { d.is_default_playback } else { d.is_default_recording });
        if !is_def {
          self.sidebar.quick.audio_busy = Some(id.clone());
          spawn(move || {
            let _ = core_api::run_core_output(&["--audio-default", &id]);
            std::thread::sleep(Duration::from_millis(400));
            ev(QEv::AudioDefault(id));
          });
        }
      }
      (QHit::NightToggle, 0) => {
        spawn(|| {
          if let Some(v) = core_json(&["--nightlight", "toggle"]) {
            ev(QEv::Night(v));
          }
        });
      }
      (QHit::NightMode(k), 0) => self.sb_night_set("mode", k.to_string()),
      (QHit::NightPreset(v), 0) => {
        self.sidebar.quick.night_level = Some(v);
        self.sb_night_set("level", v.to_string());
      }
      _ => {}
    }
    self.sb_render();
  }

  pub(in crate::native_bar::sidebar) fn sb_quick_changed(&mut self) {
    self.sidebar.store.quick_toggles = Some(self.sidebar.quick.toggles.clone());
    self.sidebar.save_soon();
  }

}
