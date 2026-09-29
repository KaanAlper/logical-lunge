import { listen } from '@tauri-apps/api/event';
import { palette, validColor } from './theme-palette.mjs';

let color = validColor(window.LL_PREFS?.focusColor) ? window.LL_PREFS.focusColor : '#b69df8';
let pending;
let revision = 0, refreshAgain = false;
function apply() {
  const light = document.documentElement.dataset.theme === 'light';
  for (const [key, value] of Object.entries(palette(color, light))) document.documentElement.style.setProperty('--' + key, value);
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
apply();
new MutationObserver(apply).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
window.addEventListener('storage', e => { if (e.key === 'll.accent' && validColor(e.newValue)) { revision++; color = e.newValue; apply(); } });
// Register once for the widget lifetime. Never register inside the callback.
for (const event of ['ll:theme-color', 'll:prefs']) listen(event, reloadTheme).catch(() => {});
reloadTheme();
