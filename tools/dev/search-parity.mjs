// Native Super menüsünün arama mantığı (shell/.../native_bar/search.rs) web menüsüyle aynı sonucu veriyor mu:
// ui/overview.html'deki tryMath / fuzzyScore / low node'da çalıştırılır, sonuçlar dosyaya yazılır.
//   node tools/dev/search-parity.mjs parity.json
//   LL_PARITY_JSON=parity.json cargo test --release -p lunge-shell web_parity   (shell klasöründe)
// Girdiler elle seçilmez: hesap makinesi için web'in kendi ad tablolarından ve sözcük türlerinden kısa dizilerin
// tamamı ve rastgele uzun diziler, eşleşme için kullanıcının uygulama adlarından türetilen sorgular.
import fs from 'node:fs';

const html = fs.readFileSync(new URL('../../ui/overview.html', import.meta.url), 'utf8');
const grab = (start, end) => {
  const i = html.indexOf(start);
  const j = html.indexOf(end, i);
  if (i < 0 || j < 0) throw new Error('bulunamadı: ' + start);
  return html.slice(i, j);
};
const lowSrc = grab('      const low = s =>', '\n');
const fuzzySrc = grab('      function fuzzyScore(text, query) {', '      function highlight(');
const mathSrc = grab('      const MATH_FN = {', '      // ii /action');
const window = { LL_LOCALE: 'tr' };
const lib = new Function('window', `${lowSrc}\n${fuzzySrc}\n${mathSrc}\nreturn { low, fuzzyScore, tryMath, MATH_FN, MATH_CONST };`)(window);

// deterministik rastgele
let seed = 12345;
const rnd = n => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; };

// ---- hesap makinesi: sözcük türleri (sayı biçimleri, web'in ad tabloları, işlem karakterleri, boşluk)
const numbers = ['2', '10', '0.5', '2,5', '1e3'];
const names = [...Object.keys(lib.MATH_FN), ...Object.keys(lib.MATH_CONST), 'x'];
const symbols = [...'+-*/%^!(),√×÷ '];
const alphabet = [...numbers, ...names, ...symbols];
const exprs = new Set();
const extend = (prefix, depth) => {
  for (const t of alphabet) {
    exprs.add(prefix + t);
    if (depth > 1) extend(prefix + t, depth - 1);
  }
};
extend('', 3);
for (let k = 0; k < 5000; k++) {
  let s = '';
  for (let t = 4 + rnd(6); t > 0; t--) s += alphabet[rnd(alphabet.length)];
  exprs.add(s);
}
const mathBefore = Object.getOwnPropertyNames(Math).map(k => [k, Math[k]]);
const math = [...exprs].map(input => {
  const m = lib.tryMath(input);
  return { input, out: m === null ? null : String(m) };
});
// Hesap makinesi sayfanın global durumunu değiştirmemeli ("max--" Math.max'i bozuyordu)
const changed = mathBefore.filter(([k, v]) => !Object.is(Math[k], v)).map(([k]) => k);
if (changed.length) throw new Error('tryMath global Math nesnesini değiştirdi: ' + changed.join(', '));

// ---- eşleşme: kullanıcının uygulama adlarından türetilen sorgular
const apps = JSON.parse(fs.readFileSync(process.env.LOCALAPPDATA + '/LogicalLunge/state/apps.json', 'utf8').replace(/^\uFEFF/, ''));
const texts = [...new Set(apps.flatMap(a => [a.name, a.also].filter(Boolean)))];
const pool = new Set();
for (const t of texts) {
  const words = t.split(/[\s\-_.()]+/).filter(Boolean);
  for (let n = 1; n <= 4; n++) pool.add(t.slice(0, n));                               // önek
  pool.add(words.map(w => w[0]).join(''));                                             // baş harfler
  if (words.length > 1) pool.add(words.map(w => w.slice(0, 3)).join(' '));             // kelime başları
  if (t.length > 6) pool.add(t.slice(2, 5));                                           // ortadan parça
  if (t.length >= 5) { const k = 1 + rnd(t.length - 2); pool.add(t.slice(0, k) + t.slice(k + 1)); } // bir harf eksik
}
const poolList = [...pool].filter(q => q.trim());
const queries = [];
for (let k = 0; k < 120; k++) queries.push(poolList[rnd(poolList.length)]);
const fuzzy = [];
for (const q of queries) for (const t of texts) fuzzy.push({ text: t, query: q, score: lib.fuzzyScore(t, q) });

fs.writeFileSync(process.argv[2], JSON.stringify({ math, fuzzy }));
console.log(`hesap ${math.length}, eşleşme ${fuzzy.length} (${queries.length} sorgu x ${texts.length} ad)`);
