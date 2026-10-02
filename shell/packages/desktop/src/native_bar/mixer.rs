//! Per-app volume, like EarTrumpet: the audio sessions of the default output
//! device, grouped by app (a browser has several). Read when the mixer opens
//! and on its tick while it is open; nothing runs while it is closed. The
//! device's own volume stays with the audio provider (the bar's wheel).

use windows::{
  core::{Interface, GUID},
  Win32::{
    Foundation::S_OK,
    Media::Audio::{
      eConsole, eRender, AudioSessionStateExpired, Endpoints::IAudioMeterInformation, IAudioSessionControl2,
      IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator,
    },
    System::Com::{CoCreateInstance, CLSCTX_ALL},
  },
};

use crate::common::windows::process_image_path;

/// One app's sessions, set together.
pub struct App {
  /// exe path; None for Windows' system sounds
  pub exe: Option<String>,
  /// 0..1
  pub volume: f32,
  pub muted: bool,
  /// current level 0..1 (the loudest session)
  pub peak: f32,
  sessions: Vec<(ISimpleAudioVolume, Option<IAudioMeterInformation>)>,
}

impl App {
  /// exe name without extension, lower case ("firefox"); "" for system sounds
  pub fn process(&self) -> String {
    self
      .exe
      .as_deref()
      .and_then(|e| std::path::Path::new(e).file_stem())
      .map(|s| s.to_string_lossy().to_lowercase())
      .unwrap_or_default()
  }

  fn read(&mut self) {
    if let Some((first, _)) = self.sessions.first() {
      unsafe {
        self.volume = first.GetMasterVolume().unwrap_or(self.volume);
        self.muted = first.GetMute().map(|m| m.as_bool()).unwrap_or(self.muted);
      }
    }
    self.peak = self
      .sessions
      .iter()
      .filter_map(|(_, meter)| meter.as_ref().and_then(|m| unsafe { m.GetPeakValue() }.ok()))
      .fold(0.0, f32::max);
  }
}

pub struct Mixer {
  manager: IAudioSessionManager2,
  pub apps: Vec<App>,
}

impl Mixer {
  /// The default output device's sessions. COM is already up on the bar's
  /// thread (OleInitialize for drag and drop).
  pub fn open() -> windows::core::Result<Self> {
    let manager: IAudioSessionManager2 = unsafe {
      let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
      enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)?
    };
    let mut mixer = Self { manager, apps: Vec::new() };
    mixer.refresh()?;
    Ok(mixer)
  }

  /// Sessions again (apps that started or stopped playing), with their
  /// volumes. Apps keep their order; new ones are added at the end, the
  /// system sounds stay last.
  pub fn refresh(&mut self) -> windows::core::Result<()> {
    let list = unsafe { self.manager.GetSessionEnumerator()? };
    let count = unsafe { list.GetCount()? };
    let mut apps: Vec<App> = Vec::new();
    for i in 0..count {
      let Ok(control) = (unsafe { list.GetSession(i) }) else { continue };
      if unsafe { control.GetState() }.is_ok_and(|s| s == AudioSessionStateExpired) {
        continue;
      }
      let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
      let system = unsafe { control2.IsSystemSoundsSession() } == S_OK;
      let exe = if system { None } else { unsafe { control2.GetProcessId() }.ok().and_then(process_image_path) };
      if !system && exe.is_none() {
        continue; // the process is gone
      }
      let Ok(volume) = control.cast::<ISimpleAudioVolume>() else { continue };
      let meter = control.cast::<IAudioMeterInformation>().ok();
      let key = exe.as_deref().map(str::to_lowercase);
      match apps.iter_mut().find(|a| a.exe.as_deref().map(str::to_lowercase) == key) {
        Some(app) => app.sessions.push((volume, meter)),
        None => apps.push(App { exe, volume: 1.0, muted: false, peak: 0.0, sessions: vec![(volume, meter)] }),
      }
    }
    let rank = |a: &App, old: &[App]| {
      if a.exe.is_none() {
        usize::MAX
      } else {
        old.iter().position(|o| o.exe == a.exe).unwrap_or(usize::MAX - 1)
      }
    };
    apps.sort_by_key(|a| rank(a, &self.apps));
    for app in &mut apps {
      app.read();
    }
    self.apps = apps;
    Ok(())
  }

  /// Volumes and levels of the known sessions (the tick between refreshes).
  pub fn read(&mut self) {
    for app in &mut self.apps {
      app.read();
    }
  }

  pub fn set_volume(&mut self, i: usize, v: f32) {
    let v = v.clamp(0.0, 1.0);
    let unmute = match self.apps.get_mut(i) {
      Some(app) => {
        for (session, _) in &app.sessions {
          let _ = unsafe { session.SetMasterVolume(v, &GUID::zeroed()) };
        }
        app.volume = v;
        app.muted && v > 0.0
      }
      None => return,
    };
    // moving the slider of a muted app unmutes it (as EarTrumpet)
    if unmute {
      self.set_mute(i, false);
    }
  }

  pub fn set_mute(&mut self, i: usize, mute: bool) {
    let Some(app) = self.apps.get_mut(i) else { return };
    for (session, _) in &app.sessions {
      let _ = unsafe { session.SetMute(mute, &GUID::zeroed()) };
    }
    app.muted = mute;
  }
}
