// Same esbuild + fake IPC + Playwright harness as tools/dev/ui-tests.mjs.
// Own temporary outputs; no shared .build or running desktop requests.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const ui = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(path.join(ui, 'package.json'));
const esbuild = require('esbuild');
const { chromium } = require(process.env.LL_PLAYWRIGHT || 'playwright');
const out = fs.mkdtempSync(path.join(os.tmpdir(), 'll-parity-sidebar-'));
const shell = path.join(out, 'shell.mjs'), events = path.join(out, 'events.mjs'), core = path.join(out, 'core.mjs');
fs.writeFileSync(events, `const handlers={}; window.__emit=(n,p)=>(handlers[n]||[]).forEach(f=>f({payload:p}));
export async function listen(n,f){(handlers[n]??=[]).push(f);return()=>handlers[n]=handlers[n].filter(x=>x!==f);}
export async function emit(n,p){window.__emit(n,p);}`);
fs.writeFileSync(shell, `const noop=async()=>{};const win=new Proxy({},{get:()=>noop});
export const currentWidget=()=>({tauriWindow:win});
export const hideWindow=noop,showWindow=noop,reviveOnFocus=noop;
export async function shellSpawn(p,args){window.__spawn.push({p,args});}
export async function shellExec(p,args){window.__calls.push(args);return{stdout:await window.__helper(args)};}
export function createProviderGroup(){return{outputMap:{host:{uptime:123}},onOutput(){}};}`);
fs.writeFileSync(core, `export async function invoke(n,args){window.__ipc.push({n,args});if(n==='screensaver_set')Object.assign(window.__saver,args);return {...window.__saver};}`);
const modulePattern = /<script type="module">\n([\s\S]*?)\n\s*<\/script>/;
for (const name of ['sidebar', 'settings']) {
  const html = fs.readFileSync(path.join(ui, `${name}.html`), 'utf8').replace(/\r\n/g, '\n');
  const contents = html.match(modulePattern)[1];
  // Resolve as the production entry does (ui/.build/<name>.jsx: one level under ui, so '../x' is ui/x) from a
  // directory of that depth that always exists: ui/.build is build.mjs's scratch folder, gone after every build.
  const options = { stdin: { contents, loader: 'jsx', resolveDir: path.join(ui, 'lib') }, bundle: true, format: 'esm', nodePaths: [path.join(ui, 'node_modules')], define: { 'process.env.NODE_ENV': '"production"' }, target: 'chrome120' };
  // Compile actual production dependencies independently as well as the mocked test entry.
  await esbuild.build({ ...options, outfile: path.join(out, `${name}-production.js`), alias: { 'lunge/shell': path.join(ui, 'lib/shell-client.js'), 'lunge/theme': path.join(ui, 'lib/theme.mjs') } });
  await esbuild.build({ ...options, outfile: path.join(out, `${name}.js`), alias: { 'lunge/shell': shell, '@tauri-apps/api/event': events, '@tauri-apps/api/core': core, 'lunge/theme': path.join(ui, 'lib/theme.mjs') } });
  fs.writeFileSync(path.join(out, `${name}.html`), html.replace(modulePattern, `<script type="module" src="./${name}.js"></script>`));
}
const server = http.createServer((req, res) => {
  const name = new URL(req.url, 'http://localhost').pathname.slice(1);
  if (name === 'prefs.json') { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify({ language: 'tr', clock: '24' })); return; }
  const base = fs.existsSync(path.join(out, name)) ? out : ui, file = path.resolve(base, name);
  if (!file.startsWith(base + path.sep) || !fs.existsSync(file)) { res.writeHead(404).end(); return; }
  res.setHeader('Content-Type', { '.js': 'text/javascript', '.html': 'text/html', '.css': 'text/css', '.json': 'application/json', '.woff2': 'font/woff2' }[path.extname(file)] || 'application/octet-stream');
  fs.createReadStream(file).pipe(res);
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.LL_BROWSER_EXECUTABLE ? { executablePath: process.env.LL_BROWSER_EXECUTABLE } : process.env.LL_BROWSER_CHANNEL ? { channel: process.env.LL_BROWSER_CHANNEL } : {}) });
  const page = await browser.newPage({ viewport: { width: 900, height: 1100 } });
  page.setDefaultTimeout(7000);
  const errors = [], requests = [], reports = [];
  page.on('pageerror', e => errors.push(e.message));
  let removeFail = false, scaleFail = false, reportFail = true;
  const removed = new Set(), prefs = { theme: 'dark', uiScale: 100, focusColor: '#b69df8' };
  await page.route('http://127.0.0.1:6131/**', async route => {
    const url = new URL(route.request().url()); requests.push(url);
    let status = 200, body = {};
    if (url.pathname === '/prefs.json') body = prefs;
    else if (url.pathname === '/pref') { status = scaleFail ? 500 : 204; if (!scaleFail) { prefs.uiScale = Number(url.searchParams.get('v')); await page.evaluate(p => Object.assign(window.__prefs, p), prefs); } }
    else if (url.pathname === '/library-remove') { status = removeFail ? 500 : 204; if (!removeFail) { removed.add(url.searchParams.get('path')); await page.evaluate(p => { window.__removed.push(p); window.__saver.choices = window.__saver.choices.filter(c => c.path !== p); }, url.searchParams.get('path')); } }
    else if (url.pathname === '/qs/radios') body = { wifi: 'On', bluetooth: 'On' };
    else if (url.pathname === '/qs/eth') body = { state: 'up' };
    else if (url.pathname === '/qs/bt') body = { adapter: true, devices: [] };
    else if (url.pathname === '/qs/wifi') body = { networks: [] };
    else if (url.pathname === '/notifications') body = { items: [], icons: {} };
    await route.fulfill({ status, contentType: 'application/json', body: status === 204 ? '' : JSON.stringify(body), headers: { 'Access-Control-Allow-Origin': '*' } });
  });
  await page.route('https://bygirolbhyziitvnaxln.supabase.co/**', async route => {
    if (route.request().method() === 'OPTIONS') { await route.fulfill({ status: 204, headers: { 'Access-Control-Allow-Origin': '*', 'Access-Control-Allow-Headers': '*', 'Access-Control-Allow-Methods': 'POST' } }); return; }
    reports.push(route.request().postDataJSON());
    await route.fulfill({ status: reportFail ? 503 : 201, body: reportFail ? 'offline' : '', headers: { 'Access-Control-Allow-Origin': '*' } });
  });
  await page.addInitScript(p => {
    window.__prefs = p; window.__calls = []; window.__spawn = []; window.__ipc = []; window.__removed = [];
    window.__saver = { enabled: true, minutes: 10, secure: true, selected: 'C:\\Users\\test\\AppData\\Local\\LogicalLunge\\screensavers\\custom.scr', choices: [{ path: 'C:\\Windows\\System32\\Mystify.scr', name: 'Windows saver' }, { path: 'C:\\Users\\test\\AppData\\Local\\LogicalLunge\\screensavers\\custom.scr', name: 'Imported saver' }] };
    window.__helper = async args => {
      let r = {};
      if (args[0] === '--settings-get') r = window.__prefs;
      if (args[0] === '--bug-report-device') r = { os: 'Windows test', cpu: 'CPU test', gpu: 'GPU test', ram: '16 GB', appVersion: '0.2.1' };
      if (args[0] === '--bug-report-file') r = args[1] === 'tiling' ? { ok: false, error: 'missing test' } : { ok: true, text: args[1] + ' diagnostic', bytes: 1800 };
      if (args[0] === '--wall-info') r = { monitors: [{ id: 'm1', x: 0, y: 0, w: 1920, h: 1080 }] };
      if (args[0] === '--wall-local' || args[0] === '--live-local') r = Array.from({ length: 20 }, (_, i) => ({ path: 'C:\\library\\' + (args[0] === '--wall-local' ? 'wall' : 'live') + i + (args[0] === '--wall-local' ? '.jpg' : '.mp4'), name: 'Local ' + i })).filter(x => !window.__removed.includes(x.path));
      if (args[0] === '--wall-thumb') return 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
      if (args[0] === '--wall-browse' || args[0] === '--live-store') r = [];
      if (args[0] === '--saver-videos') r = { videos: [] };
      if (args[0] === '--saver-video') r = { ok: true, videos: [] };
      return JSON.stringify(r);
    };
    localStorage.setItem('ll.quickToggles', JSON.stringify(['wifi', 'ethernet', 'bluetooth', 'mic', 'audio', 'nightLight'].map(type => ({ type, size: 1 }))));
  }, prefs);
  const base = `http://127.0.0.1:${server.address().port}`;
  const openSidebar = async () => { await page.goto(`${base}/sidebar.html`); await page.waitForFunction(() => window.__emit); await page.evaluate(() => window.__emit('ll:sidebar-right-toggle')); await page.locator('.panel:not(.closed)').waitFor(); };
  await openSidebar();
  await page.locator('[data-qt="nightLight"].expanded .qt-name').waitFor();
  assert.equal(await page.locator('[data-qt="wifi"] .qt-name').count(), 0);
  await page.evaluate(() => document.querySelector('.panel').style.width = '115px');
  await page.locator('[data-qt="nightLight"].single').waitFor();
  await page.evaluate(() => document.querySelector('.panel').style.width = '450px');
  await page.locator('[data-qt="nightLight"].expanded').waitFor();
  await page.locator('[data-qt="nightLight"] .qt-icon').click();
  await page.waitForFunction(() => window.__calls.some(a => a[0] === '--nightlight' && a[1] === 'toggle'));
  await page.locator('[data-qt="nightLight"] .qt-text').click();
  await page.locator('.qcard').waitFor();
  await page.locator('[data-qt="nightLight"] .qt-text').click();
  await page.locator('.qcard').waitFor({ state: 'detached' });
  await page.getByTitle('Hızlı ayarları düzenle (sürükle: taşı, tık: ekle/kaldır, sağ tık: boyut)', { exact: true }).click();
  await page.locator('[data-qt="nightLight"]').dispatchEvent('wheel', { deltaY: -1 });
  await page.locator('[data-qt="audio"].expanded').first().waitFor();
  assert.equal(await page.locator('[data-qt="nightLight"]').first().locator('.qt-name').count(), 0);
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('ll.quickToggles')).at(-1).type), 'audio');
  await page.locator('[data-qt="nightLight"]').first().click({ button: 'right' });
  await page.locator('[data-qt="nightLight"].expanded').first().waitFor();
  await page.getByTitle('Hızlı ayarları düzenle (sürükle: taşı, tık: ekle/kaldır, sağ tık: boyut)', { exact: true }).click();
  await page.getByTitle('Hata raporu', { exact: true }).click();
  await page.getByLabel('Hatayı açıkla', { exact: true }).fill('Retain this report');
  await page.getByLabel('Bitiş', { exact: true }).fill('');
  await page.getByRole('button', { name: 'Performans', exact: true }).click();
  await page.getByLabel('Cihaz bilgileri', { exact: true }).uncheck();
  await page.getByTitle('Geri', { exact: true }).click();
  await page.locator('.page').waitFor({ state: 'detached' });
  await page.getByTitle('Hata raporu', { exact: true }).click();
  assert.equal(await page.getByLabel('Hatayı açıkla', { exact: true }).inputValue(), 'Retain this report');
  assert.equal(await page.getByLabel('Bitiş', { exact: true }).inputValue(), '');
  assert.equal(await page.getByRole('button', { name: 'Performans', exact: true }).getAttribute('aria-pressed'), 'true');
  assert.equal(await page.getByLabel('Cihaz bilgileri', { exact: true }).isChecked(), false);
  await page.getByRole('button', { name: 'Gönder', exact: true }).click();
  await page.getByRole('dialog', { name: 'Bazı günlükler eklenemedi' }).waitFor();
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('.page.shown').count(), 1);
  await page.getByLabel('tiling.log', { exact: true }).uncheck();
  await page.getByRole('button', { name: 'Gönder', exact: true }).click();
  await page.getByRole('alert').waitFor();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('ll.bug.draft')).message), 'Retain this report');
  reportFail = false;
  await page.getByRole('button', { name: 'Gönder', exact: true }).click();
  await page.getByRole('button', { name: 'Rapor gönderildi', exact: true }).waitFor();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('ll.bug.draft'))), null);
  assert.equal(reports.at(-1).cpu, null); assert.equal(reports.at(-1).tiling_log, null); assert.equal(reports.at(-1).incident_end, null);
  await page.getByTitle('Geri', { exact: true }).click(); await page.locator('.page').waitFor({ state: 'detached' });
  await page.getByTitle('Hata raporu', { exact: true }).click();
  await page.getByLabel('Hatayı açıkla', { exact: true }).fill('Send with missing log');
  await page.getByRole('button', { name: 'Gönder', exact: true }).click();
  await page.getByRole('button', { name: 'Eksik günlüklerle gönder', exact: true }).click();
  await page.getByRole('button', { name: 'Rapor gönderildi', exact: true }).waitFor();
  assert.deepEqual(reports.at(-1).device_info.missingLogs, ['tiling.log']);
  assert.equal(reports.at(-1).cpu, 'CPU test');
  await page.getByTitle('Geri', { exact: true }).click(); await page.locator('.page').waitFor({ state: 'detached' });
  await page.getByTitle('Duvar kağıtları', { exact: true }).click();
  await page.locator('.wall-sec').first().getByRole('button', { name: /Kütüphanem/ }).click();
  const wallRow = page.locator('.wall-sec').first().locator('.wall-row');
  await page.waitForFunction(() => document.querySelector('.wall-row').querySelectorAll('.wall-tile:not(.custom)').length === 20);
  await wallRow.locator('.wall-tile:not(.custom)').first().click({ button: 'right' });
  await page.getByRole('menuitem', { name: /Dosya konumunu aç/ }).click();
  assert.equal(await page.evaluate(() => window.__spawn.at(-1).args[0]), '/select,C:\\library\\wall0.jpg');
  await wallRow.locator('.wall-tile:not(.custom)').first().click({ button: 'right' });
  await page.getByRole('menuitem', { name: /Kütüphaneden kaldır/ }).click();
  await page.getByRole('button', { name: 'Vazgeç', exact: true }).click();
  assert.equal(requests.filter(u => u.pathname === '/library-remove').length, 0);
  await wallRow.locator('.wall-tile:not(.custom)').first().focus(); await page.keyboard.press('Shift+F10');
  await page.getByRole('menuitem', { name: /Kütüphaneden kaldır/ }).click();
  removeFail = true; await page.getByRole('button', { name: 'Kaldır', exact: true }).click(); await page.getByRole('alert').waitFor();
  assert.equal(await wallRow.locator('.wall-tile:not(.custom)').count(), 20);
  removeFail = false; await page.getByRole('button', { name: 'Kaldır', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.wall-row').querySelectorAll('.wall-tile:not(.custom)').length === 19);
  await page.locator('.wall-sec').nth(1).getByRole('button', { name: /Kütüphanem/ }).click();
  const liveRow = page.locator('.wall-sec').nth(1).locator('.wall-row');
  await page.waitForFunction(() => document.querySelectorAll('.wall-row')[1].querySelectorAll('.wall-tile:not(.custom)').length === 20);
  await liveRow.locator('.wall-tile:not(.custom)').first().click({ button: 'right' }); await page.getByRole('menuitem', { name: /Kütüphaneden kaldır/ }).click(); await page.getByRole('button', { name: 'Kaldır', exact: true }).click();
  await page.waitForFunction(() => document.querySelectorAll('.wall-row')[1].querySelectorAll('.wall-tile:not(.custom)').length === 19);
  const saverRow = page.locator('.saver-row-tiles');
  await saverRow.getByTitle('Windows saver', { exact: true }).click({ button: 'right' });
  assert.equal(await page.getByRole('menuitem', { name: /Kütüphaneden kaldır/ }).count(), 0); await page.keyboard.press('Escape');
  await saverRow.getByTitle('Imported saver', { exact: true }).click({ button: 'right' }); await page.getByRole('menuitem', { name: /Kütüphaneden kaldır/ }).click(); await page.getByRole('button', { name: 'Kaldır', exact: true }).click();
  await page.waitForFunction(() => window.__ipc.some(x => x.n === 'screensaver_set' && x.args.enabled === false && x.args.selected === ''));
  assert.deepEqual(requests.filter(u => u.pathname === '/library-remove').map(u => u.searchParams.get('kind')), ['wall', 'wall', 'live', 'saver']);
  await page.goto(`${base}/settings.html`); await page.waitForFunction(() => window.__emit); await page.evaluate(() => window.__emit('ll:settings-toggle'));
  await page.locator('.win.open').waitFor();
  for (const value of [85, 90, 100, 110, 125, 150]) {
    await page.getByRole('button', { name: `${value}%`, exact: true }).click();
    await page.waitForFunction(v => document.querySelector(`[aria-label="Arayüz ölçeği"] button.sel`)?.textContent === `${v}%`, value);
  }
  scaleFail = true; await page.getByRole('button', { name: '85%', exact: true }).click(); await page.getByRole('alert').waitFor();
  assert.equal(await page.getByRole('button', { name: '150%', exact: true }).getAttribute('aria-pressed'), 'true');
  await page.evaluate(() => window.__emit('ll:settings-toggle')); await page.locator('.win:not(.open)').waitFor();
  await page.evaluate(() => window.__emit('ll:settings-toggle')); await page.locator('.win.open').waitFor();
  assert.equal(await page.getByRole('button', { name: '150%', exact: true }).getAttribute('aria-pressed'), 'true');
  assert.deepEqual(errors, []);
  console.log('PASS targeted production esbuild; headless report draft/attachments/failure/success, uncapped galleries, remove confirmation/errors/protected savers, measured toggles/action subregions, six persisted scales/failure');
  console.log(`Isolated outputs: ${out}`);
} finally { await browser?.close(); await new Promise(r => server.close(r)); }
