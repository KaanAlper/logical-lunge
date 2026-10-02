//! What each tile shows: its name, icon, on / off, state line (sidebar.html
//! `defs`).

use super::*;

impl Quick {
  pub(super) fn def(&self, tile: Tile, m: &Model, tr: &dyn Fn(&str) -> String) -> Def {
    let audio_out = m.audio.as_ref().and_then(|a| a.default_playback_device.as_ref());
    let ssid = m.network.as_ref().and_then(|n| n.default_gateway.as_ref()).and_then(|g| g.ssid.clone());
    let off = || tr("Kapalı");
    let on = || tr("Açık");
    match tile {
      Tile::Wifi => {
        let wifi = self.wifi_on();
        Def {
          name: "Wi-Fi",
          icon: if !wifi { "wifi_off" } else if ssid.is_some() { "wifi" } else { "wifi_find" },
          toggled: wifi,
          status: if !wifi { off() } else { ssid.unwrap_or_else(|| tr("Bağlı değil")) },
          alt: true,
          // no adapter: not shown at all
          hidden: self.hw.radios["wifi"].is_null(),
          menu: true,
        }
      }
      Tile::Ethernet => {
        let st_ = s(&self.hw.eth["state"]);
        let speed = s(&self.hw.eth["speed"]);
        Def {
          name: "Ethernet",
          icon: if st_ == "disabled" { "settings_ethernet" } else { "lan" },
          toggled: st_ == "up",
          status: if st_ == "disabled" {
            tr("Devre dışı")
          } else if !speed.is_empty() {
            format!("{} · {}", tr("Bağlı"), speed)
          } else {
            tr("Bağlı")
          },
          alt: true,
          // no cable: hidden; disabled stays so it can be switched back on
          hidden: !self.hw.eth.is_object() || st_ == "none" || st_ == "disconnected",
          menu: true,
        }
      }
      Tile::Bluetooth => {
        let none = self.bt_no_adapter();
        let on = self.bt_on();
        let devices = self.hw.bt["devices"].as_array().cloned().unwrap_or_default();
        let connected = devices.iter().find(|d| d["connected"].as_bool() == Some(true));
        Def {
          name: "Bluetooth",
          icon: if on { if connected.is_some() { "bluetooth_connected" } else { "bluetooth" } } else { "bluetooth_disabled" },
          toggled: !none && on,
          status: if none {
            tr("Adaptör yok")
          } else if !on {
            off()
          } else {
            connected.map(|d| s(&d["name"]).to_string()).unwrap_or_else(|| tr("Bağlı değil"))
          },
          alt: true,
          hidden: false,
          menu: true,
        }
      }
      Tile::IdleInhibitor => Def {
        name: "Uyanık tut",
        icon: "coffee",
        toggled: self.hw.awake,
        status: if self.hw.awake { on() } else { off() },
        alt: false,
        hidden: false,
        menu: false,
      },
      Tile::Mic => {
        let off_ = self.hw.mic == Some(false);
        Def {
          name: "Mikrofon",
          icon: if off_ { "mic_off" } else { "mic" },
          toggled: !off_,
          status: if off_ { off() } else { on() },
          alt: false,
          hidden: false,
          menu: true,
        }
      }
      Tile::Audio => {
        let muted = audio_out.is_some_and(|d| d.is_muted);
        Def {
          name: "Ses çıkışı",
          icon: if muted { "volume_off" } else { "volume_up" },
          toggled: !muted,
          status: if muted { tr("Sessiz") } else { tr("Sessiz değil") },
          alt: true,
          hidden: false,
          menu: true,
        }
      }
      Tile::NightLight => {
        let n = &self.hw.night;
        let mode = s(&n["mode"]);
        let sched = match mode {
          "after" => Some(tr(&format!("{} sonrası", s(&n["from"])))),
          "range" => Some(format!("{}–{}", s(&n["from"]), s(&n["to"]))),
          _ => None,
        };
        let is_on = n["on"].as_bool() == Some(true);
        let active = n["active"].as_bool() == Some(true);
        Def {
          name: "Gece Işığı",
          icon: "nightlight",
          toggled: is_on,
          status: if !is_on {
            off()
          } else if let Some(sc) = sched {
            format!("{} · {}", if active { on() } else { tr("Bekliyor") }, sc)
          } else {
            format!("{} · %{}", on(), n["level"].as_i64().unwrap_or(50))
          },
          alt: true,
          hidden: false,
          menu: true,
        }
      }
      Tile::DarkMode => Def {
        name: "Karanlık mod",
        icon: "contrast",
        toggled: !m.light,
        status: if !m.light { on() } else { off() },
        alt: false,
        hidden: false,
        menu: false,
      },
      Tile::ScreenSnip => Def { name: "Ekran alıntısı", icon: "screenshot_region", toggled: false, status: String::new(), alt: false, hidden: false, menu: false },
      Tile::OnScreenKeyboard => Def { name: "Ekran klavyesi", icon: "keyboard", toggled: false, status: String::new(), alt: false, hidden: false, menu: false },
      Tile::Notifications => Def {
        name: "Bildirimler",
        icon: if m.dnd { "notifications_paused" } else { "notifications_active" },
        toggled: !m.dnd,
        status: if m.dnd { tr("Sessiz") } else { on() },
        alt: false,
        hidden: false,
        menu: false,
      },
    }
  }

}
