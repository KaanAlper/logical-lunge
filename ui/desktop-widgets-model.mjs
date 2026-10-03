// Pure layout and persistence rules shared by the desktop surface and its tests.
export const GRID = 8, MARGIN = 24;
export const KINDS = {
  clock: { label: 'Saat', icon: 'schedule', size: [264, 120], min: [136, 72] },
  media: { label: 'Medya', icon: 'music_note', size: [336, 112], min: [240, 96] },
  system: { label: 'Sistem', icon: 'memory', size: [288, 128], min: [200, 96] },
  weather: { label: 'Hava durumu', icon: 'partly_cloudy_day', size: [248, 128], min: [176, 96] },
  agenda: { label: 'Ajanda', icon: 'event', size: [264, 232], min: [176, 136] },
  note: { label: 'Not', icon: 'sticky_note_2', size: [248, 200], min: [144, 96] },
};
const finite = (v, fallback) => Number.isFinite(v) ? v : fallback;
export const snap = v => Math.round(v / GRID) * GRID;
export function createSpec(id, kind, monitor = '') {
  if (!Object.hasOwn(KINDS, kind)) throw new Error('Unknown widget kind');
  const [w, h] = KINDS[kind].size;
  return { id, kind, monitor, x: MARGIN, y: MARGIN, w, h, clock: 'digital', seconds: false, date: true, temps: true, city: '', fahrenheit: false, note: '' };
}
export function normalizeSpec(raw) {
  const spec = createSpec(raw.id, raw.kind, typeof raw.monitor === 'string' ? raw.monitor : '');
  for (const k of ['x', 'y', 'w', 'h']) spec[k] = finite(raw[k], spec[k]);
  for (const k of ['seconds', 'date', 'temps', 'fahrenheit']) if (typeof raw[k] === 'boolean') spec[k] = raw[k];
  for (const k of ['city', 'note']) if (typeof raw[k] === 'string') spec[k] = raw[k];
  if (['digital', 'large', 'analog'].includes(raw.clock)) spec.clock = raw.clock;
  if (spec.w <= 0) spec.w = KINDS[spec.kind].size[0];
  if (spec.h <= 0) spec.h = KINDS[spec.kind].size[1];
  return spec;
}
export function parseStore(text) {
  const raw = typeof text === 'string' ? JSON.parse(text) : text;
  if (!raw || !Array.isArray(raw.widgets) || (raw.version ?? 1) > 1) throw new Error('Unreadable widget layout');
  const ids = new Set();
  return { version: 1, revision: finite(raw.revision, 0), widgets: raw.widgets.filter(s => {
    if (!s || !Number.isSafeInteger(s.id) || s.id <= 0 || !Object.hasOwn(KINDS, s.kind) || ids.has(s.id)) return false;
    ids.add(s.id); return true;
  }).map(normalizeSpec) };
}
export function serializeStore(store) { return JSON.stringify({ ...parseStore(store), version: 1 }, null, 2); }
export function clampRect(kind, rect, aw, ah) {
  const [mw, mh] = KINDS[kind].min;
  aw = Math.max(0, finite(aw, 0)); ah = Math.max(0, finite(ah, 0));
  const w = Math.min(aw, Math.max(mw, finite(rect.w, mw))), h = Math.min(ah, Math.max(mh, finite(rect.h, mh)));
  return { x: Math.max(0, Math.min(finite(rect.x, 0), aw - w)), y: Math.max(0, Math.min(finite(rect.y, 0), ah - h)), w, h };
}
export function overlaps(a, b) { return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h; }
export function freeSpot(size, taken, aw, ah) {
  const right = Math.max(0, Math.min(aw - size.w, snap(Math.max(0, aw - MARGIN - size.w))));
  for (let x = right; x >= 0; x -= GRID * 4) {
    for (let y = Math.min(MARGIN, Math.max(0, ah - size.h)); y + size.h <= ah; y += GRID * 2) {
      const grown = { x: x - GRID, y: y - GRID, w: size.w + GRID * 2, h: size.h + GRID * 2 };
      if (!taken.some(t => overlaps(grown, t))) return { x, y };
    }
  }
  return { x: right, y: Math.min(MARGIN, Math.max(0, ah - size.h)) };
}
export const monitorFor = (monitors, device) => monitors.find(m => device && m.device.toLowerCase() === device.toLowerCase()) ?? monitors.find(m => m.primary) ?? monitors[0];
export const dipArea = m => [m.width / m.scale, m.height / m.scale];
export function projectWidgets(store, monitors, device) {
  return store.widgets.flatMap(s => {
    const m = monitorFor(monitors, s.monitor);
    return m?.device === device ? [{ ...s, ...clampRect(s.kind, s, ...dipArea(m)), displayMonitor: m.device }] : [];
  });
}
function monitorAt(monitors, point) {
  return monitors.find(m => point.x >= m.x && point.x < m.x + m.width && point.y >= m.y && point.y < m.y + m.height) ??
    monitors.reduce((best, m) => {
      const distance = n => Math.hypot(Math.max(n.x - point.x, 0, point.x - n.x - n.width), Math.max(n.y - point.y, 0, point.y - n.y - n.height));
      return !best || distance(m) < distance(best) ? m : best;
    }, null);
}
export function dragRect(spec, start, point, monitors, resize) {
  const from = monitorFor(monitors, spec.displayMonitor ?? spec.monitor);
  if (!from) return { ...spec };
  const target = resize ? from : monitorAt(monitors, point);
  let r;
  if (resize) r = { ...spec, w: snap(spec.w + (point.x - start.x) / from.scale), h: snap(spec.h + (point.y - start.y) / from.scale) };
  else {
    const gx = (start.x - from.x) / from.scale - spec.x, gy = (start.y - from.y) / from.scale - spec.y;
    r = { ...spec, x: snap((point.x - target.x) / target.scale - gx), y: snap((point.y - target.y) / target.scale - gy) };
  }
  return { ...spec, ...clampRect(spec.kind, r, ...dipArea(target)), monitor: target.device };
}
export function settleRect(spec, taken, aw, ah) {
  const others = taken.filter(t => t.id !== spec.id);
  const next = { ...spec, ...clampRect(spec.kind, spec, aw, ah) };
  return others.some(t => overlaps(next, t)) ? { ...next, ...freeSpot(next, others, aw, ah) } : next;
}
export function applyOperation(store, op, monitors) {
  const next = parseStore(store);
  if (op.action === 'add') {
    const m = monitorFor(monitors, op.monitor);
    if (!m) throw new Error('No monitor');
    const spec = createSpec(Math.max(0, ...next.widgets.map(s => s.id)) + 1, op.kind, m.device);
    Object.assign(spec, clampRect(spec.kind, spec, ...dipArea(m)));
    Object.assign(spec, freeSpot(spec, projectWidgets(next, monitors, m.device), ...dipArea(m)));
    next.widgets.push(spec);
  } else if (op.action === 'remove') next.widgets = next.widgets.filter(s => s.id !== op.id);
  else if (op.action === 'patch') next.widgets = next.widgets.map(s => s.id === op.id ? normalizeSpec({ ...s, ...op.patch, id: s.id, kind: s.kind }) : s);
  else throw new Error('Unknown widget operation');
  next.revision++;
  return next;
}
export function weatherDescription(code, day) {
  if (code === 0) return { icon: day ? 'sunny' : 'bedtime', text: 'Açık hava' };
  if (code === 1 || code === 2) return { icon: day ? 'partly_cloudy_day' : 'partly_cloudy_night', text: 'Parçalı bulutlu' };
  if (code === 45 || code === 48) return { icon: 'foggy', text: 'Sisli' };
  if ((code >= 51 && code <= 67) || (code >= 80 && code <= 82)) return { icon: 'rainy', text: 'Yağmurlu' };
  if ((code >= 71 && code <= 77) || code === 85 || code === 86) return { icon: 'weather_snowy', text: 'Karlı' };
  if (code >= 95 && code <= 99) return { icon: 'thunderstorm', text: 'Gök gürültülü' };
  return { icon: 'cloud', text: 'Bulutlu' };
}
