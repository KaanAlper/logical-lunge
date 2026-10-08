// Every text the native shell passes to tr("…") must have a row in ui/i18n.json (a missing row shows Turkish in
// every language). Run from the repository root: node tools/dev/i18n-coverage.mjs
// Doc comments (//!, ///) are skipped: their tr("…") calls are examples, not interface text.
import fs from 'node:fs';
import path from 'node:path';

const root = process.cwd();
const i18n = JSON.parse(fs.readFileSync(path.join(root, 'ui', 'i18n.json'), 'utf8').replace(/^﻿/, ''));
const keys = new Set(Array.isArray(i18n.keys) ? i18n.keys : Object.keys(i18n.keys));
const patterns = [...keys].filter(k => k.includes('$1')).map(k => new RegExp('^' + k.split('$1').map(s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('.*') + '$'));

function* rustFiles(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) yield* rustFiles(p);
    else if (e.name.endsWith('.rs')) yield p;
  }
}

const missing = new Map();
for (const file of rustFiles(path.join(root, 'shell', 'packages', 'desktop', 'src'))) {
  const lines = fs.readFileSync(file, 'utf8').split('\n');
  let inTests = false;
  for (const line of lines) {
    if (/^\s*#\[cfg\(test\)\]/.test(line)) inTests = true;
    if (inTests || /^\s*\/\/[/!]/.test(line)) continue;
    for (const m of line.matchAll(/\btr\(\s*"((?:[^"\\]|\\.)*)"\s*\)/g)) {
      const text = m[1].replace(/\\"/g, '"');
      if (!/\p{L}/u.test(text) || keys.has(text) || patterns.some(p => p.test(text))) continue;
      missing.set(text, path.relative(root, file));
    }
  }
}

if (missing.size) {
  console.error(`${missing.size} text(s) without a translation row in ui/i18n.src.js:`);
  for (const [text, file] of missing) console.error(`  ${JSON.stringify(text)}  (${file})`);
  process.exit(1);
}
console.log(`i18n coverage: every tr() text has a row (${keys.size} rows)`);
