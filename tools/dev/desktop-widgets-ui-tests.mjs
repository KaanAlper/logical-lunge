// Bundles the actual HTML with the production framework, then tests the real
// components headlessly against isolated transports. Never calls installed core.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { applyOperation, createSpec, KINDS, preferredShapeSize } from '../../ui/desktop-widgets-model.mjs';
import { SHAPES, shapeGeometry, regionContains, shapeMinimum } from '../../ui/widget-geometry.mjs';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const require = createRequire(path.join(root, 'ui/package.json'));
const esbuild = require('esbuild');
const { chromium } = require(process.env.LL_PLAYWRIGHT || 'playwright');
const out = path.join(root, `build/tests/desktop-widgets-${process.pid}`);
fs.mkdirSync(out, { recursive: true });
const htmlSource = fs.readFileSync(path.join(root, 'ui/desktop-widgets.html'), 'utf8').replace(/\r\n/g, '\n');
const modulePattern = /<script type="module">\n([\s\S]*?)\n\s*<\/script>/g;
const modules = [...htmlSource.matchAll(modulePattern)];
assert.equal(modules.length, 1, 'build.mjs expects exactly one module');
const i18nPattern = /    <script>\n\/\/ Logical Lunge dil katmanı[\s\S]*?    <\/script>/;
assert.ok(i18nPattern.test(htmlSource), 'build.mjs exact i18n marker');
const html = htmlSource.replace(i18nPattern, () => '    <script>\n' + fs.readFileSync(path.join(root, 'ui/i18n.snippet.js'), 'utf8') + '    </script>');
const build = extra => esbuild.build({ stdin: { contents: modules[0][1].replace('../desktop-widgets-app.mjs', './desktop-widgets-app.mjs'), resolveDir: path.join(root, 'ui'), loader: 'jsx' }, bundle: true, format: 'esm', target: 'chrome120', nodePaths: [path.join(root, 'ui/node_modules')], define: { 'process.env.NODE_ENV': '"production"' }, ...extra });
// Validate real framework/Tauri imports too; output names avoid the shared .build.
await build({ outfile: path.join(out, 'production.js'), minify: true });
const events = path.join(out, 'events.mjs'), shell = path.join(out, 'shell.mjs'), core = path.join(out, 'core.mjs');
fs.writeFileSync(events, `const handlers={}; window.__emit=(n,p)=>(handlers[n]||[]).forEach(f=>f({payload:p}));
export async function listen(n,f){(handlers[n]??=[]).push(f);return()=>handlers[n]=handlers[n].filter(x=>x!==f);}
export async function emit(n,p){window.__events.push([n,p]);window.__emit(n,p);}`);
fs.writeFileSync(shell, `import ${JSON.stringify(path.join(root, 'ui/lib/theme.mjs').replace(/\\/g, '/'))};
const win={label:'widget-test'};
export const currentWidget=()=>({tauriWindow:win});
export async function shellExec(program,args=[]){window.__mediaCalls.push(args);return {stdout:args.length ? '' : window.__art || ''};}
export function createProviderGroup(config){
const outputMap={}; for(const name of Object.keys(config))outputMap[name]=window.__providerData[name];
return {outputMap,errorMap:{},onOutput(){},onError(){},stopAll:async()=>{window.__stopped++;}};}`);
fs.writeFileSync(core, `import {applyOperation} from ${JSON.stringify(path.join(root, 'ui/desktop-widgets-model.mjs').replace(/\\/g, '/'))};
export async function invoke(command,args){window.__commands.push([command,args]);
if(command==='desktop_widgets_bootstrap')return window.__bootstrap;
if(command==='desktop_widgets_cursor')return {...window.__cursor};
if(command==='desktop_widgets_regions'){window.__regions=args.regions;return;}
if(command==='desktop_widgets_editing')return;
if(command==='desktop_widgets_update'){
if(window.__failSave)throw new Error('Test disk write failed');
window.__bootstrap.store=applyOperation(window.__bootstrap.store,args.operation,window.__bootstrap.monitors);
window.__emit('ll:desktop-widgets-store',window.__bootstrap.store);return window.__bootstrap.store;
}return null;}`);
await build({ outfile: path.join(out, 'desktop-widgets.js'), alias: { '@tauri-apps/api/core': core, '@tauri-apps/api/event': events }, plugins: [{ name: 'fixture-shell', setup(build) { build.onResolve({ filter: /shell-client\.js$/ }, () => ({ path: shell })); } }] });
fs.writeFileSync(path.join(out, 'desktop-widgets.html'), html.replace(modulePattern, '<script type="module" src="./desktop-widgets.js"></script>'));
const server = http.createServer((req, res) => {
  const name = decodeURIComponent(new URL(req.url, 'http://localhost').pathname).slice(1);
  const base = fs.existsSync(path.join(out, name)) ? out : path.join(root, 'ui');
  const file = path.resolve(base, name);
  if (!file.startsWith(base + path.sep) || !fs.existsSync(file)) { res.writeHead(404).end(); return; }
  res.setHeader('Content-Type', { '.js': 'text/javascript', '.mjs': 'text/javascript', '.html': 'text/html', '.css': 'text/css', '.json': 'application/json', '.woff2': 'font/woff2' }[path.extname(file)] || 'application/octet-stream');
  fs.createReadStream(file).pipe(res);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const browser = await chromium.launch({ headless: true, executablePath: process.env.LL_CHROMIUM || undefined });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, locale: 'tr-TR' }), errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const monitors = [{ device: 'DISPLAY1', primary: true, x: 0, y: 40, width: 1280, height: 900, scale: 1 }];
  let store = { version: 1, widgets: [] };
  for (const kind of ['clock', 'media', 'system', 'weather', 'agenda', 'note']) store = applyOperation(store, { action: 'add', kind }, monitors);
  let weatherFailed = false, weatherCount = 0, placesFailed = false;
  const lookups = [], forecasts = [];
  await page.route('http://127.0.0.1:6131/**', async route => {
    const url = new URL(route.request().url());
    let status = 200, body = {};
    if (url.pathname === '/prefs.json') body = { language: 'tr', clock: '24', theme: 'dark', uiScale: 100, focusColor: '#7fd4c9' };
    else if (url.pathname === '/widgets/weather') { forecasts.push(Object.fromEntries(url.searchParams)); weatherCount++; status = weatherFailed ? 503 : 200; body = { place: 'İstanbul', temp: 17.4, high: 21, low: 12, code: 2, day: true }; }
    else if (url.pathname === '/widgets/countries') { lookups.push({ route: 'countries', ...Object.fromEntries(url.searchParams) }); body = { results: [{ code: 'TR', name: 'Türkiye', englishName: 'Turkey' }, { code: 'US', name: 'Amerika Birleşik Devletleri', englishName: 'United States' }] }; }
    else if (url.pathname === '/widgets/places') {
      lookups.push({ route: 'places', ...Object.fromEntries(url.searchParams) });
      const district = url.searchParams.get('kind') === 'district', q = url.searchParams.get('q');
      if (q === 'slow') await new Promise(resolve => setTimeout(resolve, 750));
      status = placesFailed ? 503 : 200;
      body = { results: [{ name: district ? 'Kadıköy' : q === 'slow' ? 'Stale city' : 'İstanbul', label: district ? 'Kadıköy, İstanbul' : 'İstanbul, Türkiye', countryCode: 'TR', country: 'Türkiye', city: 'İstanbul', district: district ? 'Kadıköy' : '', latitude: district ? 40.99 : 41.01, longitude: district ? 29.03 : 28.98 }] };
    }
    else if (url.pathname === '/widgets/metrics') body = { cpuTemp: 62, gpuTemp: 49, gpuUsage: 38 };
    await route.fulfill({ status, headers: { 'Access-Control-Allow-Origin': '*' }, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.addInitScript(({ store, monitors }) => {
    window.__events = []; window.__commands = []; window.__regions = []; window.__mediaCalls = []; window.__cursor = { x: 0, y: 0 }; window.__stopped = 0;
    // Retain the real weather poll callbacks so the test can trigger one without
    // waiting fifteen minutes; normal scheduling/cancellation still applies.
    window.__weatherPolls = new Map();
    const schedule = window.setTimeout.bind(window), unschedule = window.clearTimeout.bind(window);
    window.setTimeout = (fn, ms, ...args) => { const id = schedule(fn, ms, ...args); if (ms === 900000) window.__weatherPolls.set(id, fn); return id; };
    window.clearTimeout = id => { window.__weatherPolls.delete(id); unschedule(id); };
    window.__bootstrap = { monitor: 'DISPLAY1', monitors, uiScale: 1, store };
    window.__providerData = { cpu: { usage: 27 }, memory: { usage: 58 }, media: {
      currentSession: { sessionId: 'music-test', title: 'A real provider song', artist: 'Artist', isPlaying: false, positionSeconds: 20, startTime: 0, endTime: 180 },
      previous: async opt => window.__mediaCalls.push(['previous', opt]), next: async opt => window.__mediaCalls.push(['next', opt]), togglePlayPause: async opt => window.__mediaCalls.push(['toggle', opt]),
    } };
    localStorage.setItem('ll.todo', JSON.stringify([{ content: 'Buy milk', done: false }, { content: 'Finished task', done: true }]));
  }, { store, monitors });
  await page.goto(`http://127.0.0.1:${server.address().port}/desktop-widgets.html`);
  await page.waitForFunction(() => document.querySelectorAll('.widget').length === 6 && window.__regions.length === 6);
  await page.getByText('İstanbul', { exact: true }).waitFor();
  assert.equal(forecasts[0].refresh, undefined, 'initial weather load uses the ordinary cache');
  await page.getByText('Buy milk', { exact: true }).waitFor();
  assert.equal(await page.getByText('Finished task').count(), 0);
  assert.match(await page.locator('.system-card').innerText(), /27%/);
  assert.match(await page.locator('.system-card').innerText(), /62°/);
  await page.locator('.widget-media').getByRole('button', { name: 'Oynat', exact: true }).click();
  assert.deepEqual(await page.evaluate(() => window.__mediaCalls.find(a => a[0] === 'toggle')), ['toggle', { sessionId: 'music-test' }]);
  const seek = page.locator('.seek'); await seek.focus(); await seek.press('ArrowRight');
  await page.waitForFunction(() => window.__mediaCalls.some(a => a[0] === '--seek'));
  // the first note by its id: the test adds a second note later
  const noteId = await page.evaluate(() => window.__bootstrap.store.widgets.find(w => w.kind === 'note').id);
  const note = page.locator(`.widget-note[data-widget-id="${noteId}"] .note-editor`); await note.fill('saved note\n日本語 📝');
  await page.waitForFunction(() => window.__bootstrap.store.widgets.find(w => w.kind === 'note').note === 'saved note\n日本語 📝');
  await note.press('Escape');
  // Rendering hit regions matches the six card rectangles and excludes gaps.
  const regions = await page.evaluate(() => window.__regions);
  assert.equal(regions.length, 6); assert.ok(regions.every(r => r.w < 1280 && r.h < 900));
  // Parent contract maps notes at the screen point to native note persistence.
  await page.evaluate(() => window.__emit('ll:desktop-widget-add', { kind: 'notes', x: 300, y: 300 }));
  await page.waitForFunction(() => document.querySelectorAll('.widget-note').length === 2);
  // Actual Pointer Events drag and resize, not direct calls to layout helpers.
  const clock = page.locator('.widget-clock'); let box = await clock.boundingBox();
  await page.evaluate(({ x, y }) => { window.__cursor = { x: x + 30, y: y + 30 + 40 }; }, box);
  await page.mouse.move(box.x + 30, box.y + 30); await page.mouse.down();
  await page.waitForFunction(() => window.__commands.some(c => c[0] === 'desktop_widgets_cursor'));
  await page.evaluate(() => { window.__cursor = { x: 60, y: 600 }; });
  await page.mouse.move(60, 560, { steps: 2 });
  await page.waitForFunction(() => document.querySelector('.widget-clock.dragging'));
  await page.mouse.up();
  await page.waitForFunction(() => window.__commands.some(c => c[0] === 'desktop_widgets_update' && c[1].operation.patch?.monitor));
  const dragged = await page.evaluate(() => window.__bootstrap.store.widgets.find(w => w.kind === 'clock'));
  assert.equal(dragged.x % 8, 0); assert.equal(dragged.y % 8, 0);
  box = await clock.boundingBox();
  const grip = clock.locator('.resize-grip');
  await page.evaluate(({ x, y, width, height }) => { window.__cursor = { x: x + width - 8, y: y + height - 8 + 40 }; }, box);
  await grip.hover(); await page.mouse.down();
  await page.evaluate(() => { window.__cursor.x += 32; window.__cursor.y += 24; });
  await page.mouse.move(box.x + box.width + 24, box.y + box.height + 16, { steps: 2 });
  await page.waitForFunction(() => document.querySelector('.widget-clock.dragging'));
  await page.mouse.up();
  await page.waitForFunction(w => window.__bootstrap.store.widgets.find(s => s.kind === 'clock').w !== w, dragged.w);
  // Weather errors and refresh are interactive and leave other cards available.
  await page.locator('.widget-weather').click({ button: 'right' });
  await page.getByRole('menu').waitFor(); weatherFailed = true;
  await page.getByRole('menuitem', { name: 'Yenile', exact: true }).click();
  await page.getByText('Hava durumu alınamadı', { exact: true }).waitFor();
  assert.equal(forecasts.at(-1).refresh, '1', 'context-menu Yenile bypasses the backend weather cache');
  weatherFailed = false; await page.locator('.widget-weather').getByRole('button', { name: 'Yenile', exact: true }).click();
  await page.getByText('İstanbul', { exact: true }).waitFor(); assert.ok(weatherCount >= 3);
  assert.equal(forecasts.at(-1).refresh, '1', 'retry button also bypasses the backend weather cache');
  const ordinaryPoll = page.waitForResponse(response => { const url = new URL(response.url()); return url.pathname === '/widgets/weather' && !url.searchParams.has('refresh'); });
  await page.evaluate(() => { const [id, run] = [...window.__weatherPolls][0]; clearTimeout(id); run(); });
  await ordinaryPoll;
  assert.equal(forecasts.at(-1).refresh, undefined, 'the manual refresh flag is consumed before the next ordinary poll');
  const weatherCard = page.locator('.widget-weather');
  const openLocation = async () => { await weatherCard.click({ button: 'right' }); await page.getByRole('menuitem', { name: 'Konumu değiştir…', exact: true }).click(); await page.getByRole('dialog', { name: 'Konum' }).waitFor(); };
  await openLocation();
  const popup = page.getByRole('dialog', { name: 'Konum' }), country = popup.getByRole('combobox', { name: 'Ülke', exact: true }), city = popup.getByRole('combobox', { name: 'Şehir', exact: true }), district = popup.getByRole('combobox', { name: 'İlçe (isteğe bağlı)', exact: true });
  assert.ok(await city.isDisabled()); assert.ok(await popup.getByRole('button', { name: 'Kaydet', exact: true }).isDisabled());
  assert.equal(await popup.locator('.location-recents').count(), 0, 'empty history does not reserve a large list');
  assert.equal(await popup.getByRole('button', { name: 'Kapat', exact: true }).count(), 1);
  await popup.screenshot({ path: path.join(out, 'location-picker-empty.png') });
  await country.fill('T'); await popup.getByRole('option', { name: 'Türkiye', exact: true }).waitFor();
  await country.press('ArrowUp'); assert.equal(await popup.getByRole('option', { name: 'Amerika Birleşik Devletleri', exact: true }).getAttribute('aria-selected'), 'true');
  await country.press('ArrowDown'); await country.press('Enter');
  assert.ok(await city.evaluate(el => el === document.activeElement), 'country selection activates the newly enabled city input');
  await city.fill('slow');
  await page.waitForFunction(() => document.querySelector('.location-search-status')?.textContent.includes('Aranıyor'));
  await new Promise(resolve => setTimeout(resolve, 380));
  await city.fill('i'); await popup.getByRole('option', { name: 'İstanbul, Türkiye', exact: true }).waitFor();
  assert.equal(await popup.getByText('Stale city', { exact: true }).count(), 0);
  await popup.getByRole('option', { name: 'İstanbul, Türkiye', exact: true }).click();
  assert.ok(await district.evaluate(el => el === document.activeElement), 'city selection activates the optional district input');
  assert.equal(lookups.find(q => q.kind === 'city' && q.q === 'i').countryCode, 'TR');
  await district.fill('K'); await popup.getByRole('option', { name: 'Kadıköy, İstanbul', exact: true }).waitFor();
  assert.ok(await popup.evaluate(el => { const results=el.querySelector('.location-results'), field=results.parentElement.getBoundingClientRect(), rect=results.getBoundingClientRect(); return getComputedStyle(results).position==='absolute' && (rect.top>=field.bottom || rect.bottom<=field.top) && document.elementFromPoint(rect.left+20,rect.top+20)?.closest('.location-results')===results; }), 'suggestions occupy their own opaque layer next to the input');
  await popup.screenshot({ path: path.join(out, 'location-picker-suggestions.png') });
  assert.equal(await popup.getByRole('link', { name: 'OpenStreetMap contributors', exact: true }).getAttribute('href'), 'https://www.openstreetmap.org/copyright');
  await popup.getByRole('option', { name: 'Kadıköy, İstanbul', exact: true }).click();
  const districtLookup = lookups.find(q => q.kind === 'district');
  assert.equal(districtLookup.city, 'İstanbul'); assert.equal(districtLookup.latitude, '41.01'); assert.equal(districtLookup.longitude, '28.98');
  await district.fill('unsaved typing');
  const beforeBlur = await page.evaluate(() => window.__commands.length);
  await page.evaluate(() => { window.dispatchEvent(new Event('blur')); window.__emit('ll:desktop-widgets-store', window.__bootstrap.store); window.dispatchEvent(new CustomEvent('ll-widget-weather-refresh', { detail: window.__bootstrap.store.widgets.find(s => s.kind === 'weather').id })); });
  await page.getByText('İstanbul', { exact: true }).first().waitFor();
  assert.equal(await district.inputValue(), 'unsaved typing'); assert.ok(await popup.isVisible());
  const blurCommands = await page.evaluate(n => window.__commands.slice(n).filter(c => c[0] === 'desktop_widgets_editing'), beforeBlur);
  assert.equal(blurCommands.at(-1)?.[1]?.editing, false, 'external blur must disable native editing while retaining the popup draft');
  assert.ok(!blurCommands.some(c => c[1].editing), 'external blur/poll must never request native focus');
  assert.ok(await popup.getByRole('button', { name: 'Kaydet', exact: true }).isDisabled());
  await page.evaluate(() => window.__emit('ll:desktop-widgets-host', { monitor: 'DISPLAY1', monitors: [{ device: 'DISPLAY1', primary: true, x: 0, y: 40, width: 400, height: 500, scale: 1.25 }], uiScale: 1.25, active: true }));
  await page.waitForFunction(() => { const b = document.querySelector('.widget-location-popup').getBoundingClientRect(); return b.right <= 401 && b.bottom <= 501 && window.__regions.some(r => Math.abs(r.x - b.x) < 1 && Math.abs(r.h - b.height) < 1); });
  assert.equal(await district.inputValue(), 'unsaved typing');
  await page.evaluate(monitors => window.__emit('ll:desktop-widgets-host', { monitor: 'DISPLAY1', monitors, uiScale: 1, active: true }), monitors);
  await district.fill('K'); await popup.getByRole('option', { name: 'Kadıköy, İstanbul', exact: true }).click();
  // Full popup rectangle remains in HRGN, including search rows outside the card.
  await page.waitForFunction(() => { const b = document.querySelector('.widget-location-popup').getBoundingClientRect(); return window.__regions.some(r => Math.abs(r.x - b.x) < 1 && Math.abs(r.y - b.y) < 1 && Math.abs(r.h - b.height) < 1); });
  await page.evaluate(() => { window.__failSave = true; });
  await popup.getByRole('button', { name: 'Kaydet', exact: true }).click();
  await popup.getByRole('alert').waitFor(); assert.equal(await district.inputValue(), 'Kadıköy');
  await page.evaluate(() => { window.__failSave = false; });
  await popup.getByRole('button', { name: 'Kaydet', exact: true }).click(); await popup.waitFor({ state: 'hidden' });
  await page.waitForFunction(() => window.__bootstrap.store.widgets.find(w => w.kind === 'weather').location?.district === 'Kadıköy');
  await page.waitForFunction(() => document.querySelector('.weather-main'));
  assert.ok(forecasts.some(q => q.latitude === '40.99' && q.longitude === '29.03' && q.place === 'İstanbul / Kadıköy'));
  await openLocation(); assert.equal(await country.inputValue(), 'Türkiye'); assert.equal(await city.inputValue(), 'İstanbul'); assert.equal(await district.inputValue(), 'Kadıköy');
  await popup.screenshot({ path: path.join(out, 'location-picker-saved.png') });
  await country.fill('free country'); await popup.getByRole('button', { name: 'İptal', exact: true }).click();
  await openLocation(); assert.equal(await country.inputValue(), 'Türkiye');
  placesFailed = true; await district.fill('X'); await popup.getByText('Konumlar alınamadı', { exact: true }).waitFor();
  placesFailed = false; await popup.getByRole('button', { name: 'Yeniden dene', exact: true }).click(); await popup.getByRole('option', { name: 'Kadıköy, İstanbul', exact: true }).waitFor();
  placesFailed = true;
  await popup.getByRole('button', { name: 'İstanbul / Kadıköy, Türkiye', exact: true }).click();
  await popup.getByRole('button', { name: 'Kaydet', exact: true }).click(); await popup.waitFor({ state: 'hidden' });
  await weatherCard.click({ button: 'right' }); await page.getByRole('menuitemcheckbox', { name: 'Konumu saat diliminden al', exact: true }).click();
  await page.waitForFunction(() => { const s = window.__bootstrap.store.widgets.find(w => w.kind === 'weather'); return !s.location && !s.city && s.recentLocations.length === 1; });
  await openLocation(); await popup.getByRole('button', { name: 'İstanbul / Kadıköy, Türkiye', exact: true }).click();
  await district.fill(''); await popup.getByRole('button', { name: 'Kaydet', exact: true }).click(); await popup.waitFor({ state: 'hidden' });
  await page.waitForFunction(() => window.__bootstrap.store.widgets.find(w => w.kind === 'weather').location?.latitude === 41.01);
  placesFailed = false;
  // Every style applies to the surface; independent zero opacity leaves tools usable.
  await clock.click({ button: 'right' }); await page.getByRole('menuitem', { name: 'Görünüş', exact: true }).click();
  const styles = [['standard', 'Standart'], ['transparent', 'Şeffaf'], ['outline', 'Konturlu şeffaf'], ['glass', 'Cam'], ['futuristic', 'Fütüristik'], ['cartoon', 'Çizgi film'], ['paper', 'Kağıt'], ['pixel', 'Piksel']];
  for (const [id, label] of styles) { await page.getByRole('menuitemradio', { name: label, exact: true }).click(); await page.waitForFunction(id => document.querySelector('.widget-clock').dataset.appearance === id, id); }
  const background = page.getByRole('slider', { name: 'Arka plan opaklığı', exact: true }), content = page.getByRole('slider', { name: 'İçerik opaklığı', exact: true });
  await background.fill('0.23'); await content.fill('0.79');
  await page.waitForFunction(() => { const s = window.__bootstrap.store.widgets.find(w => w.kind === 'clock'); return s.backgroundOpacity === .23 && s.contentOpacity === .79; });
  assert.deepEqual(await clock.evaluate(el => [getComputedStyle(el, '::before').opacity, getComputedStyle(el.querySelector('.widget-content')).opacity, getComputedStyle(el).opacity]), ['0.23', '0.79', '1']);
  await content.fill('0'); await background.fill('0');
  await page.waitForFunction(() => { const s = window.__bootstrap.store.widgets.find(w => w.kind === 'clock'); return !s.backgroundOpacity && !s.contentOpacity; });
  await page.keyboard.press('Escape'); await clock.getByRole('button', { name: 'Ayarlar', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Görünüş', exact: true }).click(); assert.equal(await background.inputValue(), '0'); assert.equal(await content.inputValue(), '0');
  await background.fill('1'); await content.fill('1'); await page.keyboard.press('Escape');
  await page.getByRole('alert').getByRole('button', { name: 'Kapat', exact: true }).click();
  // Failed writes surface an error while preserving unsaved text.
  await page.evaluate(() => { window.__failSave = true; });
  await note.fill('unsaved draft'); await page.getByRole('alert').waitFor(); assert.equal(await note.inputValue(), 'unsaved draft');
  await page.evaluate(() => { window.__failSave = false; }); await note.press('Escape');
  await page.waitForFunction(() => window.__bootstrap.store.widgets.find(w => w.kind === 'note').note === 'unsaved draft');
  await page.getByRole('alert').getByRole('button', { name: 'Kapat', exact: true }).click();
  // Monitor resize/UI scale updates project into the new DIP area and hit region.
  await page.evaluate(() => window.__emit('ll:desktop-widgets-host', { monitor: 'DISPLAY1', monitors: [{ device: 'DISPLAY1', primary: true, x: 0, y: 40, width: 1280, height: 900, scale: 1.25 }], uiScale: 1.25, active: true }));
  await page.waitForFunction(() => document.getElementById('root').style.zoom === '1.25');
  await page.waitForFunction(() => window.__regions.length === 7);
  await page.screenshot({ path: path.join(out, 'desktop-widgets.png') });
  await page.evaluate(styles => {
    const template = window.__bootstrap.store.widgets.find(s => s.kind === 'note');
    window.__bootstrap.store = { version: 1, revision: window.__bootstrap.store.revision + 1, widgets: styles.map(([appearance, label], i) => ({ ...template, id: i + 100, kind: 'note', appearance, backgroundOpacity: 1, contentOpacity: 1, x: 24 + (i % 4) * 240, y: 24 + Math.floor(i / 4) * 228, w: 216, h: 200,
      note: `${label}\n\nLogical Lunge\nMasaüstü widget\n\nArka plan 100%\nİçerik 100%` })) };
    window.__emit('ll:desktop-widgets-store', window.__bootstrap.store);
  }, styles);
  await page.waitForFunction(() => document.querySelectorAll('.widget').length === 8 && window.__regions.length === 8);
  await page.screenshot({ path: path.join(out, 'eight-appearances.png') });
  // Compare every kind using the same material and actual production components.
  await page.setViewportSize({ width: 1640, height: 850 });
  await page.evaluate(() => window.__emit('ll:desktop-widgets-host', { monitor: 'DISPLAY1', monitors: [{ device: 'DISPLAY1', primary: true, x: 0, y: 40, width: 1640, height: 850, scale: 1 }], uiScale: 1, active: true }));
  for (const kind of Object.keys(KINDS)) {
    const specs = Object.keys(SHAPES).map((shape, i) => {
      const spec = createSpec(200 + i, kind, 'DISPLAY1');
      const [mw, mh] = shapeMinimum(shape, KINDS[kind].min);
      spec.w = Math.max(spec.w, mw); spec.h = Math.max(spec.h, mh);
      if (shape === 'circle') spec.w = spec.h = Math.max(spec.w, spec.h);
      if (shape === 'capsule') spec.w = Math.max(spec.w, spec.h * 1.8);
      return { ...spec, shape, x: 20 + i % 4 * 410, y: 30 + Math.floor(i / 4) * 400,
        note: 'Bugün\nSüt ve ekmek al.\n\nBir fikir: sakin masaüstü.', appearance: 'standard' };
    });
    await page.evaluate(widgets => {
      window.__bootstrap.store = { version: 1, revision: window.__bootstrap.store.revision + 1, widgets };
      window.__emit('ll:desktop-widgets-store', window.__bootstrap.store);
    }, specs);
    await page.waitForFunction(() => document.querySelectorAll('.widget').length === 8);
    if (kind === 'weather') await page.locator('.weather-main').first().waitFor();
    for (const spec of specs) {
      const card = page.locator(`[data-widget-id="${spec.id}"]`);
      await card.waitFor();
      const dimensions = await card.evaluate(el => ({ w: el.offsetWidth, h: el.offsetHeight }));
      const g = shapeGeometry(spec.shape, dimensions.w, dimensions.h);
      const controls = await card.evaluate(el => {
        const base = el.getBoundingClientRect();
        return [...el.querySelectorAll('.widget-tools button, .resize-grip, .media-controls button')].map(control => {
          const r = control.getBoundingClientRect();
          return { x: r.left - base.left, y: r.top - base.top, w: r.width, h: r.height };
        });
      });
      for (const r of controls) for (const [x, y] of [[r.x + 2, r.y + 2], [r.x + r.w - 2, r.y + r.h - 2]])
        assert.ok(regionContains(g.regions, x, y), `${kind}/${spec.shape} clipped control at ${x},${y}`);
    }
    await page.screenshot({ path: path.join(out, `shapes-${kind}.png`) });
  }
  await page.evaluate(widgets => {
    window.__bootstrap.store = { version: 1, revision: window.__bootstrap.store.revision + 1,
      widgets: widgets.map(s => ({ ...s, appearance: 'outline', backgroundOpacity: 0 })) };
    window.__emit('ll:desktop-widgets-store', window.__bootstrap.store);
  }, Object.keys(KINDS).map((kind, i) => ({ ...createSpec(300 + i, kind, 'DISPLAY1'),
    x: 24 + i % 3 * 480, y: 24 + Math.floor(i / 3) * 340, note: 'Unicode note 😀\nYazı konturu' })));
  await page.waitForFunction(() => document.querySelectorAll('[data-appearance="outline"]').length === 6);
  // A provider may insert an icon after the first render. Its halo must track
  // its own colour, including subsequent updates, without a store update.
  await page.evaluate(() => {
    const icon = document.createElement('span'); icon.className = 'icon'; icon.id = 'late-outline-icon';
    icon.style.color = 'rgb(12, 15, 19)'; icon.textContent = 'sunny';
    document.querySelector('.widget-media .widget-content').append(icon);
  });
  await page.waitForFunction(() => document.getElementById('late-outline-icon').style.getPropertyValue('--widget-glyph-outline') === '#ffffff');
  await page.evaluate(() => { document.getElementById('late-outline-icon').style.color = 'rgb(250, 250, 247)'; });
  await page.waitForFunction(() => document.getElementById('late-outline-icon').style.getPropertyValue('--widget-glyph-outline') === '#101016');
  await page.evaluate(() => document.getElementById('late-outline-icon').remove());
  for (const kind of Object.keys(KINDS)) {
    const surface = page.locator(`.widget-${kind}`);
    assert.equal(await surface.evaluate(el => getComputedStyle(el, '::before').borderTopWidth), '0px');
    assert.equal(await surface.evaluate(el => getComputedStyle(el.querySelector('.widget-content')).webkitTextStrokeWidth), '0.65px');
  }
  await page.screenshot({ path: path.join(out, 'glyph-outline-all-kinds.png') });
  for (const [appearance, family] of [['pixel','Pixelify Sans'], ['cartoon','Comic Sans MS'], ['paper','Georgia'], ['futuristic','Consolas']]) {
    const widgets = Object.keys(KINDS).map((kind,i) => ({ ...createSpec(400+i,kind,'DISPLAY1'),
      x:24+i%3*480,y:24+Math.floor(i/3)*340,w:360,h:260,shape:'hexagon',appearance,note:'Çığ, öğle, şüphe, İstanbul 😀' }));
    await page.evaluate(widgets => {
      window.__bootstrap.store={version:1,revision:window.__bootstrap.store.revision+1,widgets};
      window.__emit('ll:desktop-widgets-store',window.__bootstrap.store);
    },widgets);
    await page.waitForFunction(style => document.querySelectorAll(`[data-appearance="${style}"]`).length === 6,appearance);
    const loaded = await page.evaluate(() => document.fonts.load('14px "Pixelify Sans"','Çığ öğle İstanbul'));
    assert.ok(loaded.length > 0,'bundled pixel font loaded');
    for (const kind of Object.keys(KINDS)) {
      const surface=page.locator(`.widget-${kind}`);
      const text = surface.locator({clock:'.clock-time',media:'.media-title',system:'.metric-label',weather:'.weather-main > span:last-child',agenda:'.day-number',note:'.note-editor'}[kind]).first();
      assert.ok((await text.evaluate(el=>getComputedStyle(el).fontFamily)).startsWith(family) || (await text.evaluate(el=>getComputedStyle(el).fontFamily)).startsWith(`"${family}"`),`${kind}/${appearance} text family`);
      assert.equal(await surface.locator('.widget-tools .icon').evaluate(el=>getComputedStyle(el).fontFamily),'"Material Symbols Rounded"');
    }
    await page.screenshot({path:path.join(out,`fonts-${appearance}-all-kinds.png`)});
  }
  // Oversized ticket/bubble must keep the weather group together, too.
  for (const shape of ['ticket','bubble']) {
    await page.evaluate(shape=>{
      const spec={...window.__bootstrap.store.widgets.find(s=>s.kind==='weather'),shape,appearance:'standard',x:24,y:24,w:545,h:287};
      window.__bootstrap.store.widgets=[spec];window.__emit('ll:desktop-widgets-store',window.__bootstrap.store);
    },shape);
    await page.waitForFunction(shape=>document.querySelector('.widget-weather')?.dataset.shape === shape,shape);
    const span=await page.locator('.weather-card').evaluate(el=>{
      const boxes=[...el.querySelectorAll('.weather-main > span'),...el.querySelectorAll(':scope > div:not(.weather-main)')].map(c=>c.getBoundingClientRect());return Math.max(...boxes.map(b=>b.bottom))-Math.min(...boxes.map(b=>b.top));
    });
    assert.ok(span > 100 && span < 240,`${shape} weather content must grow and stay grouped, span ${span}px`);
    assert.ok(await page.locator('.weather-main > span:last-child').evaluate(el=>el.getBoundingClientRect().height) > 48,'temperature scales with widget dimensions');
    const content=await page.locator('.weather-card').evaluate(el=>{
      const outer=el.parentElement.getBoundingClientRect(),card=el.getBoundingClientRect();
      return {outerWidth:outer.width,cardWidth:card.width,outerHeight:outer.height,cardHeight:card.height};
    });
    assert.ok(Math.abs(content.cardWidth-content.outerWidth)<2 && Math.abs(content.cardHeight-content.outerHeight)<2,'scaled weather still fills the correct content box');
    await page.locator('.widget-weather').screenshot({path:path.join(out,`weather-oversized-${shape}.png`)});
    const [w,h]=preferredShapeSize('weather',shape);
    await page.evaluate(({w,h})=>{
      const spec=window.__bootstrap.store.widgets[0]; spec.w=w; spec.h=h;
      window.__emit('ll:desktop-widgets-store',window.__bootstrap.store);
    },{w,h});
    await page.waitForFunction(w=>Math.abs(document.querySelector('.widget-weather').getBoundingClientRect().width-w)<2,w);
    await page.locator('.widget-weather').screenshot({path:path.join(out,`weather-compact-${shape}.png`)});
  }
  assert.deepEqual(errors, []);
  console.log(`PASS desktop widgets headless: location editor, Save/Cancel/recents, blur/poll retention, drag/resize/DPI regions, all eight shapes for six kinds with controls inside silhouettes, glyph outline and late icon colour updates, independent opacity. Screenshots: ${out}`);
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
