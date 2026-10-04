import React, { useState, useEffect, useLayoutEffect, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen, emit } from '@tauri-apps/api/event';
import * as shell from './lib/shell-client.js';
import { trackMedia, mediaPosition } from './lib/media-clock.mjs';
import { KINDS, APPEARANCES, preferredShapeSize, parseStore, projectWidgets, dragRect, monitorFor, dipArea, settleRect } from './desktop-widgets-model.mjs';
import { weatherParams } from './widget-location.mjs';
import { LocationPicker } from './widget-location-picker.mjs';
import { SHAPES, selectShape, shapeGeometry, physicalRegions, constrainShapeRect } from './widget-geometry.mjs';
import { ShapeChrome, shapeContentStyle, shapeControlStyle } from './widget-shape-view.mjs';
import { observeGlyphOutlines } from './widget-glyph-outline.mjs';

const h = React.createElement, T = s => window.LL_T?.(s) ?? s;
const win = shell.currentWidget().tauriWindow;
const ART = '{{INSTALL_ESC}}\\tools\\lunge-media.exe';
const Icon = ({ name }) => h('span', { className: 'icon', 'aria-hidden': true }, name);
const Button = ({ icon, label, ...props }) => h('button', { type: 'button', title: T(label), 'aria-label': T(label), ...props }, h(Icon, { name: icon }));
// A card's empty, loading or error state: the message is its own element (read out as a status, styled apart from the icon)
const Empty = ({ icon, text, children }) => h('div', { className: 'empty', role: 'status' }, h(Icon, { name: icon }), h('span', null, T(text)), children);
const read = (key, fallback) => { try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; } };
const locale = () => window.LL_LOCALE || navigator.language;
async function core(route, signal) {
  const timeout = AbortSignal.timeout(18000);
  const r = await fetch(`http://127.0.0.1:6131${route}`, { method: 'POST', cache: 'no-store', signal: signal ? AbortSignal.any([signal, timeout]) : timeout });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  const v = await r.json();
  if (v?.error) throw new Error(v.error);
  return v;
}
function useTick(active, period) {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => { if (!active) return; setNow(new Date()); const timer = setInterval(() => setNow(new Date()), period); return () => clearInterval(timer); }, [active, period]);
  return now;
}
function useProviders(kinds, active) {
  const [data, setData] = useState({ output: {}, errors: {} });
  const media = kinds.includes('media'), system = kinds.includes('system');
  useEffect(() => {
    if (!active || (!media && !system)) return;
    const config = { ...(media ? { media: { type: 'media' } } : {}), ...(system ? { cpu: { type: 'cpu', refreshInterval: 4000 }, memory: { type: 'memory', refreshInterval: 4000 } } : {}) };
    const group = shell.createProviderGroup(config); let live = true;
    const update = () => { if (live) setData({ output: { ...group.outputMap }, errors: { ...group.errorMap } }); };
    group.onOutput(update); group.onError(update); update();
    return () => { live = false; group.stopAll().catch(() => {}); };
  }, [active, media, system]);
  return data;
}
function useResource(load, key, active, successMs, failureMs = 60000) {
  const [state, setState] = useState({ data: null, error: null, loading: true });
  const [generation, refresh] = useState(0);
  useEffect(() => {
    if (!active) return;
    let live = true, timer;
    setState({ data: null, error: null, loading: true });
    const run = async () => {
      let delay = successMs;
      try { const data = await load(); if (live) setState({ data, error: null, loading: false }); }
      catch (error) { delay = failureMs; if (live) setState(s => ({ ...s, error: String(error), loading: false })); }
      if (live) timer = setTimeout(run, delay);
    };
    run(); return () => { live = false; clearTimeout(timer); };
  }, [key, active, successMs, failureMs, generation]);
  return { ...state, refresh: () => refresh(n => n + 1) };
}
function Clock({ spec, active }) {
  const geometry = shapeGeometry(spec.shape, spec.w, spec.h);
  const available = spec.shape === 'card' ? spec.w - 32 : spec.shape === 'split' ? geometry.leading - 24 : ['capsule', 'ticket'].includes(spec.shape) ? geometry.inner.w * .42 : geometry.inner.w;
  const now = useTick(active, spec.seconds || spec.clock === 'analog' ? 1000 : 30000);
  const date = now.toLocaleDateString(locale(), { weekday: 'long', day: 'numeric', month: 'long' });
  const time = now.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit', ...(spec.seconds ? { second: '2-digit' } : {}), hour12: !!window.LL_HOUR12 });
  if (spec.clock === 'analog') {
    const hand = (angle, end, width, cls) => h('line', { x1: 50, y1: 50, x2: 50, y2: end, strokeWidth: width, className: cls, transform: `rotate(${angle} 50 50)` });
    return h('div', { className: 'clock analog' }, h('svg', { viewBox: '0 0 100 100', role: 'img', 'aria-label': time },
      h('circle', { cx: 50, cy: 50, r: 46, className: 'face' }),
      ...Array.from({ length: 12 }, (_, i) => h('line', { key: i, x1: 50, y1: 8, x2: 50, y2: 13, transform: `rotate(${i * 30} 50 50)`, className: 'tick' })),
      hand((now.getHours() % 12 + now.getMinutes() / 60) * 30, 27, 4, 'hour'),
      hand((now.getMinutes() + now.getSeconds() / 60) * 6, 16, 3, 'minute'),
      spec.seconds && hand(now.getSeconds() * 6, 12, 1.2, 'second'), h('circle', { cx: 50, cy: 50, r: 3, className: 'hub' })),
      spec.date && h('div', { className: 'clock-date' }, date));
  }
  return h('div', { className: `clock ${spec.clock}` }, h('time', { className: 'clock-time', dateTime: now.toISOString(), style: { fontSize: `${Math.max(spec.shape === 'split' ? 16 : 24, Math.min(spec.clock === 'large' ? 72 : 48, available / (spec.seconds ? 5.5 : 3.7), spec.h - (spec.date ? 44 : 20)))}px` } }, time), spec.date && h('div', { className: 'clock-date' }, date));
}
function Media({ output, error, active, onError }) {
  const session = output?.currentSession, [art, setArt] = useState(null), [seeking, setSeeking] = useState(null);
  const clock = useRef({ key: '', pos: 0, at: 0 });
  useTick(active && !!session?.isPlaying, 500);
  trackMedia(clock.current, session);
  const identity = JSON.stringify([session?.sessionId, session?.title, session?.artist]);
  useEffect(() => {
    setArt(null); setSeeking(null); if (!session?.title || !active) return;
    let live = true;
    shell.shellExec(ART, []).then(r => { const url = (r.stdout || '').trim(); if (live && /^data:image\/(png|jpeg|webp|bmp);base64,/.test(url)) setArt(url); }).catch(() => {});
    return () => { live = false; };
  }, [identity, active]);
  if (!session) return h(Empty, { icon: 'music_note', text: error ? 'Medya alınamadı' : output ? 'Medya yok' : 'Yükleniyor…' });
  const start = session.startTime || 0, end = session.endTime || 0;
  const pos = seeking ?? mediaPosition(clock.current, session);
  const opt = { sessionId: session.sessionId };
  const action = name => Promise.resolve(output[name](opt)).catch(onError);
  const fmt = s => `${Math.floor(Math.max(0, s) / 60)}:${String(Math.floor(Math.max(0, s) % 60)).padStart(2, '0')}`;
  return h('div', { className: 'media-card' }, h('div', { className: 'art' }, art ? h('img', { src: art, alt: '', onError: () => setArt(null) }) : h(Icon, { name: 'music_note' })),
    h('div', { className: 'media-info' }, h('div', { className: 'media-title', title: session.title }, session.title || T('Medya')), h('div', { className: 'subtext', title: session.artist }, session.artist || ''),
      h('div', { className: 'media-controls' }, h(Button, { icon: 'skip_previous', label: 'Önceki', onClick: () => action('previous') }),
        h(Button, { icon: session.isPlaying ? 'pause' : 'play_arrow', label: session.isPlaying ? 'Duraklat' : 'Oynat', onClick: () => action('togglePlayPause') }),
        h(Button, { icon: 'skip_next', label: 'Sonraki', onClick: () => action('next') }), h('small', null, `${fmt(pos)}${end > start ? ' / ' + fmt(end) : ''}`)),
      h('input', { type: 'range', className: 'seek', 'aria-label': T('Medya konumu'), min: start, max: end || 1, step: .1, value: Math.max(start, Math.min(end || 1, pos)), disabled: end <= start,
        onInput: e => setSeeking(+e.target.value), onChange: e => setSeeking(+e.target.value),
        onPointerUp: e => seek(+e.currentTarget.value), onKeyUp: e => { if (['ArrowLeft', 'ArrowRight', 'Home', 'End', 'PageUp', 'PageDown'].includes(e.key)) seek(+e.currentTarget.value); } })));
  function seek(seconds) {
    if (end <= start) return;
    Object.assign(clock.current, { pos: seconds, at: Date.now() });
    setSeeking(null);
    shell.shellExec(ART, ['--seek', seconds.toFixed(1)]).catch(onError);
  }
}
function System({ spec, providers, active }) {
  const metrics = useResource(() => core('/widgets/metrics'), 'metrics', active, 4000, 15000);
  const rows = [['CPU', providers.output.cpu?.usage, spec.temps ? metrics.data?.cpuTemp : null], ['RAM', providers.output.memory?.usage, null]];
  if (metrics.data?.gpuUsage != null || (spec.temps && metrics.data?.gpuTemp != null)) rows.push(['GPU', metrics.data.gpuUsage, spec.temps ? metrics.data.gpuTemp : null]);
  return h('div', { className: 'system-card' }, ...rows.map(([name, value, temp]) => h('div', { key: name, className: 'metric' },
    h('div', { className: 'metric-label' }, h('span', null, name), h('span', { className: 'subtext' }, Number.isFinite(temp) ? `${Math.round(temp)}°` : ''), h('span', null, Number.isFinite(value) ? `${Math.round(value)}%` : '–')),
    h('progress', { max: 100, value: Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : 0, 'aria-label': name }))),
    (providers.errors.cpu || providers.errors.memory) && h('small', { className: 'subtext' }, T('Veri alınamadı')));
}
function Weather({ spec, active }) {
  const forceRefresh = useRef(false);
  const query = weatherParams(spec, locale()).toString();
  const weather = useResource(() => {
    const params = new URLSearchParams(query);
    if (forceRefresh.current) { params.set('refresh', '1'); forceRefresh.current = false; }
    return core(`/widgets/weather?${params}`);
  }, query, active, 900000);
  const refreshWeather = () => { forceRefresh.current = true; weather.refresh(); };
  useEffect(() => { const refresh = e => { if (e.detail === spec.id) refreshWeather(); }; window.addEventListener('ll-widget-weather-refresh', refresh); return () => window.removeEventListener('ll-widget-weather-refresh', refresh); }, [spec.id]);
  const data = weather.data;
  // The core returns the native Report shape, or {report: Report}.
  const report = data?.report ?? data;
  const sky = report ? weatherDescription(report.code, report.day) : null;
  const [width,height]=preferredShapeSize('weather',spec.shape);
  const base=constrainShapeRect(spec.shape,{x:0,y:0,w:width,h:height},10000,10000,KINDS.weather.min);
  const zoom=Math.max(1,Math.min(1.75,spec.w/base.w,spec.h/base.h));
  return h('div', { className: 'weather-card', style: { zoom, width:'100%', height:'100%' } }, report ? h(React.Fragment, null,
    h('div', { className: 'weather-main' }, h(Icon, { name: sky.icon }), h('span', null, `${Math.round(report.temp)}${spec.fahrenheit ? '°F' : '°'}`)),
    h('div', null, T(sky.text), Number.isFinite(report.high) && Number.isFinite(report.low) ? ` · ↑${Math.round(report.high)}° ↓${Math.round(report.low)}°` : ''),
    h('div', { className: 'subtext' }, report.place)) : h(Empty, { icon: weather.error ? 'cloud_off' : 'partly_cloudy_day', text: weather.error ? 'Hava durumu alınamadı' : 'Yükleniyor…' }, weather.error && h(Button, { icon: 'refresh', label: 'Yenile', onClick: refreshWeather })),
    weather.error && report && h('small', { className: 'subtext', title: weather.error }, T('Hava durumu alınamadı'), h(Button, { icon: 'refresh', label: 'Yenile', onClick: refreshWeather })));
}
// Keep the same WMO descriptions as the native Open-Meteo implementation.
import { weatherDescription } from './desktop-widgets-model.mjs';
function Agenda({ active }) {
  const now = useTick(active, 30000), [todos, setTodos] = useState(() => read('ll.todo', []));
  useEffect(() => {
    const update = () => { const v = read('ll.todo', []); setTodos(Array.isArray(v) ? v : []); };
    window.addEventListener('storage', update); const timer = active ? setInterval(update, 5000) : null;
    return () => { window.removeEventListener('storage', update); clearInterval(timer); };
  }, [active]);
  const pending = (Array.isArray(todos) ? todos : []).filter(t => !t.done && t.content?.trim()).slice(0, 20);
  return h('div', { className: 'agenda-card' }, h('div', { className: 'agenda-day' }, h('span', { className: 'day-number' }, now.getDate()), h('span', null, now.toLocaleDateString(locale(), { weekday: 'long', month: 'long' }))),
    h('div', { className: 'agenda-list' }, pending.length ? pending.map((t, i) => h('div', { className: 'todo-row', key: i }, h(Icon, { name: 'radio_button_unchecked' }), h('span', null, t.content))) : h('span', { className: 'subtext' }, T('Yapılacak yok'))));
}
function Note({ spec, patch, editing, onError }) {
  const [text, setText] = useState(spec.note), [dirty, setDirty] = useState(false);
  const draft = useRef(spec.note), saved = useRef(spec.note), timer = useRef(null), area = useRef(null);
  useEffect(() => { if (!dirty) { setText(spec.note); draft.current = saved.current = spec.note; } }, [spec.note, dirty]);
  const flush = async () => {
    clearTimeout(timer.current); if (draft.current === saved.current) return;
    const value = draft.current;
    try { await patch({ note: value }); saved.current = value; if (draft.current === value) setDirty(false); }
    catch (e) { onError(e); }
  };
  useEffect(() => {
    const handler = e => { if (e.detail === spec.id) { editing(true).then(() => area.current?.focus()); } };
    window.addEventListener('ll-widget-note-edit', handler);
    const stop = () => { flush(); };
    window.addEventListener('pagehide', stop);
    return () => { clearTimeout(timer.current); flush(); window.removeEventListener('pagehide', stop); window.removeEventListener('ll-widget-note-edit', handler); };
  }, [spec.id]);
  return h('textarea', { ref: area, className: 'note-editor', 'aria-label': T('Not'), placeholder: T('Not yazmak için tıkla'), value: text, spellCheck: true,
    onPointerDown: () => editing(true).then(() => area.current?.focus()).catch(onError), onFocus: () => editing(true).catch(onError),
    onChange: e => { const value = e.target.value; setText(value); draft.current = value; setDirty(true); clearTimeout(timer.current); timer.current = setTimeout(flush, 400); },
    onBlur: () => { flush(); editing(false).catch(onError); }, onKeyDown: e => { if (e.key === 'Escape' || (e.key === 'Enter' && e.ctrlKey)) { flush(); e.currentTarget.blur(); e.stopPropagation(); } } });
}
function App() {
  const [host, setHost] = useState(null), [store, setStore] = useState({ version: 1, revision: 0, widgets: [] });
  const [error, setError] = useState(null), [menu, setMenu] = useState(null), [preview, setPreview] = useState(null);
  const hostRef = useRef(null), storeRef = useRef(store), drag = useRef(null), serial = useRef(Promise.resolve()), menuRef = useRef(null);
  const regionSerial = useRef(Promise.resolve());
  const [popup, setPopup] = useState(null), popupRef = useRef(null), popupElement = useRef(null);
  const showError = e => setError(String(e));
  const accept = value => {
    try { const next = parseStore(value); if (next.revision >= storeRef.current.revision) { storeRef.current = next; setStore(next); } if (value.warning) showError(value.warning); } catch (e) { showError(e); }
  };
  const update = operation => {
    const job = serial.current.catch(() => {}).then(() => invoke('desktop_widgets_update', { operation }));
    serial.current = job; job.then(accept).catch(showError); return job;
  };
  // Native editing(true) also requests focus. In-window blur can retain a
  // popup's activation; global window blur disables it directly below.
  const editing = value => !value && popupRef.current ? Promise.resolve() : invoke('desktop_widgets_editing', { editing: value });
  const closeLocation = () => { popupRef.current = null; setPopup(null); editing(false).catch(showError); };
  const active = host?.active !== false && !document.hidden;
  const projected = host ? projectWidgets(store, host.monitors, host.monitor) : [];
  const previewHere = preview?.spec && host && monitorFor(host.monitors, preview.spec.monitor)?.device === host.monitor;
  const shown = projected.map(s => s.id === preview?.spec?.id ? (previewHere ? { ...preview.spec, displayMonitor: host.monitor } : { ...s, ghostOut: true }) : s);
  if (previewHere && !shown.some(s => s.id === preview.spec.id)) shown.push({ ...preview.spec, displayMonitor: host.monitor });
  const providers = useProviders(shown.map(s => s.kind).join(','), active);
  useEffect(() => {
    let live = true, retry, off = [];
    const on = async (name, callback) => { const unlisten = await listen(name, callback); if (live) off.push(unlisten); else unlisten(); };
    const init = async () => {
      try {
        const value = await invoke('desktop_widgets_bootstrap'); if (!live) return;
        value.active = true; hostRef.current = value; setHost(value); accept(value.store);
      } catch (e) { if (live) { showError(e); retry = setTimeout(init, 3000); } }
    };
    // Subscribe before loading: mutations from another monitor cannot be lost.
    Promise.all([
      on('ll:desktop-widgets-store', e => accept(e.payload)),
      on('ll:desktop-widgets-host', e => { if (drag.current) finishDrag(true); hostRef.current = e.payload; setHost(e.payload); }),
      on('ll:desktop-widget-add', e => {
        const here = hostRef.current; if (!here) return;
        const p = e.payload, m = here.monitors.find(m => p.x >= m.x && p.x < m.x + m.width && p.y >= m.y - 128 && p.y < m.y + m.height) ?? monitorFor(here.monitors, '');
        if (m?.device === here.monitor) update({ action: 'add', kind: p.kind === 'notes' ? 'note' : p.kind, monitor: m.device });
      }),
      on('ll:desktop-widgets-action', e => {
        const here = hostRef.current, p = e.payload;
        if (here && (p.monitor || monitorFor(here.monitors, '')?.device) === here.monitor) update({ ...p, kind: p.kind === 'notes' ? 'note' : p.kind });
      }),
      on('ll:desktop-widgets-drag', e => { if (e.payload.source !== win.label) setPreview(e.payload.spec ? e.payload : null); }),
    ]).then(init).catch(showError);
    const key = e => { if (e.key === 'Escape') { finishDrag(true); setMenu(null); editing(false).catch(showError); } };
    const blur = () => { finishDrag(true); setMenu(null); invoke('desktop_widgets_editing', { editing: false }).catch(() => {}); };
    const location = e => {
      if (popupRef.current?.id === e.detail) { editing(true).catch(showError); return; }
      const spec = storeRef.current.widgets.find(s => s.id === e.detail);
      if (!spec) return;
      const next = { id: spec.id, x: spec.x, y: spec.y + spec.h + 8 };
      popupRef.current = next; setPopup(next); setMenu(null);
    };
    window.addEventListener('keydown', key); window.addEventListener('blur', blur);
    window.addEventListener('ll-widget-weather-edit', location);
    return () => { live = false; clearTimeout(retry); off.forEach(fn => fn()); window.removeEventListener('keydown', key); window.removeEventListener('blur', blur); window.removeEventListener('ll-widget-weather-edit', location); };
  }, []);
  useEffect(() => { const change = () => setHost(s => s ? { ...s } : s); document.addEventListener('visibilitychange', change); return () => document.removeEventListener('visibilitychange', change); }, []);
  useEffect(() => {
    let live = true;
    const refresh = () => core('/prefs.json').then(p => {
      if (!live) return; window.LL_PREFS = p; window.LL_HOUR12 = p.clock === '12';
      if (p.language && p.language !== 'system') window.LL_LOCALE = p.language;
      document.documentElement.dataset.theme = p.theme === 'light' ? 'light' : 'dark';
      setHost(s => s ? { ...s } : s);
    }).catch(() => {});
    const off = listen('ll:prefs', refresh); refresh();
    return () => { live = false; off.then(f => f()).catch(() => {}); };
  }, []);
  // Factory owns browser zoom (web_scale); compensate CSS coordinate geometry
  // using physical viewport and the module's actual monitor scale. No double zoom.
  useLayoutEffect(() => {
    if (!host) return;
    const m = monitorFor(host.monitors, host.monitor);
    const browserScale = window.devicePixelRatio || 1;
    const factor = m.scale / browserScale;
    document.getElementById('root').style.zoom = factor;
  }, [host]);
  useLayoutEffect(() => {
    if (!host) return;
    let frame;
    const collect = () => {
      const [aw, ah] = dipArea(monitorFor(host.monitors, host.monitor));
      const zoom = monitorFor(host.monitors, host.monitor).scale / (window.devicePixelRatio || 1);
      for (const [element, anchor] of [[menuRef.current, menu], [popupElement.current, popup]]) {
        if (!element || !anchor) continue;
        element.style.maxWidth = `${aw}px`; element.style.maxHeight = `${Math.max(0, ah - 8)}px`;
        const r = element.getBoundingClientRect();
        element.style.left = `${Math.max(0, Math.min(anchor.x, aw - r.width / zoom))}px`;
        element.style.top = `${Math.max(0, Math.min(anchor.y, ah - r.height / zoom))}px`;
      }
      const rects = [...document.querySelectorAll('.widget, .widget-menu, .widget-location-popup, .host-error')].flatMap(el => {
        const r = el.getBoundingClientRect(), dpi = window.devicePixelRatio || 1;
        if (el.matches('.widget')) {
          const spec = shown.find(s => s.id === +el.dataset.widgetId);
          return physicalRegions(shapeGeometry(spec.shape, spec.w, spec.h), { x: r.left * dpi, y: r.top * dpi, scaleX: r.width * dpi / spec.w, scaleY: r.height * dpi / spec.h });
        }
        return { x: r.left * dpi, y: r.top * dpi, w: r.width * dpi, h: r.height * dpi };
      });
      // Retain the press rectangle while captured; empty space stays outside HRGN.
      if (drag.current?.initialRegions) rects.push(...drag.current.initialRegions);
      regionSerial.current = regionSerial.current.catch(() => {}).then(() => invoke('desktop_widgets_regions', { regions: rects })).catch(showError);
    };
    const schedule = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(collect); };
    const observer = new ResizeObserver(schedule);
    document.querySelectorAll('.widget, .widget-menu, .widget-location-popup, .host-error').forEach(el => observer.observe(el));
    schedule(); window.addEventListener('resize', schedule);
    return () => { observer.disconnect(); cancelAnimationFrame(frame); window.removeEventListener('resize', schedule); };
  }, [host, store, menu, popup, preview, error]);
  useLayoutEffect(() => {
    const cleanups = [...document.querySelectorAll('.widget[data-appearance="outline"] .widget-content')].map(observeGlyphOutlines);
    return () => cleanups.forEach(cleanup => cleanup());
  }, [host, store, preview]);
  useEffect(() => { if (popup && !store.widgets.some(s => s.id === popup.id)) closeLocation(); }, [store, popup]);
  const pointerDown = async (e, spec, resize = false) => {
    if (e.button !== 0 || drag.current || !hostRef.current) return;
    if (!resize && e.target.closest('button, input, textarea, form')) return;
    e.preventDefault(); setMenu(null);
    const element = e.currentTarget.closest('.widget'), rect = element.getBoundingClientRect(), dpi = window.devicePixelRatio || 1;
    const pointerId = e.pointerId;
    // Capture synchronously before awaiting IPC; a fast release still cancels.
    element.setPointerCapture(pointerId);
    const initialRegions = physicalRegions(shapeGeometry(spec.shape, spec.w, spec.h), { x: rect.left * dpi, y: rect.top * dpi, scaleX: rect.width * dpi / spec.w, scaleY: rect.height * dpi / spec.h });
    const d = { spec, resize, element, pointerId, initialRegions, pending: false, latest: spec, moved: false };
    drag.current = d;
    try { d.start = await invoke('desktop_widgets_cursor'); } catch (error) { showError(error); finishDrag(true); }
  };
  async function moveDrag() {
    const d = drag.current; if (!d || d.pending || !d.start) return;
    d.pending = true;
    try {
      const point = await invoke('desktop_widgets_cursor'); if (drag.current !== d) return;
      if (Math.hypot(point.x - d.start.x, point.y - d.start.y) < 2 && !d.moved) return;
      d.moved = true; d.latest = dragRect(d.spec, d.start, point, hostRef.current.monitors, d.resize);
      const payload = { source: win.label, spec: d.latest }; setPreview(payload); await emit('ll:desktop-widgets-drag', payload);
    } catch (error) { showError(error); } finally { d.pending = false; }
  }
  async function finishDrag(cancel = false) {
    const d = drag.current; if (!d) return;
    drag.current = null;
    try { if (d.element.hasPointerCapture(d.pointerId)) d.element.releasePointerCapture(d.pointerId); } catch {}
    setPreview(null); emit('ll:desktop-widgets-drag', { source: win.label, spec: null }).catch(showError);
    if (!cancel && d.moved) {
      const m = monitorFor(hostRef.current.monitors, d.latest.monitor);
      const spec = settleRect(d.latest, projectWidgets(storeRef.current, hostRef.current.monitors, m.device), ...dipArea(m));
      await update({ action: 'patch', id: spec.id, patch: { x: spec.x, y: spec.y, w: spec.w, h: spec.h, monitor: spec.monitor } });
    }
  }
  const openMenu = (e, spec) => {
    e.preventDefault(); e.stopPropagation();
    const zoom = monitorFor(hostRef.current.monitors, hostRef.current.monitor).scale / (window.devicePixelRatio || 1);
    setMenu({ id: spec.id, x: e.clientX / zoom, y: e.clientY / zoom }); editing(true).catch(showError);
  };
  const menuSpec = store.widgets.find(s => s.id === menu?.id);
  const popupSpec = store.widgets.find(s => s.id === popup?.id);
  return h('main', { className: 'desktop-surface', onPointerDown: () => { if (menu) { setMenu(null); editing(false).catch(showError); } } },
    ...shown.map(spec => { const geometry = shapeGeometry(spec.shape, spec.w, spec.h), clipId = `widget-clip-${spec.id}`;
      return h('article', { key: spec.id, className: `widget widget-${spec.kind}${preview?.spec?.id === spec.id ? ' dragging' : ''}${spec.ghostOut ? ' ghost-out' : ''}`, 'data-widget-id': spec.id, 'data-appearance': spec.appearance, 'data-shape': spec.shape, 'aria-label': T(KINDS[spec.kind].label),
      style: { left: spec.x, top: spec.y, width: spec.w, height: spec.h, borderRadius: spec.shape === 'card' ? 18 : 0, clipPath: `url(#${clipId})`, '--widget-background-opacity': spec.backgroundOpacity, '--widget-content-opacity': spec.contentOpacity }, onPointerDown: e => pointerDown(e, spec), onPointerMove: moveDrag,
      onPointerUp: () => finishDrag(), onPointerCancel: () => finishDrag(true), onLostPointerCapture: () => { if (drag.current) finishDrag(); }, onContextMenu: e => openMenu(e, spec) },
      h(ShapeChrome, { geometry, id: clipId }),
      h('div', { className: 'widget-content', style: shapeContentStyle(geometry) },
        spec.shape === 'split' && ['system', 'note'].includes(spec.kind) && h('div', { className: 'shape-lead' }, h(Icon, { name: KINDS[spec.kind].icon })),
        spec.kind === 'clock' && h(Clock, { spec, active }),
        spec.kind === 'media' && h(Media, { output: providers.output.media, error: providers.errors.media, active, onError: showError }),
        spec.kind === 'system' && h(System, { spec, providers, active }),
        spec.kind === 'weather' && h(Weather, { spec, active }),
        spec.kind === 'agenda' && h(Agenda, { active }),
        spec.kind === 'note' && h(Note, { spec, patch: patch => update({ action: 'patch', id: spec.id, patch }), editing, onError: showError }),
        spec.shape === 'polaroid' && ['system', 'note'].includes(spec.kind) && h('div', { className: 'shape-caption' }, T(KINDS[spec.kind].label))),
      h('div', { className: 'widget-tools', style: shapeControlStyle(geometry.toolbar) }, h(Button, { icon: 'more_horiz', label: 'Ayarlar', onClick: e => openMenu(e, spec) })),
      h('button', { className: 'resize-grip', style: shapeControlStyle(geometry.grip), title: T('Boyutlandır'), 'aria-label': T('Boyutlandır'), onPointerDown: e => { e.stopPropagation(); pointerDown(e, spec, true); }, onKeyDown: e => {
        const moves = { ArrowLeft: [-8, 0], ArrowRight: [8, 0], ArrowUp: [0, -8], ArrowDown: [0, 8] }; if (!moves[e.key]) return;
        e.preventDefault(); const [dw, dh] = moves[e.key];
        const side = spec.w + dw + dh;
        update({ action: 'patch', id: spec.id, patch: spec.shape === 'circle' ? { w: side, h: side } : { w: spec.w + dw, h: spec.h + dh } });
      } }, h(Icon, { name: 'south_east' }))); }),
    menu && menuSpec && h('div', { className: 'widget-menu', role: 'menu', ref: menuRef, style: { left: menu.x, top: menu.y }, onPointerDown: e => e.stopPropagation() },
      menu.shape ? h(React.Fragment, null, h('button', { role: 'menuitem', onClick: () => setMenu({ ...menu, shape: false }) }, h(Icon, { name: 'arrow_back' }), T('Geri')),
        h('div', { className: 'menu-heading' }, T('Biçim')), ...Object.entries(SHAPES).map(([shape, label]) => h('button', { key: shape, role: 'menuitemradio', 'aria-checked': menuSpec.shape === shape, onClick: () => {
          const patch = selectShape(menuSpec, shape, ...dipArea(monitorFor(host.monitors, menuSpec.monitor)), KINDS[menuSpec.kind].min, preferredShapeSize(menuSpec.kind,shape)); update({ action: 'patch', id: menuSpec.id, patch });
        } }, menuSpec.shape === shape ? h(Icon, { name: 'check' }) : h('span', { className: 'menu-spacer' }), T(label)))) : menu.appearance ? h(AppearanceMenu, { key: menuSpec.id, spec: menuSpec, patch: patch => update({ action: 'patch', id: menuSpec.id, patch }), back: () => setMenu({ ...menu, appearance: false }) }) : menuItems(menuSpec).map(item => item.sep ? h('hr', { key: item.key }) : h('button', { key: item.key, role: item.checked == null ? 'menuitem' : 'menuitemcheckbox', 'aria-checked': item.checked, onClick: () => {
        if (item.key === 'appearance') { setMenu({ ...menu, appearance: true }); return; }
        if (item.key === 'shape') { setMenu({ ...menu, shape: true }); return; }
        setMenu(null); editing(false).catch(showError);
        if (item.patch) update({ action: 'patch', id: menuSpec.id, patch: item.patch });
        else if (item.kind) update({ action: 'add', kind: item.kind, monitor: host.monitor });
        else if (item.key === 'remove') update({ action: 'remove', id: menuSpec.id });
        else if (item.key === 'edit') window.dispatchEvent(new CustomEvent('ll-widget-note-edit', { detail: menuSpec.id }));
        else if (item.key === 'city:edit') window.dispatchEvent(new CustomEvent('ll-widget-weather-edit', { detail: menuSpec.id }));
        else if (item.key === 'refresh') { update({ action: 'patch', id: menuSpec.id, patch: {} }); window.dispatchEvent(new CustomEvent('ll-widget-weather-refresh', { detail: menuSpec.id })); }
      } }, item.checked ? h(Icon, { name: 'check' }) : h('span', { className: 'menu-spacer' }), T(item.label)))),
    popup && popupSpec && h(LocationPicker, { key: popup.id, elementRef: popupElement, spec: popupSpec, load: core, language: locale(), translate: T, editing,
      position: { left: popup.x, top: popup.y }, cancel: closeLocation, save: async patch => { await update({ action: 'patch', id: popupSpec.id, patch }); closeLocation(); } }),
    error && host && h('div', { className: 'host-error', role: 'alert' }, h('span', null, T('İşlem tamamlanamadı'), ': ', error), h(Button, { icon: 'close', label: 'Kapat', onClick: () => setError(null) })));
}
function menuItems(s) {
  const items = [];
  if (s.kind === 'clock') {
    for (const [style, label] of [['digital', 'Dijital'], ['large', 'Büyük'], ['analog', 'Analog']]) items.push({ key: style, label, checked: s.clock === style, patch: { clock: style } });
    items.push({ key: 'seconds', label: 'Saniyeleri göster', checked: s.seconds, patch: { seconds: !s.seconds } }, { key: 'date', label: 'Tarihi göster', checked: s.date, patch: { date: !s.date } });
  }
  if (s.kind === 'system') items.push({ key: 'temps', label: 'Sıcaklıkları göster', checked: s.temps, patch: { temps: !s.temps } });
  if (s.kind === 'weather') items.push({ key: 'city:auto', label: 'Konumu saat diliminden al', checked: !s.city && !s.location, patch: { city: '', location: null } }, { key: 'city:edit', label: 'Konumu değiştir…' }, { key: 'fahrenheit', label: '°F', checked: s.fahrenheit, patch: { fahrenheit: !s.fahrenheit } }, { key: 'refresh', label: 'Yenile' });
  if (s.kind === 'note') items.push({ key: 'edit', label: 'Düzenle' });
  items.push({ key: 'appearance', label: 'Görünüş' }, { key: 'shape', label: 'Biçim' }, { key: 'add-heading', sep: true });
  for (const [kind, meta] of Object.entries(KINDS)) items.push({ key: `add:${kind}`, label: `${T('Widget ekle')}: ${T(meta.label)}`, kind });
  items.push({ key: 'end', sep: true }, { key: 'remove', label: 'Widget’ı kaldır' });
  return items;
}
function AppearanceMenu({ spec, patch, back }) {
  const [opacity, setOpacity] = useState(() => ({ backgroundOpacity: spec.backgroundOpacity, contentOpacity: spec.contentOpacity }));
  return h(React.Fragment, null, h('button', { role: 'menuitem', onClick: back }, h(Icon, { name: 'arrow_back' }), T('Geri')),
    h('div', { className: 'menu-heading' }, T('Görünüş')),
    ...Object.entries(APPEARANCES).map(([id, label]) => h('button', { key: id, role: 'menuitemradio', 'aria-checked': spec.appearance === id, onClick: () => patch({ appearance: id }) }, spec.appearance === id ? h(Icon, { name: 'check' }) : h('span', { className: 'menu-spacer' }), T(label))),
    h('hr'), ...[['backgroundOpacity', 'Arka plan opaklığı'], ['contentOpacity', 'İçerik opaklığı']].map(([key, label]) => h('label', { className: 'opacity-control', key }, h('span', null, T(label), h('output', null, `${Math.round(opacity[key] * 100)}%`)),
      h('input', { type: 'range', min: 0, max: 1, step: .01, 'aria-label': T(label), value: opacity[key], onChange: e => { const value = +e.target.value; setOpacity(s => ({ ...s, [key]: value })); patch({ [key]: value }); } }))));
}
createRoot(document.getElementById('root')).render(h(App));
