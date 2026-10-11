// Kısayolların adları ve grupları: sağ paneldeki kısayol düzenleyicisi (sidebar.html) ve kısayol listesi (cheatsheet.html,
// Super+/) aynı tabloyu kullanır. Model çekirdeğin `lunge.exe --keybinds-model` çıktısı. Metinler Türkçe kaynak, ekranda
// dil katmanı (i18n.json) çevirir. native-ui'daki karşılığı: native_bar/sidebar/keys.rs ve native_bar/cheatsheet.rs.

export const LL_LABELS = {
  'focus-left': 'Sola odaklan', 'focus-right': 'Sağa odaklan', 'focus-up': 'Yukarı odaklan', 'focus-down': 'Aşağı odaklan',
  'move-left': 'Pencereyi sola taşı', 'move-right': 'Pencereyi sağa taşı', 'move-up': 'Pencereyi yukarı taşı', 'move-down': 'Pencereyi aşağı taşı',
  'ws-prev': 'Önceki workspace', 'ws-next': 'Sonraki workspace', 'ws-prev5': '5 workspace geri', 'ws-next5': '5 workspace ileri',
  'ws-move-prev': "Pencereyi önceki workspace'e taşı", 'ws-move-next': "Pencereyi sonraki workspace'e taşı",
  terminal: 'Terminal', 'terminal-alt': 'Terminal (ikinci kısayol)', browser: 'Tarayıcı', files: 'Dosya yöneticisi',
  code: 'Kod editörü', editor: 'Metin editörü', close: 'Pencereyi kapat', screenshot: 'Ekran alıntısı', 'screenshot-screen': 'Monitörün tamamı (panoya)', clipboard: 'Pano geçmişi',
  'file-search': 'Dosya araması', 'screenshot-alt': 'Ekran alıntısı (ikinci kısayol)', 'overview-alt': 'Super menüsü',
  run: 'Çalıştır', search: 'Ara (Super menüsü)', workspaces: "Workspace'ler (Super menüsü)", settings: 'Ayarlar',
  sidebar: 'Sağ panel', notifications: 'Bildirimler', 'task-manager': 'Görev Yöneticisi', 'focus-urgent-or-last': 'Dikkat isteyen pencereye git',
  cheatsheet: 'Kısayol listesi', ocr: 'Ekrandan metin (OCR)', 'color-picker': 'Renk seçici (ekrandan)',
};

export const llLabel = id => LL_LABELS[id]
  || (/^ws-move-\d+$/.test(id) ? `Pencereyi workspace ${id.slice(8)}'e taşı` : null)
  || (id.startsWith('ws-') ? `Workspace ${id.slice(3)}` : id);

// Düzenleyicinin grupları (win / ws / app / sys)
export const llGroup = id => id.startsWith('focus-') || id.startsWith('move-') || id === 'close' ? 'win'
  : (id.startsWith('ws-') || id === 'workspaces') ? 'ws' : 'sys';

const TILING = {
  'focus --prev-workspace': 'Önceki workspace (PageUp)', 'focus --next-workspace': 'Sonraki workspace (PageDown)',
  'focus --prev-active-workspace': 'Önceki dolu workspace', 'focus --next-active-workspace': 'Sonraki dolu workspace',
  'move --prev-workspace ; focus --prev-workspace': "Pencereyi önceki workspace'e taşı (PageUp)",
  'move --next-workspace ; focus --next-workspace': "Pencereyi sonraki workspace'e taşı (PageDown)",
  'toggle-special-workspace': 'Gizli workspace’i aç / kapat', 'move-to-special-workspace': 'Pencereyi gizli workspace’e gönder',
  'toggle-pin': 'Pencereyi sabitle (her workspace’te)',
  'toggle-fullscreen': 'Tam ekran', 'toggle-fullscreen-spoof': 'Yuvasında tam ekran (spoof)', 'toggle-floating --centered': 'Yüzen pencere', 'toggle-tiling-direction': 'Bölme yönünü değiştir',
  'toggle-minimized': 'Simge durumuna küçült', 'wm-cycle-focus': 'Yüzen / döşeli pencereler arasında geç',
  'resize --width -10%': 'Pencereyi daralt', 'resize --width +10%': 'Pencereyi genişlet', 'wm-enable-binding-mode --name resize': 'Boyutlandırma modu',
  'split-ratio -0.1': 'Bölme oranını azalt', 'split-ratio 0.1': 'Bölme oranını artır',
  'focus --prev-active-workspace-on-monitor': 'Bu monitörde önceki dolu workspace', 'focus --next-active-workspace-on-monitor': 'Bu monitörde sonraki dolu workspace',
  'focus --workspace-in-direction left': 'Soldaki monitöre geç', 'focus --workspace-in-direction right': 'Sağdaki monitöre geç',
  'move --workspace-in-direction left ; focus --workspace-in-direction left': 'Pencereyi soldaki monitöre gönder',
  'move --workspace-in-direction right ; focus --workspace-in-direction right': 'Pencereyi sağdaki monitöre gönder',
  'move-workspace --direction left': "Workspace'i soldaki monitöre taşı", 'move-workspace --direction right': "Workspace'i sağdaki monitöre taşı",
  'shell-exec ms-settings:': 'Windows ayarları', 'shell-exec sndvol': 'Ses karıştırıcı',
  'wm-reload-config': 'Pencere yöneticisi ayarlarını yenile', 'wm-redraw': 'Pencereleri yeniden çiz', 'wm-toggle-pause': 'Döşemeyi duraklat', 'wm-exit': "Logical Lunge'dan çık",
};

export function tilingLabel(cmds) {
  const c = cmds.join(' ; ');
  const m = c.match(/^move --workspace (\d+)$/);
  if (m) return `Pencereyi workspace ${m[1]}'e gönder`;
  if (TILING[c]) return TILING[c];
  if (/wezterm/i.test(c)) return 'Terminal (yedek)';
  return c;
}

export const tilingGroup = cmds => {
  const c = cmds.join(' ');
  if (/workspace/.test(c)) return 'ws';
  if (/shell-exec/.test(c)) return 'app';
  if (/^wm-(reload|redraw|toggle-pause|exit)/.test(c)) return 'sys';
  return 'win';
};

export const KEY_GLYPH = { Left: '←', Right: '→', Up: '↑', Down: '↓', Enter: '↵', PageUp: 'PgUp', PageDown: 'PgDn', Escape: 'Esc' };

// ---- Kısayol listesi (ii cheatsheet): gruplar gösterildikleri sırayla
export const CATS = [
  ['ws', 'Workspace’ler', 'view_carousel'],
  ['win', 'Pencereler', 'select_window'],
  ['layout', 'Yerleşim', 'dashboard'],
  ['apps', 'Uygulamalar ve başlatıcı', 'apps'],
  ['tools', 'Ekran görüntüsü ve araçlar', 'screenshot_region'],
  ['media', 'Medya', 'music_note'],
  ['sys', 'Sistem', 'settings'],
];

const TOOLS = new Set(['screenshot', 'screenshot-alt', 'screenshot-screen', 'ocr', 'color-picker', 'cheatsheet']);
const LAUNCH = new Set(['overview', 'overview-alt', 'search', 'run', 'file-search', 'clipboard', 'terminal', 'terminal-alt']);

export function coreCategory(id, app) {
  if (app || id.startsWith('app:')) return 'apps';
  if (TOOLS.has(id)) return 'tools';
  if (LAUNCH.has(id)) return 'apps';
  if (id === 'workspaces') return 'ws';
  if (id === 'close' || id === 'focus-urgent-or-last' || id.startsWith('focus-') || id.startsWith('move-')) return 'win';
  if (id.startsWith('ws-')) return 'ws';
  if (id.includes('media') || id.includes('volume')) return 'media';
  return 'sys';
}

export function tilingCategory(cmds) {
  const c = cmds.join(' ; ').toLowerCase();
  const has = s => c.includes(s);
  if (has('sndvol') || has('media') || has('volume') || has('audio')) return 'media';
  if (has('ms-settings') || ['wm-reload', 'wm-redraw', 'wm-toggle-pause', 'wm-exit'].some(p => c.startsWith(p))) return 'sys';
  if (has('shell-exec')) return 'apps';
  if (has('workspace-in-direction') || c.startsWith('move-workspace')) return 'layout';
  if (has('workspace')) return 'ws';
  if (['toggle-floating', 'toggle-fullscreen', 'toggle-tiling-direction', 'resize', 'split-ratio', 'toggle-pin', 'binding-mode'].some(has)) return 'layout';
  return 'win';
}

// "Super+Ctrl+Left" -> ["Super", "Ctrl", "←"]
export const keycaps = combo => String(combo || '').split('+').map(k => k.trim()).filter(Boolean).map(k => KEY_GLYPH[k] || k);

// Arama için küçük harf; Türkçe I'lar bir: "İ" -> "i" (+ birleşen nokta atılır), "ı" -> "i"
export const fold = s => String(s || '').toLowerCase().replace(/̇/g, '').replace(/ı/g, 'i');

function entry(cat, label, combos) {
  combos = combos.filter(c => c && String(c).trim());
  if (!combos.length || !label || !String(label).trim()) return null;
  const caps = combos.map(keycaps);
  return { cat, label, combos: caps, hay: fold(`${label} ${combos.join(' ')} ${caps.flat().join(' ')}`) };
}

// Her arama kelimesi adda ya da tuşlarda
export const matches = (e, query) => fold(query).split(/\s+/).filter(Boolean).every(w => e.hay.includes(w));

// Liste, düzenleyicinin satırları gibi: sabitler (Super, Dock, fare), çekirdeğinkiler (kaldırılan uygulamalar hariç), kullanıcının
// eklediği uygulamalar, pencere yöneticisininkiler. Tuşu olmayanlar yok. tr: dil katmanı (window.LL_T)
export function cheatEntries(model, tr = s => s) {
  const fixed = [
    ['apps', 'Arama / overview', 'Super'], ['apps', 'Uygulama Dock’u', 'Super+Alt'],
    ['win', 'Pencereyi taşı (sürükle)', 'Super+Mouse L'], ['win', 'Pencereyi boyutlandır (sürükle)', 'Super+Mouse R'],
    ['ws', 'Workspace değiştir (tekerlek)', 'Super+Wheel'], ['ws', 'Gizli workspace’i aç / kapat', 'Super+Mouse 4'],
  ];
  const out = fixed.map(([cat, label, combo]) => entry(cat, tr(label), [combo]));
  for (const b of (model && Array.isArray(model.core) ? model.core : [])) {
    if (!b || b.removed) continue;
    const id = String(b.id || '');
    const label = b.custom ? (b.name || id) : tr(llLabel(id));
    out.push(entry(coreCategory(id, !!b.app), label, [b.combo || '']));
  }
  for (const g of (model && Array.isArray(model.tiling) ? model.tiling : [])) {
    const cmds = Array.isArray(g.commands) ? g.commands : [];
    out.push(entry(tilingCategory(cmds), tr(tilingLabel(cmds)), Array.isArray(g.bindings) ? g.bindings : []));
  }
  return out.filter(Boolean);
}

// Her grup en kısa sütuna (masonry): [sütun, ...] grup başına
export function masonry(heights, cols) {
  const tops = new Array(Math.max(1, cols)).fill(0);
  return heights.map(h => {
    let col = 0;
    for (let i = 1; i < tops.length; i++) if (tops[i] < tops[col]) col = i;
    tops[col] += h + 1;
    return col;
  });
}

export const columnsFor = width => (width >= 980 ? 3 : width >= 620 ? 2 : 1);
