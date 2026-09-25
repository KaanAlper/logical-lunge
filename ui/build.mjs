// Logical Lunge widget'larını paketler: node build.mjs <çıktı klasörü>  (kabuğun widget paketi: ui\logical-lunge)
//
// - Her widget'ın modül betiği (JSX dahil) derleme sırasında esbuild ile paketlenir: React, Tauri API'si ve kabuk
//   istemcisi (lib/) uygulamanın içinden gelir. Çalışırken ağdan hiçbir şey yüklenmez, tarayıcıda Babel çalışmaz.
// - Ortak kod (React, istemci) ayrı parçalara bölünür; widget'lar aynı dosyaları paylaşır.
// - Her widget'a aynı dil katmanı (i18n.snippet.js) satır içi eklenir (sayfa çizilmeden önce çalışmalı).
// - Fontlar (fonts/, fonts.css) ve diğer dosyalar olduğu gibi kopyalanır.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import * as esbuild from 'esbuild';

const here = path.dirname(fileURLToPath(import.meta.url));
const out = path.resolve(process.argv[2] ?? path.join(here, 'dist'));
const work = path.join(here, '.build');

// Kaynakta kullanılan ve pakete girmeyen dosyalar
const SKIP = new Set(['build.mjs', 'package.json', 'package-lock.json', 'i18n.src.js', 'i18n.snippet.js']);
const SKIP_DIRS = new Set(['node_modules', 'lib', '.build', 'dist']);

execFileSync(process.execPath, [path.join(here, 'i18n.src.js')], { stdio: 'inherit' });
const snippet = fs.readFileSync(path.join(here, 'i18n.snippet.js'), 'utf8').replace(/\r\n/g, '\n');

fs.rmSync(work, { recursive: true, force: true });
fs.mkdirSync(work, { recursive: true });
fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(out, { recursive: true });

const I18N_BLOCK = /    <script>\n\/\/ Logical Lunge dil katmanı[\s\S]*?    <\/script>/;
const MODULE = /<script type="module">\n([\s\S]*?)\n\s*<\/script>/g;

const pages = [];
const entries = {};
for (const file of fs.readdirSync(here)) {
  if (!file.endsWith('.html')) continue;
  const name = file.slice(0, -5);
  let html = fs.readFileSync(path.join(here, file), 'utf8').replace(/\r\n/g, '\n');
  if (!I18N_BLOCK.test(html)) throw new Error(`${file}: dil katmanı bloğu yok`);
  // replace fonksiyonla: metindeki $ işaretleri değiştirme kalıbı sayılmasın
  html = html.replace(I18N_BLOCK, () => '    <script>\n' + snippet + '    </script>');
  const scripts = [...html.matchAll(MODULE)];
  if (scripts.length !== 1) throw new Error(`${file}: ${scripts.length} modül betiği (1 bekleniyordu)`);
  const entry = path.join(work, `${name}.jsx`);
  fs.writeFileSync(entry, scripts[0][1]);
  entries[name] = entry;
  html = html.replace(MODULE, () => `<script type="module" src="./${name}.js"></script>`);
  if (/https?:\/\/(esm\.sh|unpkg\.com|cdn\.|fonts\.googleapis)/.test(html)) throw new Error(`${file}: ağdan yüklenen kaynak kaldı`);
  pages.push([file, html]);
}

const result = await esbuild.build({
  entryPoints: entries,
  bundle: true,
  splitting: true,
  format: 'esm',
  outdir: out,
  entryNames: '[name]',
  chunkNames: 'lib/[name]-[hash]',
  loader: { '.jsx': 'jsx' },
  jsx: 'transform',
  jsxFactory: 'React.createElement',
  jsxFragment: 'React.Fragment',
  alias: { 'lunge/shell': path.join(here, 'lib', 'shell-client.js') },
  nodePaths: [path.join(here, 'node_modules')],
  define: { 'process.env.NODE_ENV': '"production"' },
  target: 'chrome120',
  minify: true,
  legalComments: 'none',
  logLevel: 'warning',
  metafile: true,
});

for (const [file, html] of pages) fs.writeFileSync(path.join(out, file), html);

// Diğer dosyalar (stiller, çeviriler, paket tanımı, fontlar)
function copy(src, dst) {
  for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
    if (entry.name.startsWith('.')) continue;
    const s = path.join(src, entry.name), d = path.join(dst, entry.name);
    if (entry.isDirectory()) {
      if (src === here && SKIP_DIRS.has(entry.name)) continue;
      fs.mkdirSync(d, { recursive: true });
      copy(s, d);
    } else if (!(src === here && (SKIP.has(entry.name) || entry.name.endsWith('.html')))) {
      fs.copyFileSync(s, d);
    }
  }
}
copy(here, out);
fs.rmSync(work, { recursive: true, force: true });

const sizes = Object.entries(result.metafile.outputs).map(([f, o]) => `${path.relative(out, f).replace(/\\/g, '/')} ${(o.bytes / 1024).toFixed(0)} KB`);
console.log(`widget'lar paketlendi -> ${out}\n  ${sizes.join('\n  ')}`);
