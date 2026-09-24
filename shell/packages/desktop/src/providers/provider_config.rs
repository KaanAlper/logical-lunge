use serde::Deserialize;

#[cfg(windows)]
use super::{
  audio::AudioProviderConfig, media::MediaProviderConfig,
  systray::SystrayProviderConfig,
};
use super::{
  battery::BatteryProviderConfig, cpu::CpuProviderConfig,
  host::HostProviderConfig, memory::MemoryProviderConfig,
  network::NetworkProviderConfig,
};

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
  #[cfg(windows)]
  Audio(AudioProviderConfig),
  Battery(BatteryProviderConfig),
  Cpu(CpuProviderConfig),
  Host(HostProviderConfig),
  #[cfg(windows)]
  Media(MediaProviderConfig),
  Memory(MemoryProviderConfig),
  Network(NetworkProviderConfig),
  #[cfg(windows)]
  Systray(SystrayProviderConfig),
}