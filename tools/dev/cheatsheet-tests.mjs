// Kısayol listesi (Super+/, ui/cheatsheet.html) ve ortak etiketler (ui/keymap-labels.mjs). Saf kısım her zaman; --ui ile
// sayfa da gerçek modül betiğiyle, sahte kabuk ve olaylarla başsız tarayıcıda (çalışan masaüstüne hiç bağlanmaz).
//   node tools/dev/cheatsheet-tests.mjs [--ui]     (LL_PLAYWRIGHT: playwright paketinin yolu)
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { coreCategory, tilingCategory, keycaps, fold, cheatEntries, matches, masonry, columnsFor, llLabel } from '../../ui/keymap-labels.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

// ---- saf
assert.equal(coreCategory('ocr', false), 'tools');
assert.equal(coreCategory('color-picker', false), 'tools');
assert.equal(coreCategory('cheatsheet', false), 'tools');
assert.equal(coreCategory('ws-3', false), 'ws');
assert.equal(coreCategory('ws-move-prev', false), 'ws');
assert.equal(coreCategory('focus-left', false), 'win');
assert.equal(coreCategory('close', false), 'win');
assert.equal(coreCategory('browser', true), 'apps');
assert.equal(coreCategory('app:k1', false), 'apps');
assert.equal(coreCategory('run', false), 'apps');
assert.equal(coreCategory('settings', false), 'sys');
assert.equal(tilingCategory(['toggle-floating --centered']), 'layout');
assert.equal(tilingCategory(['resize --width -10%']), 'layout');
assert.equal(tilingCategory(['move --workspace-in-direction left', 'focus --workspace-in-direction left']), 'layout');
assert.equal(tilingCategory(['focus --next-workspace']), 'ws');
assert.equal(tilingCategory(['shell-exec sndvol']), 'media');
assert.equal(tilingCategory(['shell-exec ms-settings:']), 'sys');
assert.equal(tilingCategory(['wm-reload-config']), 'sys');
assert.equal(tilingCategory(['shell-exec wezterm']), 'apps');
assert.equal(tilingCategory(['wm-cycle-focus']), 'win');
assert.deepEqual(keycaps('Super+Ctrl+Left'), ['Super', 'Ctrl', '←']);
assert.deepEqual(keycaps('Super+Shift+/'), ['Super', 'Shift', '/']);
assert.deepEqual(keycaps('Super+PageDown'), ['Super', 'PgDn']);
assert.equal(fold('İzle'), fold('ızle'));
assert.equal(llLabel('ocr'), 'Ekrandan metin (OCR)');
const model = {
  core: [
    { id: 'ocr', combo: 'Super+Shift+T' }, { id: 'cheatsheet', combo: 'Super+/' }, { id: 'ws-2', combo: 'Super+2' },
    { id: 'files', combo: 'Super+E', app: true, removed: true }, { id: 'code', combo: '', app: true },
    { id: 'app:x1', combo: 'Super+Alt+K', app: true, custom: true, name: 'Krita' },
  ],
  tiling: [{ commands: ['toggle-fullscreen'], bindings: ['Super+F', 'F11'] }, { commands: ['wm-redraw'], bindings: [] }],
};
const list = cheatEntries(model);
const labels = list.map(e => e.label);
assert.ok(labels.includes('Ekrandan metin (OCR)') && labels.includes('Krita') && labels.includes('Tam ekran'), 'rows from the model');
assert.ok(!labels.includes('Dosya yöneticisi') && !labels.includes('Kod editörü') && !labels.includes('Pencereleri yeniden çiz'), 'removed and unbound rows are left out');
assert.ok(labels.includes('Arama / overview') && labels.includes('Pencereyi taşı (sürükle)'), 'the fixed rows are there');
assert.deepEqual(list.find(e => e.label === 'Tam ekran').combos, [['Super', 'F'], ['F11']]);
const ocr = list.find(e => e.label === 'Ekrandan metin (OCR)');
assert.ok(matches(ocr, 'metin') && matches(ocr, 'shift t') && matches(ocr, 'EKRAN') && !matches(ocr, 'renk'), 'search by label and keys');
assert.ok(matches(list.find(e => e.label === 'Krita'), 'krı'), 'Turkish i / ı match');
assert.deepEqual(masonry([10, 2, 2, 2], 2), [0, 1, 1, 1]);
assert.deepEqual(masonry([3, 3, 3], 3), [0, 1, 2]);
assert.equal(columnsFor(1180), 3); assert.equal(columnsFor(800), 2); assert.equal(columnsFor(400), 1);
console.log('PASS cheatsheet grouping, key caps, search, model rows and columns');

// ---- sayfa
if (process.argv.includes('--ui')) {
  const require = createRequire(path.join(root, 'ui/package.json'));
  const esbuild = require('esbuild');
  const { chromium } = require(process.env.LL_PLAYWRIGHT || 'playwright');
  const out = fs.mkdtempSync(path.join(root, 'ui', '.cheatsheet-test-'));
  try {
    fs.writeFileSync(path.join(out, 'events.mjs'), `const h = {}; window.__emit = (n, p) => (h[n] || []).forEach(f => f({ payload: p }));
export async function listen(n, f) { (h[n] ??= []).push(f); return () => {}; }
export async function emit(n, p) { window.__emit(n, p); }`);
    fs.writeFileSync(path.join(out, 'shell.mjs'), `const noop = async () => {}; const win = new Proxy({}, { get: () => noop });
export const currentWidget = () => ({ tauriWindow: win });
export const hideWindow = async () => { window.__hidden = (window.__hidden || 0) + 1; }, showWindow = noop, reviveOnFocus = noop, shellSpawn = noop;
export async function shellExec(p, args) { window.__calls = (window.__calls || 0) + 1; return { stdout: JSON.stringify(window.__model) }; }`);
    fs.writeFileSync(path.join(out, 'theme.mjs'), '');
    const html = fs.readFileSync(path.join(root, 'ui/cheatsheet.html'), 'utf8').replace(/\r\n/g, '\n');
    const source = html.match(/<script type="module">\n([\s\S]*?)\n\s*<\/script>/)[1];
    // üretimdeki gibi ui/.build/<ad>.jsx derinliğinden: '../keymap-labels.mjs' = ui/keymap-labels.mjs
    await esbuild.build({ stdin: { contents: source, loader: 'jsx', resolveDir: out }, bundle: true, format: 'esm', outfile: path.join(out, 'app.js'),
      alias: { 'lunge/shell': path.join(out, 'shell.mjs'), '@tauri-apps/api/event': path.join(out, 'events.mjs'), 'lunge/theme': path.join(out, 'theme.mjs') } });
    const page = html.replace(/<script type="module">\n[\s\S]*?\n\s*<\/script>/, '<script type="module" src="/app.js"></script>');
    const server = http.createServer((req, res) => {
      const name = new URL(req.url, 'http://x').pathname.slice(1);
      if (name === 'app.js') { res.setHeader('Content-Type', 'text/javascript'); res.end(fs.readFileSync(path.join(out, 'app.js'))); return; }
      if (name === '' || name === 'cheatsheet.html') { res.setHeader('Content-Type', 'text/html'); res.end(page); return; }
      if (name === 'prefs.json') { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify({ language: 'tr' })); return; }
      const file = path.join(root, 'ui', name);
      if (file.startsWith(path.join(root, 'ui')) && fs.existsSync(file) && fs.statSync(file).isFile()) { res.end(fs.readFileSync(file)); return; }
      res.writeHead(404).end();
    });
    await new Promise(r => server.listen(0, '127.0.0.1', r));
    const browser = await chromium.launch({ headless: true });
    try {
      const p = await browser.newPage({ viewport: { width: 1400, height: 900 } });
      const errors = [];
      p.on('pageerror', e => errors.push(e.message));
      await p.route('http://127.0.0.1:6131/**', r => r.fulfill({ status: 204, headers: { 'Access-Control-Allow-Origin': '*' } }));
      await p.addInitScript(m => { window.__model = m; }, model);
      await p.goto(`http://127.0.0.1:${server.address().port}/cheatsheet.html`);
      await p.evaluate(() => window.__emit('ll:cheatsheet-toggle'));
      await p.waitForFunction(() => document.querySelectorAll('.row').length > 0);
      assert.equal(await p.locator('.col').count(), 3, 'three columns on a wide monitor');
      if (process.env.LL_CHEATSHEET_SHOT) { await p.waitForTimeout(450); await p.screenshot({ path: process.env.LL_CHEATSHEET_SHOT }); }
      const groups = await p.locator('.ghead span:last-child').allTextContents();
      assert.deepEqual(groups.slice().sort(), ['Ekran görüntüsü ve araçlar', 'Pencereler', 'Uygulamalar ve başlatıcı', 'Workspace’ler', 'Yerleşim'].sort(), 'groups with rows only');
      assert.ok(await p.locator('.kc', { hasText: '/' }).count() >= 1, 'Super+/ drawn');
      await p.keyboard.type('ocr');
      assert.deepEqual(await p.locator('.row .label').allTextContents(), ['Ekrandan metin (OCR)'], 'typing filters');
      await p.keyboard.type('zzz');
      assert.equal(await p.locator('#msg').textContent(), 'Eşleşen kısayol yok');
      await p.keyboard.press('Escape');
      assert.equal(await p.inputValue('#q'), '', 'Esc clears the search first');
      assert.ok(await p.evaluate(() => window.__llCheatsheet.open), 'still open after the first Esc');
      await p.keyboard.press('Escape');
      assert.ok(!(await p.evaluate(() => window.__llCheatsheet.open)), 'the second Esc closes');
      await p.evaluate(() => window.__emit('ll:cheatsheet-toggle'));
      await p.waitForFunction(() => window.__llCheatsheet.open);
      await p.mouse.click(5, 5);
      assert.ok(!(await p.evaluate(() => window.__llCheatsheet.open)), 'a click outside the card closes');
      await p.setViewportSize({ width: 700, height: 700 });
      await p.evaluate(() => window.__emit('ll:cheatsheet-toggle'));
      await p.waitForFunction(() => document.querySelectorAll('.row').length > 0);
      assert.equal(await p.locator('.col').count(), 2, 'two columns on a narrow monitor');
      assert.deepEqual(errors, []);
      console.log('PASS cheatsheet page: groups, columns, search, Esc, click outside');
    } finally { await browser.close(); server.close(); }
  } finally { fs.rmSync(out, { recursive: true, force: true }); }
}
