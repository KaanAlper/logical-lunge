use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use serde::{ser::SerializeStruct, Serialize};
use tokio::{
  sync::{mpsc, oneshot, Mutex},
  task,
};
use tracing::info;

#[cfg(windows)]
use super::{
  audio::AudioProvider, media::MediaProvider, systray::SystrayProvider,
};
use super::{
  battery::BatteryProvider, cpu::CpuProvider, host::HostProvider,
  memory::MemoryProvider, network::NetworkProvider, Provider,
  ProviderConfig, ProviderFunction, ProviderFunctionResponse,
  ProviderFunctionResult, ProviderOutput, RuntimeType,
};

/// Common fields for a provider.
pub struct CommonProviderState {
  /// Wrapper around the sender channel of provider emissions.
  pub emitter: ProviderEmitter,

  /// Wrapper around the receiver channel for incoming inputs to the
  /// provider.
  pub input: ProviderInput,

  /// Shared `sysinfo` instance.
  pub sysinfo: Arc<Mutex<sysinfo::System>>,
}

/// Handle for receiving provider inputs.
pub struct ProviderInput {
  /// Async receiver channel for incoming inputs to the provider.
  pub async_rx: mpsc::Receiver<ProviderInputMsg>,

  /// Sync receiver channel for incoming inputs to the provider.
  pub sync_rx: crossbeam::channel::Receiver<ProviderInputMsg>,
}

pub enum ProviderInputMsg {
  Function(ProviderFunction, oneshot::Sender<ProviderFunctionResult>),
}

/// Handle for sending provider emissions.
#[derive(Clone, Debug)]
pub struct ProviderEmitter {
  /// Sender channel for outgoing provider emissions.
  emit_tx: mpsc::UnboundedSender<ProviderEmission>,

  /// Hash of the provider's config.
  config_hash: String,

  /// Previous emission from the provider.
  prev_emission: Option<ProviderEmission>,
}

impl ProviderEmitter {
  fn emit(&self, emission: ProviderEmission) {
    let send_res = self.emit_tx.send(emission);

    if let Err(err) = send_res {
      tracing::error!("Error sending provider result: {}", err);
    }
  }

  /// Emits an output from a provider.
  pub fn emit_output<T>(&self, output: anyhow::Result<T>)
  where
    T: Into<ProviderOutput>,
  {
    self.emit(ProviderEmission {
      config_hash: self.config_hash.clone(),
      result: output.map(Into::into).map_err(|err| err.to_string()),
    });
  }

  /// Emits an output from a provider and prevents duplicate emissions by
  /// caching the previous emission.
  ///
  /// Note that this won't share the same cache if the `ProviderEmitter`
  /// is cloned.
  pub fn emit_output_cached<T>(&mut self, output: anyhow::Result<T>)
  where
    T: Into<ProviderOutput>,
  {
    let emission = ProviderEmission {
      config_hash: self.config_hash.clone(),
      result: output.map(Into::into).map_err(|err| err.to_string()),
    };

    if self.prev_emission.as_ref() != Some(&emission) {
      self.prev_emission = Some(emission.clone());
      self.emit(emission);
    }
  }
}

/// Emission from a provider.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEmission {
  /// Hash of the provider's config.
  pub config_hash: String,

  /// A thread-safe `Result` type for provider outputs and errors.
  #[serde(serialize_with = "serialize_result")]
  pub result: Result<ProviderOutput, String>,
}

/// Reference to an active provider.
struct ProviderRef {
  /// Sender channel for sending inputs to the provider.
  async_input_tx: mpsc::Sender<ProviderInputMsg>,

  /// Sender channel for sending inputs to the provider.
  sync_input_tx: crossbeam::channel::Sender<ProviderInputMsg>,

  /// Runtime type of the provider.
  runtime_type: RuntimeType,
}

/// Manages the creation and cleanup of providers.
pub struct ProviderManager {
  /// Map of active provider refs.
  provider_refs: Arc<Mutex<HashMap<String, ProviderRef>>>,

  /// Cache of provider emissions.
  emit_cache: Arc<Mutex<HashMap<String, ProviderEmission>>>,

  /// Sender channel for provider emissions.
  emit_tx: mpsc::UnboundedSender<ProviderEmission>,

  /// Shared `sysinfo` instance.
  sysinfo: Arc<Mutex<sysinfo::System>>,
}

impl ProviderManager {
  /// Creates a new provider manager.
  ///
  /// Returns a tuple containing the `ProviderManager` instance and a
  /// channel for provider emissions.
  pub fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<ProviderEmission>) {
    let (emit_tx, emit_rx) = mpsc::unbounded_channel::<ProviderEmission>();

    (
      Arc::new(Self {
        provider_refs: Arc::new(Mutex::new(HashMap::new())),
        emit_cache: Arc::new(Mutex::new(HashMap::new())),
        sysinfo: Arc::new(Mutex::new(sysinfo::System::new_all())),
        emit_tx,
      }),
      emit_rx,
    )
  }

  /// Creates a provider with the given config.
  pub async fn create(
    &self,
    config_hash: String,
    config: ProviderConfig,
  ) -> anyhow::Result<()> {
    // If a provider with the given config already exists, re-emit its
    // latest emission and return early.
    {
      if let Some(found_emit) =
        self.emit_cache.lock().await.get(&config_hash)
      {
        tracing::info!(
          "Emitting cached provider emission for: {}",
          config_hash
        );

        // through the same channel as a fresh emission (the bar listens there)
        self.emit_tx.send(found_emit.clone())?;
        return Ok(());
      };
    }

    // Hold the lock for `provider_refs` to prevent duplicate providers
    // from potentially being created.
    let mut provider_refs = self.provider_refs.lock().await;

    // No-op if the provider has already been created (but has not emitted
    // yet). Several parts of the bar can ask for the same provider; all
    // get its output once it emits.
    if provider_refs.contains_key(&config_hash) {
      return Ok(());
    }

    tracing::info!("Creating provider: {}", config_hash);

    let (async_input_tx, async_input_rx) = mpsc::channel(1);
    let (sync_input_tx, sync_input_rx) = crossbeam::channel::bounded(1);

    let common = CommonProviderState {
      input: ProviderInput {
        async_rx: async_input_rx,
        sync_rx: sync_input_rx,
      },
      emitter: ProviderEmitter {
        emit_tx: self.emit_tx.clone(),
        config_hash: config_hash.clone(),
        prev_emission: None,
      },
      sysinfo: self.sysinfo.clone(),
    };

    // the provider's task runs until its input channel closes
    let (_task, runtime_type) =
      self.create_instance(config, config_hash.clone(), common)?;

    let provider_ref = ProviderRef {
      async_input_tx,
      sync_input_tx,
      runtime_type,
    };

    provider_refs.insert(config_hash, provider_ref);

    Ok(())
  }

  /// Creates a new provider instance.
  fn create_instance(
    &self,
    config: ProviderConfig,
    config_hash: String,
    common: CommonProviderState,
  ) -> anyhow::Result<(task::JoinHandle<()>, RuntimeType)> {
    let runtime_type = match config {
      #[cfg(windows)]
      ProviderConfig::Systray(..) => RuntimeType::Async,
      _ => RuntimeType::Sync,
    };

    // Spawn the provider's task based on its runtime type.
    let task_handle = match &runtime_type {
      RuntimeType::Async => task::spawn(async move {
        match config {
          #[cfg(windows)]
          ProviderConfig::Systray(config) => {
            let mut provider = SystrayProvider::new(config, common);
            provider.start_async().await;
          }
          _ => unreachable!(),
        }

        info!("Provider stopped: {}", config_hash);
      }),
      RuntimeType::Sync => task::spawn_blocking(move || {
        match config {
          #[cfg(windows)]
          ProviderConfig::Audio(config) => {
            let mut provider = AudioProvider::new(config, common);
            provider.start_sync();
          }
          ProviderConfig::Battery(config) => {
            let mut provider = BatteryProvider::new(config, common);
            provider.start_sync();
          }
          ProviderConfig::Cpu(config) => {
            let mut provider = CpuProvider::new(config, common);
            provider.start_sync();
          }
          ProviderConfig::Host(config) => {
            let mut provider = HostProvider::new(config, common);
            provider.start_sync();
          }
          #[cfg(windows)]
          ProviderConfig::Media(config) => {
            let mut provider = MediaProvider::new(config, common);
            provider.start_sync();
          }
          ProviderConfig::Memory(config) => {
            let mut provider = MemoryProvider::new(config, common);
            provider.start_sync();
          }

          ProviderConfig::Network(config) => {
            let mut provider = NetworkProvider::new(config, common);
            provider.start_sync();
          }

          _ => unreachable!(),
        }

        info!("Provider stopped: {}", config_hash);
      }),
    };

    Ok((task_handle, runtime_type))
  }

  /// Sends a function call through a channel to be executed by the
  /// provider.
  ///
  /// Returns the result of the function execution.
  pub async fn call_function(
    &self,
    config_hash: String,
    function: ProviderFunction,
  ) -> anyhow::Result<ProviderFunctionResponse> {
    info!(
      "Calling provider function: {:?} for: {}",
      function, config_hash
    );

    // Only the sender is taken under the lock: one slow call (a hung app's
    // media session) held every other provider operation (the volume wheel,
    // tray clicks, starting a provider) until it answered.
    enum Input {
      Async(mpsc::Sender<ProviderInputMsg>),
      Sync(crossbeam::channel::Sender<ProviderInputMsg>),
    }
    let input = {
      let provider_refs = self.provider_refs.lock().await;
      let provider_ref = provider_refs
        .get(&config_hash)
        .context("No provider found with config.")?;
      match provider_ref.runtime_type {
        RuntimeType::Async => Input::Async(provider_ref.async_input_tx.clone()),
        RuntimeType::Sync => Input::Sync(provider_ref.sync_input_tx.clone()),
      }
    };

    let (tx, rx) = oneshot::channel();
    match input {
      Input::Async(input) => {
        input
          .send(ProviderInputMsg::Function(function, tx))
          .await
          .context("Failed to send function call to provider.")?;
      }
      Input::Sync(input) => {
        input
          .send(ProviderInputMsg::Function(function, tx))
          .context("Failed to send function call to provider.")?;
      }
    }

    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
      .await
      .context("Provider function timed out.")??
      .map_err(anyhow::Error::msg)
  }

  /// Updates the cache with the given provider emission.
  pub async fn update_cache(&self, emission: ProviderEmission) {
    let mut cache = self.emit_cache.lock().await;
    cache.insert(emission.config_hash.clone(), emission);
  }
}

/// Custom serializer for Result<ProviderOutput, String> that converts:
/// - Ok(output) -> {"output": output}
/// - Err(error) -> {"error": error}
fn serialize_result<S>(
  result: &Result<ProviderOutput, String>,
  serializer: S,
) -> Result<S::Ok, S::Error>
where
  S: serde::Serializer,
{
  let mut state = serializer.serialize_struct("Result", 1)?;

  match result {
    Ok(output) => state.serialize_field("output", output)?,
    Err(error) => state.serialize_field("error", error)?,
  }

  state.end()
}
