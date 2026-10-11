import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { palette, validColor } from './theme-palette.mjs';

let color = validColor(window.LL_PREFS?.focusColor) ? window.LL_PREFS.focusColor : '#b69df8';
let pending;
let revision = 0, refreshAgain = false;
// Colours from the wallpaper (prefs.json "wallpaperColors"): the shell reads the wallpaper and builds the same
// Material scheme as the native bar and the window borders, both modes ({ dark: {...}, light: {...} }); null while
// the accent colour is in use. Cached for the first paint of the next panel, read again on ll:wall-palette.
const validWall = p => !!p && typeof p === 'object' && ['dark', 'light'].every(m => p[m] && typeof p[m] === 'object'
  && Object.keys(p[m]).length > 0 && Object.values(p[m]).every(validColor));
let wall = null, wallAsked = 0;
try {
  const cached = JSON.parse(localStorage.getItem('ll.wallColors') || 'null');
  if (window.LL_PREFS?.wallpaperColors === true && validWall(cached)) wall = cached;
} catch {}
// Variables set by the last apply: the wallpaper's scheme sets more of them than the accent palette, and those
// return to the stylesheet's own values when the accent colour comes back.
let applied = [];
function apply() {
  const light = document.documentElement.dataset.theme === 'light';
  const next = wall ? wall[light ? 'light' : 'dark'] : palette(color, light), style = document.documentElement.style;
  for (const key of applied) if (!(key in next)) style.removeProperty('--' + key);
  for (const [key, value] of Object.entries(next)) style.setProperty('--' + key, value);
  applied = Object.keys(next);
}
export function applyAccent(value) {
  if (!validColor(value)) return;
  revision++;
  color = value.toLowerCase();
  try { localStorage.setItem('ll.accent', color); } catch {}
  apply();
}
export function reloadTheme() {
  // One request per burst; POST avoids the widget service worker's GET cache.
  if (pending) { refreshAgain = true; return pending; }
  const startedAt = revision;
  return pending = fetch('http://127.0.0.1:6131/prefs.json', { method: 'POST', signal: AbortSignal.timeout(4000) })
    .then(r => { if (!r.ok) throw new Error('Preferences unavailable'); return r.json(); })
    .then(p => { if (revision === startedAt && validColor(p.focusColor)) applyAccent(p.focusColor); })
    .catch(() => {}).finally(() => {
      pending = null;
      if (refreshAgain) { refreshAgain = false; reloadTheme(); }
    });
}
export function reloadWallColors() {
  // The last answer wins: a slow one must not bring back colours that were switched off since.
  const asked = ++wallAsked;
  return invoke('wall_palette')
    .then(p => {
      if (asked !== wallAsked) return;
      wall = validWall(p) ? p : null;
      try { if (wall) localStorage.setItem('ll.wallColors', JSON.stringify(wall)); else localStorage.removeItem('ll.wallColors'); } catch {}
      apply();
    })
    .catch(() => {});
}
apply();
new MutationObserver(apply).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
window.addEventListener('storage', e => { if (e.key === 'll.accent' && validColor(e.newValue)) { revision++; color = e.newValue; apply(); } });
// Register once for the widget lifetime. Never register inside the callback.
for (const event of ['ll:theme-color', 'll:prefs']) listen(event, reloadTheme).catch(() => {});
listen('ll:wall-palette', reloadWallColors).catch(() => {});
reloadTheme();
reloadWallColors();
