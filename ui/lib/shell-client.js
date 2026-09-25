// Logical Lunge kabuk istemcisi: widget'ların kabukla (lunge-shell) konuştuğu API.
//
// Zebar'ın istemci kütüphanesinden (3.3.1, GPL-3.0, © glzr-io; bkz. THIRD_PARTY_NOTICES.md) türetildi. Değişiklikler:
// - Yalnızca kullanılan API: currentWidget, shellExec, shellSpawn, createProvider, createProviderGroup.
// - Yalnızca kabuğun sunduğu sağlayıcılar: audio, battery, cpu, date, host, media, memory, network, systray, tiling.
// - Sağlayıcı çıktıları ve komutlar konsola yazılmıyor: her çıktı (saniyede bir saat, her pencere olayında bütün pencere
//   ağacı, simge verileriyle tepsi) nesneleriyle birlikte konsola yazılıyor ve tarayıcı bunları bellekte tutuyordu.
// - Pencere yöneticisi sağlayıcısı (tiling): yalnızca ilgili olaylara abone olur, art arda gelen olayları tek sorguda
//   birleştirir ve durum gerçekten değiştiyse gönderir (her olayda 3-4 sorgu + widget'ın yeniden çizilmesi yerine).
// - Tepsi: kimliği (hash) olmayan simgeler için her güncellemede yeni blob URL üretilip bırakılmıyordu.
import { z } from 'zod';
import { DateTime } from 'luxon';
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import {
  availableMonitors as getAvailableMonitors,
  currentMonitor as getCurrentMonitor,
  primaryMonitor as getPrimaryMonitor,
  getCurrentWindow,
} from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { getTilingClient } from './tiling-client.js';

// ---------------------------------------------------------------- kabuk komutları
const desktopCommands = {
  listenProvider: args => invoke('listen_provider', args),
  unlistenProvider: configHash => invoke('unlisten_provider', { configHash }),
  callProviderFunction: (configHash, fn) => invoke('call_provider_function', { configHash, function: fn }),
  setAlwaysOnTop: () => invoke('set_always_on_top'),
  shellExec: (program, args = [], options = {}) => invoke('shell_exec', { program, args, options }),
  shellSpawn: (program, args = [], options = {}) => invoke('shell_spawn', { program, args, options }),
  shellWrite: (processId, buffer) => invoke('shell_write', { processId, buffer }),
  shellKill: processId => invoke('shell_kill', { processId }),
};

async function invoke(command, args) {
  try {
    return await tauriInvoke(command, args);
  } catch (err) {
    throw new Error(`Kabuk komutu '${command}' başarısız: ${err}`);
  }
}

// Sağlayıcı ayarlarının kimliği (kabuk aynı ayarı bir kez çalıştırır)
function simpleHash(...args) {
  return JSON.stringify(args, (_, val) => (typeof val === 'object' ? val : String(val)));
}

function getCoordinateDistance(a, b) {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

// ---------------------------------------------------------------- monitörler
let monitorCachePromise = null;

function getMonitors() {
  return (monitorCachePromise ??= createMonitorCache());
}

async function createMonitorCache() {
  const [currentMonitor, primaryMonitor, allMonitors] = await Promise.all([
    getCurrentMonitor(),
    getPrimaryMonitor(),
    getAvailableMonitors(),
  ]);
  const cache = {
    currentMonitor: currentMonitor ? toMonitor(currentMonitor) : null,
    primaryMonitor: primaryMonitor ? toMonitor(primaryMonitor) : null,
    secondaryMonitors: allMonitors.filter(m => !primaryMonitor || !isSameMonitor(m, primaryMonitor)).map(toMonitor),
    allMonitors: allMonitors.map(toMonitor),
  };
  const update = async () => {
    const m = await getCurrentMonitor();
    cache.currentMonitor = m ? toMonitor(m) : null;
  };
  getCurrentWindow().onResized(update);
  getCurrentWindow().onMoved(update);
  return cache;
}

function isSameMonitor(a, b) {
  return a.name === b.name && a.position.x === b.position.x && a.position.y === b.position.y &&
    a.size.width === b.size.width && a.size.height === b.size.height;
}

function toMonitor(m) {
  return { name: m.name, width: m.size.width, height: m.size.height, x: m.position.x, y: m.position.y, scaleFactor: m.scaleFactor };
}

// ---------------------------------------------------------------- kabuktan gelen sağlayıcı çıktıları
let emitListenPromise = null;
let emitCallbacks = [];

async function onProviderEmit(config, callback) {
  const configHash = simpleHash(config);
  const entry = { configHash, fn: event => event.payload.configHash === configHash && callback(event.payload) };
  emitCallbacks.push(entry);
  const unlisten = await (emitListenPromise ??= listen('provider-emit', event => {
    for (const cb of emitCallbacks) cb.fn(event);
  }));
  await desktopCommands.listenProvider({ configHash, config });
  return async () => {
    emitCallbacks = emitCallbacks.filter(cb => cb !== entry);
    await desktopCommands.unlistenProvider(configHash);
    if (emitCallbacks.length === 0) {
      unlisten();
      emitListenPromise = null;
    }
  };
}

// ---------------------------------------------------------------- kabuk süreçleri
export async function shellExec(program, args, options) {
  return desktopCommands.shellExec(program, args, options);
}

export async function shellSpawn(program, args, options) {
  const processId = await desktopCommands.shellSpawn(program, args, options);
  const stdout = [], stderr = [], exit = [];
  const unlisten = await listen('shell-emit', event => {
    if (event.payload.pid !== processId) return;
    const e = event.payload.event;
    switch (e.type) {
      case 'stdout': stdout.forEach(cb => cb(e.data)); break;
      case 'stderr': stderr.forEach(cb => cb(e.data)); break;
      case 'terminated': exit.forEach(cb => cb(e.data)); unlisten(); break;
    }
  });
  return {
    processId,
    onStdout: cb => stdout.push(cb),
    onStderr: cb => stderr.push(cb),
    onExit: cb => exit.push(cb),
    kill: () => desktopCommands.shellKill(processId),
    write: data => desktopCommands.shellWrite(processId, data),
  };
}

// ---------------------------------------------------------------- bu widget
function getWidgetState() {
  if (window.__LUNGE_STATE) return window.__LUNGE_STATE;
  const saved = sessionStorage.getItem('LUNGE_STATE');
  if (!saved) throw new Error('Widget durumu bulunamadı.');
  return JSON.parse(saved);
}

export function currentWidget() {
  const state = getWidgetState();
  const tauriWindow = getCurrentWindow();
  const setZOrder = async zOrder => {
    if (zOrder === 'bottom_most') await tauriWindow.setAlwaysOnBottom(true);
    else if (zOrder === 'top_most') await desktopCommands.setAlwaysOnTop();
    else await tauriWindow.setAlwaysOnTop(false);
  };
  return {
    id: state.id,
    name: state.name,
    packId: state.packId,
    configPath: state.configPath,
    htmlPath: state.htmlPath,
    tauriWindow,
    window: { get tauri() { return tauriWindow; }, setZOrder },
    isPreview: state.isPreview,
    setZOrder,
    close: () => tauriWindow.close(),
  };
}

// ---------------------------------------------------------------- sağlayıcılar
function createBaseProvider(config, fetcher) {
  const outputListeners = new Set();
  const errorListeners = new Set();
  let latest = { output: null, error: null, hasError: false };
  const start = () =>
    fetcher({
      output: output => {
        latest = { output, error: null, hasError: false };
        outputListeners.forEach(l => l(output));
      },
      error: error => {
        latest = { output: null, error, hasError: true };
        errorListeners.forEach(l => l(error));
      },
    });
  let unlisten = start();
  return {
    get output() { return latest.output; },
    get error() { return latest.error; },
    get hasError() { return latest.hasError; },
    config,
    restart: async () => {
      if (unlisten) await (await unlisten)();
      unlisten = start();
    },
    stop: async () => {
      outputListeners.clear();
      errorListeners.clear();
      if (unlisten) {
        await (await unlisten)();
        unlisten = null;
      }
    },
    onOutput: cb => outputListeners.add(cb),
    onError: cb => errorListeners.add(cb),
  };
}

// Kabuğun hesapladığı sağlayıcılar: çıktı doğrudan aktarılır
function createPassThroughProvider(schema, config, decorate) {
  const merged = schema.parse(config);
  return createBaseProvider(merged, async queue =>
    onProviderEmit(merged, ({ configHash, result }) => {
      if ('error' in result) queue.error(result.error);
      else queue.output(decorate ? decorate(result.output, configHash) : result.output);
    }),
  );
}

const providerFunction = (configHash, type, name, args) =>
  desktopCommands.callProviderFunction(configHash, { type, function: { name, args } });

const schemas = {
  audio: z.object({ type: z.literal('audio') }),
  battery: z.object({ type: z.literal('battery'), refreshInterval: z.coerce.number().default(60 * 1e3) }),
  cpu: z.object({ type: z.literal('cpu'), refreshInterval: z.coerce.number().default(5 * 1e3) }),
  host: z.object({ type: z.literal('host'), refreshInterval: z.coerce.number().default(60 * 1e3) }),
  media: z.object({ type: z.literal('media') }),
  memory: z.object({ type: z.literal('memory'), refreshInterval: z.coerce.number().default(5 * 1e3) }),
  network: z.object({ type: z.literal('network'), refreshInterval: z.coerce.number().default(5 * 1e3) }),
  systray: z.object({ type: z.literal('systray') }),
  date: z.object({
    type: z.literal('date'),
    refreshInterval: z.coerce.number().default(1e3),
    timezone: z.string().default('local'),
    locale: z.string().optional(),
    formatting: z.string().default('EEE\td MMM t'),
  }),
  tiling: z.object({ type: z.literal('tiling') }),
};

function createAudioProvider(config) {
  return createPassThroughProvider(schemas.audio, config, (output, hash) => ({
    ...output,
    setVolume: (volume, options) => providerFunction(hash, 'audio', 'set_volume', { volume, deviceId: options?.deviceId }),
    setMute: (mute, options) => providerFunction(hash, 'audio', 'set_mute', { mute, deviceId: options?.deviceId }),
  }));
}

function createMediaProvider(config) {
  return createPassThroughProvider(schemas.media, config, (output, hash) => ({
    ...output,
    session: output.currentSession,
    play: options => providerFunction(hash, 'media', 'play', options ?? {}),
    pause: options => providerFunction(hash, 'media', 'pause', options ?? {}),
    togglePlayPause: options => providerFunction(hash, 'media', 'toggle_play_pause', options ?? {}),
    next: options => providerFunction(hash, 'media', 'next', options ?? {}),
    previous: options => providerFunction(hash, 'media', 'previous', options ?? {}),
  }));
}

function createSystrayProvider(config) {
  // simge kimliği -> { iconBlob, iconUrl }; kimliği olmayan simgelerin URL'si bir sonraki güncellemede bırakılır
  const iconCache = new Map();
  let unkeyed = [];
  return createPassThroughProvider(schemas.systray, config, (output, hash) => {
    const current = new Set();
    const nextUnkeyed = [];
    const icons = output.icons.map(icon => {
      let cached = icon.iconHash ? iconCache.get(icon.iconHash) : null;
      if (!cached) {
        const iconBlob = new Blob([new Uint8Array(icon.iconBytes)], { type: 'image/png' });
        cached = { iconBlob, iconUrl: URL.createObjectURL(iconBlob) };
        if (icon.iconHash) iconCache.set(icon.iconHash, cached);
        else nextUnkeyed.push(cached.iconUrl);
      }
      if (icon.iconHash) current.add(icon.iconHash);
      return { ...icon, iconBlob: cached.iconBlob, iconUrl: cached.iconUrl };
    });
    for (const [key, cached] of iconCache) {
      if (!current.has(key)) {
        URL.revokeObjectURL(cached.iconUrl);
        iconCache.delete(key);
      }
    }
    // yeni çıktı çizilince eskileri bırak (aynı anda ikisi de görünmesin diye bir sonraki turda)
    const old = unkeyed;
    unkeyed = nextUnkeyed;
    setTimeout(() => old.forEach(u => URL.revokeObjectURL(u)), 1000);
    const fn = name => iconId => providerFunction(hash, 'systray', name, { iconId });
    return {
      ...output,
      icons,
      onHoverEnter: fn('icon_hover_enter'),
      onHoverLeave: fn('icon_hover_leave'),
      onHoverMove: fn('icon_hover_move'),
      onLeftClick: fn('icon_left_click'),
      onLeftDoubleClick: fn('icon_left_double_click'),
      onRightClick: fn('icon_right_click'),
      onMiddleClick: fn('icon_middle_click'),
    };
  });
}

function createDateProvider(config) {
  const merged = schemas.date.parse(config);
  return createBaseProvider(merged, async queue => {
    const value = () => {
      const dt = DateTime.now().setZone(merged.timezone);
      return {
        new: dt.toJSDate(),
        now: dt.toMillis(),
        iso: dt.toISO(),
        formatted: dt.toFormat(merged.formatting, { locale: merged.locale }),
      };
    };
    queue.output(value());
    const interval = setInterval(() => queue.output(value()), merged.refreshInterval);
    return () => clearInterval(interval);
  });
}

// Pencere yöneticisi: odak, workspace'ler, pencereler, bağlama modları, bölme yönü, duraklatma
const TILING_EVENTS = [
  'focus_changed', 'focused_container_moved', 'workspace_activated', 'workspace_deactivated', 'workspace_updated',
  'window_managed', 'window_unmanaged', 'monitor_added', 'monitor_updated', 'monitor_removed',
  'binding_modes_changed', 'tiling_direction_changed', 'pause_changed',
];

function createTilingProvider(config) {
  const merged = schemas.tiling.parse(config);
  return createBaseProvider(merged, async queue => {
    const monitors = await getMonitors();
    const client = getTilingClient();
    const runCommand = (command, subjectContainerId) => client.runCommand(command, subjectContainerId);
    let state = null;
    let lastJson = '';
    // bir sonraki sorguda yenilenecekler
    let needAll = true, needTree = false, needDirection = false;
    let timer = 0, running = false, again = false;

    const schedule = () => {
      if (timer || running) { again = again || running; return; }
      // aynı anda gelen olaylar (workspace geçişi: devre dışı + etkin + odak) tek sorguda
      timer = setTimeout(flush, 8);
    };

    async function flush() {
      timer = 0;
      running = true;
      const all = needAll, tree = needTree, direction = needDirection;
      needAll = needTree = needDirection = false;
      try {
        let next = { ...(state ?? {}) };
        const jobs = [];
        if (all) {
          jobs.push(client.query('focused').then(r => (next.focusedContainer = r.focused)));
          jobs.push(client.query('binding-modes').then(r => (next.bindingModes = r.bindingModes)));
          jobs.push(client.query('paused').then(r => (next.isPaused = r.paused), () => (next.isPaused = false)));
        }
        if (all || direction) jobs.push(client.query('tiling-direction').then(r => (next.tilingDirection = r.tilingDirection)));
        if (all || tree) jobs.push(monitorState().then(s => Object.assign(next, s)));
        await Promise.all(jobs);
        next.runCommand = runCommand;
        const json = JSON.stringify(next);
        if (json !== lastJson) {
          lastJson = json;
          state = next;
          queue.output(state);
        }
      } catch {
        // bağlantı koptu: yeniden bağlanınca her şey baştan sorulur
        needAll = true;
      } finally {
        running = false;
        if (again) { again = false; schedule(); }
      }
    }

    async function monitorState() {
      const [{ monitors: wmMonitors }, { windows }] = await Promise.all([client.query('monitors'), client.query('windows')]);
      const here = monitors.currentMonitor ?? monitors.primaryMonitor ?? { x: 0, y: 0 };
      const current = wmMonitors.reduce((a, b) => (getCoordinateDistance(here, a) <= getCoordinateDistance(here, b) ? a : b));
      const focusedMonitor = wmMonitors.find(m => m.hasFocus);
      return {
        displayedWorkspace: current.children.find(w => w.isDisplayed),
        focusedWorkspace: focusedMonitor?.children.find(w => w.hasFocus),
        currentWorkspaces: current.children,
        allWorkspaces: wmMonitors.flatMap(m => m.children),
        focusedMonitor,
        currentMonitor: current,
        allMonitors: wmMonitors,
        allWindows: windows,
      };
    }

    const offEvents = client.subscribe(TILING_EVENTS, e => {
      switch (e.eventType) {
        case 'binding_modes_changed':
          state && (state = { ...state, bindingModes: e.newBindingModes });
          break;
        case 'pause_changed':
          state && (state = { ...state, isPaused: e.isPaused });
          break;
        case 'tiling_direction_changed':
          state && (state = { ...state, tilingDirection: e.newTilingDirection });
          break;
        case 'focus_changed':
        case 'focused_container_moved':
          state && (state = { ...state, focusedContainer: e.focusedContainer });
          needTree = true;
          needDirection = true;
          break;
        default:
          needTree = true;
      }
      schedule();
    });
    const offConnect = client.onConnect(() => { needAll = true; schedule(); });
    const offDisconnect = client.onDisconnect(() => queue.error('Pencere yöneticisine bağlanılamadı.'));
    return () => {
      clearTimeout(timer);
      offEvents();
      offConnect();
      offDisconnect();
    };
  });
}

export function createProvider(config) {
  switch (config.type) {
    case 'audio': return createAudioProvider(config);
    case 'battery': return createPassThroughProvider(schemas.battery, config);
    case 'cpu': return createPassThroughProvider(schemas.cpu, config);
    case 'date': return createDateProvider(config);
    case 'host': return createPassThroughProvider(schemas.host, config);
    case 'media': return createMediaProvider(config);
    case 'memory': return createPassThroughProvider(schemas.memory, config);
    case 'network': return createPassThroughProvider(schemas.network, config);
    case 'systray': return createSystrayProvider(config);
    case 'tiling': return createTilingProvider(config);
    default: throw new Error(`Bilinmeyen sağlayıcı: ${config.type}`);
  }
}

export function createProviderGroup(configMap) {
  const outputListeners = new Set();
  const errorListeners = new Set();
  const providerMap = Object.fromEntries(Object.entries(configMap).map(([name, config]) => [name, createProvider(config)]));
  let outputMap = Object.fromEntries(Object.keys(providerMap).map(name => [name, null]));
  let errorMap = Object.fromEntries(Object.keys(providerMap).map(name => [name, null]));
  for (const [name, provider] of Object.entries(providerMap)) {
    provider.onOutput(() => {
      outputMap = { ...outputMap, [name]: provider.output };
      errorMap = { ...errorMap, [name]: null };
      outputListeners.forEach(l => l(outputMap));
    });
    provider.onError(() => {
      errorMap = { ...errorMap, [name]: provider.error };
      outputMap = { ...outputMap, [name]: null };
      errorListeners.forEach(l => l(errorMap));
    });
  }
  return {
    get outputMap() { return outputMap; },
    get errorMap() { return errorMap; },
    get hasErrors() { return Object.values(errorMap).some(e => e != null); },
    configMap,
    raw: providerMap,
    onOutput: cb => outputListeners.add(cb),
    onError: cb => errorListeners.add(cb),
    restartAll: () => Promise.all(Object.values(providerMap).map(p => p.restart())),
    stopAll: async () => {
      outputListeners.clear();
      errorListeners.clear();
      await Promise.all(Object.values(providerMap).map(p => p.stop()));
    },
  };
}
