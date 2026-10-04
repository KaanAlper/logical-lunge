import test from 'node:test';
import assert from 'node:assert/strict';
import { parseStore, serializeStore, createSpec, clampRect, freeSpot, overlaps, projectWidgets, dragRect, settleRect, weatherDescription, applyOperation } from '../../ui/desktop-widgets-model.mjs';

const monitors = [
  { device: 'DISPLAY1', primary: true, x: 0, y: 40, width: 1920, height: 1040, scale: 1 },
  { device: 'DISPLAY2', primary: false, x: -2560, y: 0, width: 2560, height: 1440, scale: 2 },
];
test('native layout schema round trips notes and settings; rejects corrupt data', () => {
  const spec = { ...createSpec(7, 'note', 'DISPLAY2'), note: 'süt al\n日本語 📝', city: 'İzmir', fahrenheit: true };
  assert.deepEqual(parseStore(serializeStore({ widgets: [spec] })).widgets, [spec]);
  assert.throws(() => parseStore('not json'));
  assert.throws(() => parseStore('{"widgets":42}'));
  assert.throws(() => parseStore('{"version":99,"widgets":[]}'));
});
test('duplicates, zero IDs and unknown kinds cannot create ghost cards', () => {
  const store = parseStore(JSON.stringify({ widgets: [{ id: 4, kind: 'note' }, { id: 4, kind: 'clock' }, { id: 0 }, { id: 3, kind: 'bad' }] }));
  assert.equal(store.widgets.length, 1);
  assert.equal(store.widgets[0].kind, 'note');
  assert.equal(store.widgets[0].w, 248);
});
test('add places every kind without collisions, remove and patches preserve other monitors', () => {
  let s = { version: 1, widgets: [] };
  for (const kind of ['clock', 'media', 'system', 'weather', 'agenda', 'note']) s = applyOperation(s, { action: 'add', kind, monitor: 'DISPLAY1' }, monitors);
  s.widgets.forEach((a, i) => s.widgets.slice(i + 1).forEach(b => assert.equal(overlaps(a, b), false)));
  s = applyOperation(s, { action: 'add', kind: 'note', monitor: 'DISPLAY2' }, monitors);
  s = applyOperation(s, { action: 'patch', id: 1, patch: { seconds: true, id: 100, kind: 'note' } }, monitors);
  assert.equal(s.widgets[0].id, 1); assert.equal(s.widgets[0].kind, 'clock'); assert.equal(s.widgets[0].seconds, true);
  assert.equal(s.widgets[6].monitor, 'DISPLAY2');
  s = applyOperation(s, { action: 'remove', id: 1 }, monitors);
  assert.equal(s.widgets.length, 6);
});
test('free spot is deterministic and full area has bounded fallback', () => {
  const a = createSpec(1, 'clock', 'DISPLAY1');
  Object.assign(a, freeSpot(a, [], 1920, 1040));
  assert.deepEqual([a.x, a.y], [1632, 24]);
  assert.equal(overlaps({ ...a, ...freeSpot(a, [a], 1920, 1040) }, a), false);
  const p = freeSpot(a, [{ x: 0, y: 0, w: 400, h: 300 }], 400, 300);
  assert.ok(p.x >= 0 && p.x + a.w <= 400 && p.y + a.h <= 300);
});
test('drag uses physical cursor and target DPI, retaining the grab offset', () => {
  const s = { ...createSpec(1, 'clock', 'DISPLAY1'), x: 80, y: 80 };
  const next = dragRect(s, { x: 100, y: 140 }, { x: -2160, y: 200 }, monitors, false);
  assert.equal(next.monitor, 'DISPLAY2');
  assert.deepEqual([next.x, next.y], [184, 80]);
  assert.equal(next.w, s.w);
});
test('resize snaps to eight DIP grid, minimum size and work area boundaries', () => {
  const s = { ...createSpec(1, 'media', 'DISPLAY1'), x: 80, y: 80 };
  const next = dragRect(s, { x: 400, y: 200 }, { x: 417, y: 217 }, monitors, true);
  assert.deepEqual([next.w, next.h], [352, 128]);
  assert.deepEqual(clampRect('media', { x: -20, y: 1000, w: 1, h: 1 }, 1920, 1040), { x: 0, y: 944, w: 240, h: 96 });
  assert.deepEqual(clampRect('note', { x: 0, y: 0, w: 9999, h: 9999 }, 100, 60), { x: 0, y: 0, w: 100, h: 60 });
});
test('disconnected monitors project on primary without overwriting original layouts', () => {
  const s = createSpec(1, 'note', 'DISPLAY2'); s.x = 1000;
  const store = { widgets: [s] };
  const before = JSON.stringify(store);
  const projected = projectWidgets(store, [monitors[0]], 'DISPLAY1');
  assert.equal(projected.length, 1); assert.equal(projected[0].displayMonitor, 'DISPLAY1');
  assert.equal(JSON.stringify(store), before);
  assert.equal(projectWidgets(store, monitors, 'DISPLAY1').length, 0);
  assert.equal(projectWidgets(store, monitors, 'DISPLAY2')[0].x, 1000);
  assert.deepEqual(projectWidgets(store, [], 'DISPLAY1'), []);
});
test('collision settlement respects monitor boundaries and ignores the moving widget', () => {
  const a = createSpec(1, 'clock', 'DISPLAY1'), b = createSpec(2, 'note', 'DISPLAY1');
  const result = settleRect(b, [a, b], 1920, 1040);
  assert.equal(overlaps(result, a), false);
  assert.deepEqual(settleRect(a, [a], 1920, 1040), a);
});
test('weather descriptions preserve day/night and WMO categories', () => {
  assert.equal(weatherDescription(0, false).icon, 'bedtime');
  assert.equal(weatherDescription(95, true).icon, 'thunderstorm');
  assert.equal(weatherDescription(85, true).icon, 'weather_snowy');
});
const location = { countryCode: 'TR', country: 'Türkiye', city: 'İstanbul', district: 'Kadıköy', latitude: 40.99, longitude: 29.03, cityLatitude: 41.01, cityLongitude: 28.98 };
test('old layouts acquire appearance and location defaults without losing legacy city', () => {
  const spec = parseStore({ widgets: [{ id: 1, kind: 'weather', city: 'İzmir' }] }).widgets[0];
  assert.equal(spec.city, 'İzmir'); assert.equal(spec.appearance, 'standard');
  assert.equal(spec.backgroundOpacity, 1); assert.equal(spec.contentOpacity, 1);
  assert.equal(spec.location, null); assert.deepEqual(spec.recentLocations, []);
});
test('all appearances and separate opacity values survive store roundtrip and partial edits', () => {
  for (const appearance of ['standard', 'transparent', 'outline', 'glass', 'futuristic', 'cartoon', 'paper', 'pixel']) {
    const spec = { ...createSpec(1, 'weather'), appearance, backgroundOpacity: .23, contentOpacity: .79, location, recentLocations: [location] };
    assert.deepEqual(parseStore(serializeStore({ widgets: [spec] })).widgets[0], spec);
    const patched = applyOperation({ widgets: [spec] }, { action: 'patch', id: 1, patch: { backgroundOpacity: 0 } }, monitors).widgets[0];
    assert.equal(patched.contentOpacity, .79); assert.deepEqual(patched.location, location);
  }
});
test('unknown style falls back per card and invalid coordinates cannot become a saved place', () => {
  const normalized = raw => parseStore({ widgets: [{ id: 1, kind: 'weather', ...raw }, { id: 2, kind: 'clock' }] });
  const s = normalized({ appearance: 'future-style', backgroundOpacity: -2, contentOpacity: 9, recentLocations: Array(12).fill(location), location });
  assert.equal(s.widgets.length, 2); assert.equal(s.widgets[0].appearance, 'standard');
  assert.equal(s.widgets[0].backgroundOpacity, 0); assert.equal(s.widgets[0].contentOpacity, 1);
  assert.equal(s.widgets[0].recentLocations.length, 1);
  for (const bad of [null, '41', Infinity, NaN, 91]) assert.equal(normalized({ location: { ...location, latitude: bad } }).widgets[0].location, null);
  assert.equal(normalized({ location: { ...location, cityLongitude: 181 } }).widgets[0].location, null);
  assert.equal(normalized({ location: { ...location, city: '' } }).widgets[0].location, null);
  assert.deepEqual(normalized({ recentLocations: [{ ...location, longitude: null }, location] }).widgets[0].recentLocations, [location]);
  assert.equal(normalized({ location: { ...location, countryCode: 'Türkiye' } }).widgets[0].location, null);
  assert.equal(normalized({ location: { ...location, countryCode: 'ΤR' } }).widgets[0].location, null);
  assert.equal(normalized({ location: { ...location, countryCode: 'tr' } }).widgets[0].location.countryCode, 'TR');
  const recents = [location, { ...location, countryCode: 'tr' }, ...Array.from({ length: 10 }, (_, i) => ({ ...location, district: `District ${i}` }))];
  const r = normalized({ recentLocations: recents }).widgets[0].recentLocations;
  assert.equal(r.length, 8); assert.equal(r[1].district, 'District 0'); assert.equal(r[7].district, 'District 6');
});
if (process.argv.includes('--ui')) test('headless desktop surface integration', async () => { await import('./desktop-widgets-ui-tests.mjs'); });
