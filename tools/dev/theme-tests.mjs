import assert from 'node:assert/strict';
import { palette, contrast } from '../../ui/lib/theme-palette.mjs';
const oldDark = {
  m3primary: '#d0bcff', m3onPrimary: '#381e72', m3primaryContainer: '#4f378b',
  colLayer0: '#141218', colLayer1: '#1d1b20', colLayer1Hover: '#2b2930',
  colLayer2: '#2b2930', colBackgroundSurfaceContainer: '#211f26',
};
const oldLight = { m3primary: '#6750a4', colLayer0: '#fef7ff', colLayer1: '#f3edf7' };
for (const [name, expected] of Object.entries(oldDark)) assert.equal(palette('#b69df8')[name], expected, `legacy dark ${name}`);
for (const [name, expected] of Object.entries(oldLight)) assert.equal(palette('#b69df8', true)[name], expected, `legacy light ${name}`);
for (const color of ['#b69df8', '#8ab4f8', '#7fd4c9', '#a6d189', '#f5a3c7', '#ffb77c', '#f28b82', '#123abc', '#000000', '#ffffff', '#ff0000', '#00ff00', '#0000ff']) {
  for (const light of [false, true]) {
    const p = palette(color, light);
    for (const [fg, bg] of [['m3primary','colLayer0'], ['m3onPrimary','m3primary'], ['m3onPrimaryContainer','m3primaryContainer'], ['m3onSecondaryContainer','m3secondaryContainer'], ['m3onSurface','colLayer1']]) {
      assert.ok(contrast(p[fg], p[bg]) >= 4.5, `${color} ${light} ${fg}/${bg}`);
    }
  }
}
assert.deepEqual(palette('invalid'), palette('#b69df8'));
assert.notDeepEqual(palette('#ff0000'), palette('#00ff00'));
assert.notEqual(palette('#123abc').m3primary, palette('#b69df8').m3primary);
console.log('PASS: 130 theme contrast cases, legacy purple, custom accent and invalid-color fallback');
