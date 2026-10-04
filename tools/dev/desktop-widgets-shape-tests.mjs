import test from 'node:test';
import assert from 'node:assert/strict';
import { createSpec, KINDS, parseStore, serializeStore, clampRect, dragRect, applyOperation } from '../../ui/desktop-widgets-model.mjs';
const api = await import('../../ui/widget-geometry.mjs').catch(() => ({}));
const shapes = ['card', 'capsule', 'circle', 'ticket', 'bubble', 'hexagon', 'polaroid', 'split'];
const monitors = [{ device: 'DISPLAY1', primary: true, x: 0, y: 40, width: 1280, height: 900, scale: 1.25 }];

test('browsing shapes never accumulates size and remembers a manual resize after persistence', () => {
  for (const [kind, meta] of Object.entries(KINDS)) {
    let spec = createSpec(1, kind);
    const original = [spec.w, spec.h];
    const select = shape => { spec = { ...spec, ...api.selectShape(spec, shape, 1920, 1080, meta.min, meta.size) }; };
    for (let i = 0; i < 5; i++) {
      for (const shape of shapes) select(shape);
      select('card'); assert.deepEqual([spec.w, spec.h], original, `${kind} grew while browsing`);
    }
    select('capsule'); spec.w = 640; spec.h = 192; select('circle');
    spec = parseStore(serializeStore({ widgets: [spec] })).widgets[0];
    select('capsule'); assert.deepEqual([spec.w, spec.h], [640, 192]);
  }
});
test('shape roundtrips independently of material, opacity and location; legacy and unknown default to card', () => {
  const loc = { countryCode: 'TR', country: 'Türkiye', city: 'İstanbul', district: '', latitude: 41, longitude: 29, cityLatitude: 41, cityLongitude: 29 };
  assert.equal(parseStore({ widgets: [{ id: 1, kind: 'clock' }] }).widgets[0].shape, 'card');
  assert.equal(parseStore({ widgets: [{ id: 1, kind: 'weather', shape: 'future' }] }).widgets[0].shape, 'card');
  for (const shape of shapes) {
    const spec = { ...createSpec(1, 'weather'), w: 288, h: 288, shape, appearance: 'outline', backgroundOpacity: .31, contentOpacity: .83, location: loc, recentLocations: [loc] };
    assert.deepEqual(parseStore(serializeStore({ widgets: [spec] })).widgets[0], spec);
    const changed = applyOperation({ widgets: [spec] }, { action: 'patch', id: 1, patch: { appearance: 'glass' } }, monitors).widgets[0];
    assert.equal(changed.shape, shape); assert.equal(changed.contentOpacity, .83); assert.deepEqual(changed.location, loc);
  }
});
test('circle selection expands to a square, clamps to monitor and preserves aspect on pointer resize', () => {
  assert.equal(typeof api.selectShape, 'function', 'shape geometry implementation required');
  const s = { ...createSpec(1, 'weather', 'DISPLAY1'), x: 900, y: 600 };
  const patch = api.selectShape(s, 'circle', 1024, 720, KINDS.weather.min);
  assert.equal(patch.w, 248); assert.equal(patch.h, 248); assert.equal(patch.x, 776); assert.equal(patch.y, 472);
  const resized = dragRect({ ...s, ...patch }, { x: 1200, y: 800 }, { x: 1225, y: 825 }, monitors, true);
  assert.equal(resized.w, resized.h); assert.equal(resized.w % 8, 0);
  assert.deepEqual(clampRect('weather', { ...s, shape: 'circle', w: 900, h: 400 }, 160, 120), { x: 40, y: 0, w: 120, h: 120 });
});
test('every shape gives every kind safe inner/control rectangles at minimum dimensions', () => {
  assert.equal(typeof api.shapeGeometry, 'function');
  for (const shape of shapes) for (const [kind, meta] of Object.entries(KINDS)) {
    const rect = clampRect(kind, { shape, x: 0, y: 0, w: 1, h: 1 }, 1280, 900);
    const g = api.shapeGeometry(shape, rect.w, rect.h);
    assert.ok(g.inner.w > 0 && g.inner.h > 0, `${shape}/${kind}`);
    for (const box of [g.inner, g.toolbar, g.grip]) {
      for (const [x, y] of [[box.x + 1, box.y + 1], [box.x + box.w - 1, box.y + 1], [box.x + 1, box.y + box.h - 1], [box.x + box.w - 1, box.y + box.h - 1]]) {
        if (shape === 'split' && box === g.inner) continue;
        assert.ok(api.regionContains(g.regions, x, y), `${shape}/${kind} unsafe ${JSON.stringify(box)} at ${x},${y}`);
      }
    }
  }
});
test('native region descriptors exclude circle corners, ticket bites, split gap and bubble empty tail edges', () => {
  assert.equal(typeof api.regionContains, 'function');
  const contains = (shape, x, y) => api.regionContains(api.shapeGeometry(shape, 320, 240).regions, x, y);
  assert.equal(contains('circle', 1, 1), false); assert.equal(contains('circle', 160, 120), true);
  assert.equal(contains('capsule', 1, 1), false); assert.equal(contains('capsule', 160, 20), true);
  assert.equal(contains('ticket', 1, 120), false); assert.equal(contains('ticket', 18, 120), true);
  assert.equal(contains('hexagon', 1, 1), false); assert.equal(contains('hexagon', 160, 120), true);
  const split = api.shapeGeometry('split', 320, 240); assert.equal(api.regionContains(split.regions, split.leading + 6, 120), false);
  assert.equal(contains('bubble', 310, 235), false); assert.equal(contains('bubble', 102, 232), true);
  assert.equal(contains('polaroid', 3, 3), false); assert.equal(contains('polaroid', 160, 10), true);
  const scaled = api.physicalRegions(api.shapeGeometry('ticket', 320, 240), { x: 100, y: 50, scaleX: 1.5, scaleY: 1.5 });
  assert.equal(api.regionContains(scaled, 101.5, 230), false); assert.equal(api.regionContains(scaled, 127, 230), true);
});
