import assert from 'node:assert/strict';
import { palette, contrast } from '../../ui/lib/theme-palette.mjs';
for (const color of ['#b69df8', '#8ab4f8', '#7fd4c9', '#a6d189', '#f5a3c7', '#ffb77c', '#f28b82', '#000000', '#ffffff', '#ff0000', '#00ff00', '#0000ff']) {
  for (const light of [false, true]) {
    const p = palette(color, light);
    for (const [fg, bg] of [['m3primary','colLayer0'], ['m3onPrimary','m3primary'], ['m3onPrimaryContainer','m3primaryContainer'], ['m3onSecondaryContainer','m3secondaryContainer'], ['m3onSurface','colLayer1']]) {
      assert.ok(contrast(p[fg], p[bg]) >= 4.5, `${color} ${light} ${fg}/${bg}`);
    }
  }
}
assert.deepEqual(palette('invalid'), palette('#b69df8'));
assert.notDeepEqual(palette('#ff0000'), palette('#00ff00'));
console.log('PASS: 120 theme contrast cases, invalid-color fallback and accent changes');
