// Exercises the real LL menu component with an isolated command boundary.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const require=createRequire(path.join(root,'ui/package.json'));
const {chromium}=require(process.env.LL_PLAYWRIGHT||'playwright');
const esbuild=require('esbuild');
const out=path.join(root,'build/tests/menu');fs.mkdirSync(out,{recursive:true});
await esbuild.build({stdin:{contents:`import React from 'react';import{createRoot}from'react-dom/client';import{ContextMenu}from'./lib/context-menu.jsx';
window.calls=[];window.menuClosed=0;
const items=[{id:'disabled',label:'Disabled',enabled:false},{id:'group',label:'Widgets',children:[{id:'clock',label:'Clock'}]},{id:'error',label:'Failure'}];
createRoot(document.getElementById('root')).render(<ContextMenu items={items} position={{x:9999,y:9999}} onClose={()=>window.menuClosed++} onAction={async id=>{window.calls.push(id);if(id==='error')throw new Error('Try again');}}/>);`,loader:'jsx',resolveDir:path.join(root,'ui')},bundle:true,format:'esm',outfile:path.join(out,'test.js'),nodePaths:[path.join(root,'ui/node_modules')],define:{'process.env.NODE_ENV':'"production"'}});
const server=http.createServer((req,res)=>{
 if(req.url==='/test.js'){res.setHeader('Content-Type','text/javascript');res.end(fs.readFileSync(path.join(out,'test.js')));}
 else if(req.url==='/menu.css'){res.setHeader('Content-Type','text/css');res.end(fs.readFileSync(path.join(root,'ui/menu.css')));}
 else {res.setHeader('Content-Type','text/html');res.end('<!doctype html><link rel="stylesheet" href="/menu.css"><div id="root"></div><script type="module" src="/test.js"></script>');}
});await new Promise(r=>server.listen(0,'127.0.0.1',r));
const browser=await chromium.launch({headless:true});
try{
 const page=await browser.newPage({viewport:{width:500,height:400}}),errors=[];page.on('pageerror',e=>{errors.push(e.message);console.error(e.message);});
 await page.goto(`http://127.0.0.1:${server.address().port}`);await page.getByRole('menu').waitFor({timeout:3000}).catch(async e=>{console.error(await page.content(),errors);throw e;});
 assert.equal(await page.getByRole('button',{name:'Disabled'}).count(),0);
 assert.equal(await page.locator('button').first().isDisabled(),true);
 await page.keyboard.press('ArrowDown');await page.keyboard.press('ArrowRight');
 await page.getByText('Clock',{exact:true}).waitFor();await page.keyboard.press('Enter');
 assert.deepEqual(await page.evaluate(()=>window.calls),['clock']);
 await page.keyboard.press('ArrowLeft');await page.getByText('Failure',{exact:true}).click();
 await page.getByRole('alert').waitFor();assert.equal(await page.getByRole('alert').textContent(),'Try again');
 assert.equal(await page.evaluate(()=>window.menuClosed),1,'failure keeps menu open');
 await page.keyboard.press('Escape');assert.equal(await page.evaluate(()=>window.menuClosed),2);
 const bounds=await page.getByRole('menu').boundingBox();assert.ok(bounds.x>=0&&bounds.x+bounds.width<=500&&bounds.y>=0&&bounds.y+bounds.height<=400);
 assert.deepEqual(errors,[]);console.log('LL menus: disabled actions, keyboard submenus, failed action retention, Escape and screen bounds passed.');
}finally{await browser.close();await new Promise(r=>server.close(r));}
