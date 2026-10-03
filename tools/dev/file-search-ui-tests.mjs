// Render the real overview with fake IPC. Never touches the running desktop.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const require = createRequire(path.join(root, 'ui/package.json'));
const esbuild = require('esbuild');
const { chromium } = require(process.env.LL_PLAYWRIGHT || 'playwright');
const out = path.join(root, 'build/tests/file-search');
fs.mkdirSync(out, { recursive: true });
const shell = path.join(out, 'shell.mjs'), events = path.join(out, 'events.mjs'), core = path.join(out, 'core.mjs');
fs.writeFileSync(shell, `const noop=async()=>{};
const win=new Proxy({}, {get:(_,name)=>name==='isVisible'?async()=>true:noop});
export const currentWidget=()=>({tauriWindow:win});
export const hideWindow=noop, showWindow=noop, reviveOnFocus=noop, markHidden=noop, setWebviewVisible=noop, shellSpawn=noop;
export async function shellExec(){return {stdout:'[]'};}
export function createProviderGroup(){return {outputMap:{tiling:null},onOutput(){}};}`);
fs.writeFileSync(events, 'export async function listen(){return ()=>{};} export async function emit(){}');
fs.writeFileSync(core, `window.__requests=[];window.__pending=[];
export function invoke(command,args){if(command!=='everything_search_page')return Promise.resolve(null);
window.__requests.push(args);return new Promise(resolve=>window.__pending.push(resolve));}`);
const html = fs.readFileSync(path.join(root, 'ui/overview.html'), 'utf8').replace(/\r\n/g, '\n');
const modulePattern = /<script type="module">\n([\s\S]*?)\n\s*<\/script>/;
await esbuild.build({stdin:{contents:html.match(modulePattern)[1],loader:'jsx',resolveDir:path.join(root,'ui/.build')},bundle:true,format:'esm',outfile:path.join(out,'overview.js'),
  alias:{'lunge/shell':shell,'@tauri-apps/api/event':events,'@tauri-apps/api/core':core,'lunge/apps':path.join(root,'ui/lib/app-icons.js')},nodePaths:[path.join(root,'ui/node_modules')],define:{'process.env.NODE_ENV':'"production"'}});
fs.writeFileSync(path.join(out,'overview.html'), html.replace(modulePattern,'<script type="module" src="./overview.js"></script>'));
const server=http.createServer((req,res)=>{
  const name=new URL(req.url,'http://localhost').pathname.slice(1);
  const base=fs.existsSync(path.join(out,name))?out:path.join(root,'ui');
  const file=path.resolve(base,name);
  if(!file.startsWith(base+path.sep)||!fs.existsSync(file)){res.writeHead(404).end();return;}
  res.setHeader('Content-Type', {'.js':'text/javascript','.html':'text/html','.css':'text/css','.json':'application/json'}[path.extname(file)]||'application/octet-stream');
  fs.createReadStream(file).pipe(res);
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const browser=await chromium.launch({headless:true,...(process.env.LL_BROWSER_EXEC?{executablePath:process.env.LL_BROWSER_EXEC}:process.env.LL_BROWSER_CHANNEL?{channel:process.env.LL_BROWSER_CHANNEL}:{})});
try {
  const page=await browser.newPage({viewport:{width:1400,height:950}}), errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  await page.route('http://127.0.0.1:6131/**',route=>{
    const url=new URL(route.request().url());
    const body=url.pathname==='/overview-mode'?'#':url.pathname==='/apps.json'?'[{"name":"Test","path":"test.exe"}]':url.pathname==='/overview-wait'?'unavailable':'{}';
    return route.fulfill({status:url.pathname==='/overview-wait'?503:200,headers:{'Access-Control-Allow-Origin':'*'},body});
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/overview.html`);
  await page.locator('input').waitFor();
  await page.locator('input').fill('# files');
  await page.waitForFunction(()=>window.__requests.length===1);
  const resolvePage=async(total=70)=>page.evaluate(total=>{
    const {offset}=window.__requests.at(-1);
    window.__pending.shift()({offset,total,hits:Array.from({length:10},(_,i)=>({name:`file-${offset+i}.txt`,fullPath:`C:/files/file-${offset+i}.txt`,isDir:false}))});
  },total);
  await resolvePage();
  await page.waitForFunction(()=>window.__requests.length===2);
  await resolvePage();
  await page.waitForFunction(()=>document.querySelectorAll('.results .item').length===20);
  const scrollBefore=await page.locator('.results').evaluate(list=>{list.scrollTop=list.scrollHeight;return list.scrollTop;});
  await page.waitForFunction(()=>window.__requests.length===3);
  await page.getByRole('status').waitFor();
  await resolvePage();
  await page.waitForFunction(()=>document.querySelectorAll('.results .item').length===30);
  assert.equal(await page.locator('.results').evaluate(list=>list.scrollTop),scrollBefore);
  assert.match(await page.locator('.results .item').first().innerText(),/file-0.txt/);
  for(let count=30;count<70;count+=10){
    await page.locator('.results').evaluate(list=>{list.scrollTop=list.scrollHeight;});
    await page.waitForFunction(count=>window.__requests.length===count/10+1,count);
    await resolvePage();
    await page.waitForFunction(count=>document.querySelectorAll('.results .item').length===count+10,count);
  }
  await page.locator('.results').evaluate(list=>{list.scrollTop=list.scrollHeight;});
  assert.equal(await page.evaluate(()=>window.__requests.length),7);
  assert.deepEqual(errors,[]);
  console.log('Overview UI: bottom loading indicator, 70 appended rows, original first row and unchanged scrollTop passed.');
} finally { await browser.close(); await new Promise(resolve=>server.close(resolve)); }
