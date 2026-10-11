// The custom accent's colour picker arithmetic: #rrggbb <-> [r, g, b] <-> { h, s, v } (hue in degrees,
// saturation and value 0..1). Plain functions, tested by tools/dev/color-math-tests.mjs.

// '#rrggbb' or 'rrggbb' (any case) -> [r, g, b]; null for anything else
export function hexToRgb(text) {
  const m = /^#?([0-9a-f]{6})$/i.exec(String(text ?? '').trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [n >> 16 & 255, n >> 8 & 255, n & 255];
}

// lower case, what the core stores
export const rgbToHex = rgb => '#' + rgb.map(c => c.toString(16).padStart(2, '0')).join('');

const unit = x => Math.min(1, Math.max(0, x));

export function hsvToRgb({ h, s, v }) {
  const k = ((h % 360) + 360) % 360 / 60;
  s = unit(s); v = unit(v);
  const chroma = v * s, x = chroma * (1 - Math.abs(k % 2 - 1)), m = v - chroma;
  const [r, g, b] = [[chroma, x, 0], [x, chroma, 0], [0, chroma, x], [0, x, chroma], [x, 0, chroma], [chroma, 0, x]][Math.min(5, Math.floor(k))];
  return [r, g, b].map(c => Math.min(255, Math.max(0, Math.round((c + m) * 255))));
}

// `hue` stays when the colour has none (a grey): the picker's hue bar does not jump back to red
export function rgbToHsv([r, g, b], hue = 0) {
  [r, g, b] = [r / 255, g / 255, b / 255];
  const max = Math.max(r, g, b), d = max - Math.min(r, g, b);
  let h = hue;
  if (d > 0) h = max === r ? 60 * ((((g - b) / d) % 6 + 6) % 6) : max === g ? 60 * ((b - r) / d + 2) : 60 * ((r - g) / d + 4);
  return { h, s: max > 0 ? d / max : 0, v: max };
}
