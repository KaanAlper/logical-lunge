// The token definitions are also included by the native renderer.
import tokens from '../theme-tokens.json' with { type: 'json' };
export const validColor = value => typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value);
const rgb = hex => [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
const mix = (base, seed, amount) => base.map((v, i) => Math.round(v * (1 - amount) + seed[i] * amount));
const hex = color => '#' + color.map(v => v.toString(16).padStart(2, '0')).join('');
export function luminance(color) {
  return rgb(color).map(v => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; })
    .reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0);
}
export const contrast = (a, b) => (Math.max(luminance(a), luminance(b)) + 0.05) / (Math.min(luminance(a), luminance(b)) + 0.05);
export function palette(seed = '#b69df8', light = false) {
  if (!validColor(seed)) seed = '#b69df8';
  const p = rgb(seed), out = {};
  for (const [key, [base, amount]] of Object.entries(tokens[light ? 'light' : 'dark'])) out[key] = hex(mix(rgb(base), p, amount));
  const primary = rgb(out.m3primary), edge = light ? [0, 0, 0] : [255, 255, 255];
  for (let i = 1; contrast(out.m3primary, out.colLayer0) < 4.5 && i <= 100; i++) out.m3primary = hex(mix(primary, edge, i / 100));
  out.m3onPrimary = luminance(out.m3primary) > 0.179 ? '#000000' : '#ffffff';
  return out;
}
