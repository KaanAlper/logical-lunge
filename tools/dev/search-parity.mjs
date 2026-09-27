// Native Super menüsünün arama mantığı (shell/.../native_bar/search.rs) web menüsüyle aynı sonucu veriyor mu:
// ui/overview.html'deki tryMath / fuzzyScore / low node'da çalıştırılır, sonuçlar dosyaya yazılır.
//   node tools/dev/search-parity.mjs parity.json
//   LL_PARITY_JSON=parity.json cargo test --release -p lunge-shell web_parity   (shell klasöründe)
import fs from 'node:fs';

const html = fs.readFileSync(new URL('../../ui/overview.html', import.meta.url), 'utf8');
const grab = (start, end) => {
  const i = html.indexOf(start);
  const j = html.indexOf(end, i);
  if (i < 0 || j < 0) throw new Error('bulunamadı: ' + start);
  return html.slice(i, j);
};
const lowSrc = grab('      const low = s =>', '\n', 0);
const fuzzySrc = grab('      function fuzzyScore(text, query) {', '      function highlight(');
const mathSrc = grab('      const MATH_FN = {', '      // ii /action');
const window = { LL_LOCALE: 'tr' };
const lib = new Function('window', `${lowSrc}\n${fuzzySrc}\n${mathSrc}\nreturn { low, fuzzyScore, tryMath };`)(window);

// hesap makinesi girdileri: elle seçilmiş + üretilmiş
const manual = [
  '2+2', '9', 'sqrt(9)', 'sqrt(9', '√16', '√ 16', '√(1+3)', '√2', '5!', '(2+3)!', '2^10', '2**3**2', '2^-1', '-2^2', '--3', '- -3',
  '50%', '200*10%', '10%3', '50% + 1', '3,5+1', '1,2,3', 'max(1,5)', 'min(3,1,2)', 'max()', '2pi', 'pi2', 'pi pi', '0.1+0.2', '1/3',
  '2 3', '1 000', '1e21*10', '1e-7', '2e3', '2e', 'round(-2.5)', 'round(2.5)', '3×4÷2', 'karekök(16)', 'karekok(16)', '1/0', 'e', 'chrome',
  '2(3)', '(2)(3)', '(2)3', '171!', '170!', '0!', '3.5!', 'sqrt 9', 'sqrt', 'pow(2,10)', 'pow(2)', 'log(1000)', 'ln(e)', 'log2(8)',
  'sin(pi/2)', 'cos(0)', 'tan(pi/4)', 'abs(-3)', 'floor(2.7)', 'ceil(2.1)', 'cbrt(27)', 'exp(1)', 'tau', 'phi', 'π', '2π', 'fact(5)',
  '((2))!', '(2+3', '2+', '*2', '2**', '10%', '10 %', '%5', '5%%', '(1+2)%', '2^3^2', '100!', '1.5.2', '12,34', 'sqrt(16)!', '-5',
  '+5', '-(2)', '2*-3', '2--3', '2- -3', '7/2', '1e308*10', '0.000001', '0.0000001', '123456789012345', '1/7', '2^0.5', '(-8)^(1/3)',
  'asin(2)', 'atan(1)*4', 'sinh(0)', 'cosh(0)', 'tanh(0)', 'log10(100)', 'ln(0)', 'hesap', 'phi*2', '2tau', 'e2', 'e^2', '2e^2',
];
const ops = ['+', '-', '*', '/', '%', '^', '!', '(', ')', ' ', ','];
const atoms = ['1', '2', '3', '10', '0.5', '2,5', 'pi', 'e', 'sqrt(', 'max(', '√', '%'];
let seed = 12345;
const rnd = n => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; };
const gen = [];
for (let k = 0; k < 600; k++) {
  let s = '';
  const len = 1 + rnd(7);
  for (let t = 0; t < len; t++) s += rnd(2) ? atoms[rnd(atoms.length)] : ops[rnd(ops.length)];
  gen.push(s);
}
const math = [...manual, ...gen].map(input => {
  const m = lib.tryMath(input);
  return { input, out: m === null ? null : String(m) };
});

// bulanık eşleşme: gerçek uygulama listesi x sorgular
const apps = JSON.parse(fs.readFileSync(process.env.LOCALAPPDATA + '/LogicalLunge/state/apps.json', 'utf8').replace(/^\uFEFF/, ''));
const names = [...new Set(apps.flatMap(a => [a.name, a.also].filter(Boolean)))];
const queries = ['a', 'ch', 'chr', 'chrome', 'vsc', 'vs code', 'ekr kla', 'ins', 'inst', 'ste', 'steam', 'dis', 'disc', 'word', 'not',
  'notepad', 'gör', 'görev', 'ayar', 'ayarlar', 'set', 'term', 'terminal', 'fire', 'xyzqq', 'ğ', 'ıl', 'İn', 'ph', 'photo', 'ps',
  'pain', 'goog', 'google chrome', 'gc', 'mst', 'micro', 'offi', 'ex', 'excel', 'wd', 'wa', 'whats', 'spot', 'spotify', 'obs', 'lol',
  'league', 'riot', 'epic', 'code', 'visual', 'kontrol', 'denetim', 'cmd', 'komut', 'power', 'pwsh', 'git', 'bash', '442', '3b', '7'];
const fuzzy = [];
for (const q of queries) for (const n of names) fuzzy.push({ text: n, query: q, score: lib.fuzzyScore(n, q) });

fs.writeFileSync(process.argv[2], JSON.stringify({ math, fuzzy }));
console.log(`hesap ${math.length}, eşleşme ${fuzzy.length}`);
