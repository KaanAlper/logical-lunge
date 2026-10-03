// Bundles the actual HTML with the production framework, then tests the real
// components headlessly against isolated transports. Never calls installed core.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { applyOperation } from '../../ui/desktop-widgets-model.mjs';
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
const build = extra => esbuild.build({ stdin: { contents: modules[0][1], resolveDir: path.join(root, 'ui/.build'), loader: 'jsx' }, bundle: true, format: 'esm', target: 'chrome120', nodePaths: [path.join(root, 'ui/node_modules')], define: { 'process.env.NODE_ENV': '"production"' }, ...extra });
// Validate real framework/Tauri imports too; output names avoid the shared .build.
await build({ outfile: path.join(out, 'production.js'), minify: true });
const events = path.join(out, 'events.mjs'), shell = path.join(out, 'shell.mjs'), core = path.join(out, 'core.mjs');
fs.writeFileSync(events, `const handlers={}; window.__emit=(n,p)=>(handlers[n]||[]).forEach(f=>f({payload:p}));
export async function listen(n,f){(handlers[n]??=[]).push(f);return()=>handlers[n]=handlers[n].filter(x=>x!==f);}
export async function emit(n,p){window.__events.push([n,p]);window.__emit(n,p);}`);
fs.writeFileSync(shell, `import ${JSON.stringify(path.join(root, 'ui/lib/theme.mjs').replace(/\\/g, '/'))};
const win={label:'widget-test'};
export const currentWidget=()=>({tauriWindow:win});
export async function shellExec(program,args=[]){window.__mediaCalls.push(args);return {stdout:''};}
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
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, locale: 'tr-TR' }), errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const monitors = [{ device: 'DISPLAY1', primary: true, x: 0, y: 40, width: 1280, height: 900, scale: 1 }];
  let store = { version: 1, widgets: [] };
  for (const kind of ['clock', 'media', 'system', 'weather', 'agenda', 'note']) store = applyOperation(store, { action: 'add', kind }, monitors);
  let weatherFailed = false, weatherCount = 0;
  await page.route('http://127.0.0.1:6131/**', async route => {
    const url = new URL(route.request().url());
    let status = 200, body = {};
    if (url.pathname === '/prefs.json') body = { language: 'tr', clock: '24', theme: 'dark', uiScale: 100, focusColor: '#7fd4c9' };
    else if (url.pathname === '/widgets/weather') { weatherCount++; status = weatherFailed ? 503 : 200; body = { place: 'İstanbul', temp: 17.4, high: 21, low: 12, code: 2, day: true }; }
    else if (url.pathname === '/widgets/metrics') body = { cpuTemp: 62, gpuTemp: 49, gpuUsage: 38 };
    await route.fulfill({ status, headers: { 'Access-Control-Allow-Origin': '*' }, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.addInitScript(({ store, monitors }) => {
    window.__events = []; window.__commands = []; window.__regions = []; window.__mediaCalls = []; window.__cursor = { x: 0, y: 0 }; window.__stopped = 0;
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
  weatherFailed = false; await page.locator('.widget-weather').getByRole('button', { name: 'Yenile', exact: true }).click();
  await page.getByText('İstanbul', { exact: true }).waitFor(); assert.ok(weatherCount >= 3);
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
  assert.deepEqual(errors, []);
  console.log(`PASS desktop widgets headless: real esbuild imports, six live cards, media seek, notes/errors, parent add event, drag/resize, weather retry, monitor scale, native hit regions. Screenshot: ${out}/desktop-widgets.png`);
} finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
