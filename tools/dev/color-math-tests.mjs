// The settings colour picker's arithmetic: hex <-> RGB <-> HSV round trips (run by theme-tests.mjs too).
import assert from 'node:assert/strict';
import { hexToRgb, rgbToHex, hsvToRgb, rgbToHsv } from '../../ui/lib/color-math.mjs';

assert.deepEqual(hexToRgb('#b69df8'), [0xb6, 0x9d, 0xf8]);
assert.deepEqual(hexToRgb('B69DF8'), [0xb6, 0x9d, 0xf8]);
assert.deepEqual(hexToRgb(' #00ff00 '), [0, 255, 0]);
for (const bad of ['', '#', '#abc', '#b69df', '#b69df8a', '#gggggg', 'b69df8#', null, undefined]) assert.equal(hexToRgb(bad), null, String(bad));
assert.equal(rgbToHex([0xb6, 0x9d, 0xf8]), '#b69df8');
assert.equal(rgbToHex([0, 0, 0]), '#000000');

assert.deepEqual(hsvToRgb({ h: 0, s: 1, v: 1 }), [255, 0, 0]);
assert.deepEqual(hsvToRgb({ h: 120, s: 1, v: 1 }), [0, 255, 0]);
assert.deepEqual(hsvToRgb({ h: 240, s: 1, v: 1 }), [0, 0, 255]);
assert.deepEqual(hsvToRgb({ h: 360, s: 1, v: 1 }), [255, 0, 0]);
assert.deepEqual(hsvToRgb({ h: 200, s: 0, v: 0.5 }), [128, 128, 128]);
assert.deepEqual(rgbToHsv([255, 0, 0]), { h: 0, s: 1, v: 1 });
// a grey has no hue: the one the picker had stays
assert.equal(rgbToHsv([80, 80, 80], 210).h, 210);
assert.equal(rgbToHsv([0, 0, 0], 33).h, 33);

let cases = 0;
for (let r = 0; r <= 255; r += 3) for (let g = 0; g <= 255; g += 5) for (let b = 0; b <= 255; b += 7) {
  const hsv = rgbToHsv([r, g, b]);
  assert.ok(hsv.h >= 0 && hsv.h < 360, `${[r, g, b]} -> ${JSON.stringify(hsv)}`);
  const back = rgbToHex(hsvToRgb(hsv));
  assert.equal(back, rgbToHex([r, g, b]), `${[r, g, b]} -> ${JSON.stringify(hsv)}`);
  assert.deepEqual(hexToRgb(back), [r, g, b]);
  cases++;
}
console.log(`PASS: colour picker arithmetic (${cases} hex/RGB/HSV round trips, greys keep their hue)`);
