// Pencerenin süreç adına uygulama listesinden (apps.json) simge: bar'ın workspace noktaları ve overview.
//
// Sırayla: exe adı birebir, uygulama adı birebir; sonra en uzun eşleşme, yalnızca anlamlı biçimlerde:
//   - uygulama adı / exe süreç adının öneki ("Steam" -> steamwebhelper)
//   - uygulama adı süreç adının bir kelimesi (süreç adı büyük harf ve ayraçlardan bölünür: "Terminal" ->
//     WindowsTerminal, "WezTerm" -> wezterm-gui)
//   - süreç adı uygulama adının bir kelimesi ("explorer" -> "File Explorer", "chrome" -> "Google Chrome", ama
//     "Monochrome" değil)
// En az 4 harf; genel kelimeler (setup, launcher, client...) tek başına eşleşme sebebi olmaz. Eskiden her iki yönde
// "içeriyor mu" bakılıyordu: "steamwebhelper" ve "leagueclientux" içinde "ea" geçtiği için Steam ve LoL pencereleri
// EA simgesini, "client" exe'li bir uygulama LoL'ün simgesini, "code" CodeBlocks'unkini alıyordu.
const flat = s => (s || '').toLowerCase().replace(/[^a-z0-9]/g, '');
const words = s => (s || '').toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
// "WindowsTerminal" -> windows, terminal; "wezterm-gui" -> wezterm, gui
const procWords = s => words((s || '').replace(/([a-z0-9])([A-Z])/g, '$1 $2'));
const MIN = 4;
const GENERIC = new Set(['setup', 'install', 'installer', 'uninstall', 'launcher', 'client', 'helper', 'service',
  'update', 'updater', 'host', 'app', 'main', 'game', 'games', 'tool', 'tools', 'server', 'runtime', 'manager',
  'windows', 'microsoft', 'system']);
const usable = w => w.length >= MIN && !GENERIC.has(w);

export function appIconFor(apps, proc) {
  const p = (proc || '').toLowerCase();
  if (!p || !apps || !apps.length) return null;
  const fp = flat(p);
  if (!fp) return null;

  for (const a of apps) if (a.icon && a.exe && a.exe === p) return a.icon;
  for (const a of apps) if (a.icon && flat(a.name) === fp) return a.icon;
  if (GENERIC.has(fp)) return null;
  const pw = procWords(proc).filter(usable);

  let best = null, bestScore = 0;
  for (const a of apps) {
    if (!a.icon) continue;
    const name = flat(a.name);
    let score = 0;
    for (const cand of [name, flat(a.exe)]) {
      if (usable(cand) && fp.startsWith(cand)) score = Math.max(score, cand.length * 2);
    }
    if (!score && usable(name) && pw.includes(name)) score = name.length;
    if (!score && usable(fp) && words(a.name).includes(fp)) score = fp.length;
    if (score > bestScore) { bestScore = score; best = a.icon; }
  }
  return best;
}

// Uygulama listesinde karşılığı olmayan pencerenin (Git Bash, oyun istemcisi, kurulum) kendi simgesi: çekirdek
// pencereden ya da exe'sinden verir. Pencere başına bir kez sorulur; gelince onLoad çağrılır (yeniden çizim).
// Alınamazsa 30 sn sonra yeniden denenir.
const winIconCache = new Map();
export function winIconOf(handle, onLoad) {
  if (!handle) return null;
  const k = String(handle);
  if (winIconCache.has(k)) return winIconCache.get(k);
  if (winIconCache.size > 200) winIconCache.clear();
  winIconCache.set(k, null);
  fetch(`http://127.0.0.1:6131/winicon?h=${k}`, { method: 'POST', cache: 'no-store' })
    .then(r => (r.status === 200 ? r.text() : ''))
    .then(t => {
      if (t.startsWith('data:image/')) { winIconCache.set(k, t); if (onLoad) onLoad(); }
      else setTimeout(() => winIconCache.delete(k), 30000);
    })
    .catch(() => setTimeout(() => winIconCache.delete(k), 30000));
  return null;
}
