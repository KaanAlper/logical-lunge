// Headless widget tests with fake transports. Never connects to the running desktop.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const web = path.resolve(process.argv[2] || root);
const hasBar = fs.existsSync(path.join(web, 'ui/bar.html'));
const require = createRequire(path.join(root, 'ui/package.json'));
const esbuild = require('esbuild');
const { chromium } = require(process.env.LL_PLAYWRIGHT || 'playwright');
const out = path.join(root, 'build/tests/widget-fixtures');
fs.mkdirSync(out, { recursive: true });
const events = path.join(out, 'events.mjs'), shell = path.join(out, 'shell.mjs');
fs.writeFileSync(events, `const handlers = {}; window.__emit = (n,p) => (handlers[n] || []).forEach(f => f({payload:p}));
export async function listen(n, f) { (handlers[n] ??= []).push(f); return () => handlers[n] = handlers[n].filter(x => x !== f); }
export async function emit(n,p) { window.__emit(n,p); }`);
fs.writeFileSync(shell, `import 'lunge/theme';
const noop = async () => {}; const win = new Proxy({}, {get:()=>noop});
export const currentWidget = () => ({tauriWindow:win});
export const hideWindow = noop, showWindow = noop, reviveOnFocus = noop, shellSpawn = noop;
export async function shellExec(p,args=[]) { return {stdout:JSON.stringify(args[0] === '--settings-get' ? window.__prefs : args[0] === '--apps' ? [] : {})}; }
export function createProviderGroup() { const group = { outputMap:window.__data, onOutput(f){window.__output = f;} }; return group; }`);
for (const [name, repo] of [['settings', root], ['bar', web]]) {
  if (!fs.existsSync(path.join(repo, `ui/${name}.html`))) continue;
  fs.mkdirSync(path.join(repo, 'ui/.build'), { recursive: true });
  const html = fs.readFileSync(path.join(repo, `ui/${name}.html`), 'utf8').replace(/\r\n/g, '\n');
  const source = html.match(/<script type="module">\n([\s\S]*?)\n\s*<\/script>/)[1];
  await esbuild.build({ stdin:{contents:source, loader:'jsx', resolveDir:path.join(repo,'ui/.build')}, bundle:true, format:'esm', outfile:path.join(out,`${name}.js`),
    alias:{'lunge/shell':shell, '@tauri-apps/api/event':events, 'lunge/theme':path.join(root,'ui/lib/theme.mjs'), 'lunge/apps':path.join(repo,'ui/lib/app-icons.js')}, nodePaths:[path.join(root,'ui/node_modules')], define:{'process.env.NODE_ENV':'"production"'} });
  fs.writeFileSync(path.join(out,`${name}.html`),html.replace(/<script type="module">\n[\s\S]*?\n\s*<\/script>/,`<script type="module" src="/${name}.js"></script>`));
}
const server = http.createServer((req,res) => {
  const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname).slice(1);
  const repo = req.headers.referer?.includes('bar.html') ? web : root;
  let file = path.resolve(out, pathname);
  if (!file.startsWith(out + path.sep)) {res.writeHead(403).end();return;}
  if (!fs.existsSync(file)) file = path.join(repo,'ui',pathname);
  if (!fs.existsSync(file)) {res.writeHead(404).end();return;}
  const type = {'.js':'text/javascript','.html':'text/html','.css':'text/css','.json':'application/json','.woff2':'font/woff2'}[path.extname(file)] || 'application/octet-stream';
  res.writeHead(200,{'Content-Type':type});fs.createReadStream(file).pipe(res);
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser = await chromium.launch({headless:true,...(process.env.LL_BROWSER_CHANNEL ? {channel:process.env.LL_BROWSER_CHANNEL} : {})});
try {
  const page = await browser.newPage({viewport:{width:1400,height:950}});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  const prefs={language:'tr',clock:'24',theme:'dark',focusColor:'#b69df8',animations:false};
  let fail=false;
  await page.route('http://127.0.0.1:6131/**',async route=>{
    const url=new URL(route.request().url());
    let body=prefs,status=200;
    if (url.pathname==='/focus-color') { status=fail?500:200; if(!fail)prefs.focusColor=url.searchParams.get('v'); body=fail?{ok:false}:{ok:true,focusColor:prefs.focusColor}; }
    else if(url.pathname!=='/prefs.json') body=[];
    await route.fulfill({status,headers:{'Access-Control-Allow-Origin':'*'},contentType:'application/json',body:JSON.stringify(body)});
  });
  await page.addInitScript(p=>{
    window.__prefs=p;
    window.__data={battery:{chargePercent:49,isCharging:true,state:'charging',powerConsumption:25.6,timeTillFull:5400000}};
  },prefs);
  const base=`http://127.0.0.1:${server.address().port}`;
  await page.goto(`${base}/settings.html`);
  await page.waitForFunction(()=>window.__emit);
  await page.evaluate(()=>window.__emit('ll:settings-toggle'));
  await page.locator('.win.open').waitFor();
  await page.getByRole('button',{name:'Mavi',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('[aria-label="Mavi"]').getAttribute('aria-pressed')==='true');
  const blue=await page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--colLayer0'));
  await page.getByRole('button',{name:'Kırmızı',exact:true}).click();
  await page.waitForFunction(()=>document.querySelector('[aria-label="Kırmızı"]').getAttribute('aria-pressed')==='true');
  assert.notEqual(await page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--colLayer0')),blue);
  fail=true;
  await page.getByRole('button',{name:'Yeşil',exact:true}).click();
  await page.getByRole('alert').waitFor();
  assert.equal(await page.getByRole('button',{name:'Kırmızı',exact:true}).getAttribute('aria-pressed'),'true');
  await page.getByRole('button',{name:/Aydınlık/}).click();
  await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');
  await page.waitForFunction(()=>document.querySelector('.seg button.sel')?.textContent.includes('Aydınlık'));
  await page.screenshot({path:path.join(out,'settings-light.png')});
  if(hasBar) {
    await page.goto(`${base}/bar.html`);
    await page.locator('.battery').hover();
    await page.locator('.battery-pop').waitFor();
    const detail=await page.locator('.battery-pop').innerText();
    for(const value of ['49%','1:30','25.6 W']) assert.ok(detail.includes(value),detail);
    assert.equal(await page.locator('.battery').getAttribute('title'),null);
    assert.equal(await page.locator('.battery .label').innerText(),'bolt');
    const pill = await page.locator('.battery').boundingBox(), bolt = await page.locator('.battery .icon').boundingBox();
    assert.ok(Math.abs((bolt.x + bolt.width / 2) - (pill.x + pill.width / 2)) < 1, 'charging icon must stay centered');
    assert.ok(bolt.y >= pill.y && bolt.y + bolt.height <= pill.y + pill.height, 'charging icon must fit the pill');
    await page.screenshot({path:path.join(out,'battery-charging.png')});
    await page.evaluate(()=>{window.__data.battery={chargePercent:18,isCharging:false,state:'discharging',powerConsumption:0,timeTillEmpty:null};window.__output();});
    await page.waitForFunction(()=>document.querySelector('.battery-pop').textContent.includes('Veri yok'));
  }
  assert.deepEqual(errors,[]);
  console.log('PASS headless settings: clickable colors, shell palette, failed save, light mode' + (hasBar ? '; battery hover: units, centered charging icon, unavailable data' : ''));
} finally {await browser.close(); await new Promise(r=>server.close(r));}
