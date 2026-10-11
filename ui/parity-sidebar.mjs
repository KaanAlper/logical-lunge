// Native sidebar/bug.rs and core BugReports contracts. No desktop side effects on import.
export const REPORT_FILES = [['blackbox', 'blackbox_record.txt'], ['core', 'core.log'], ['shell', 'shell.log'], ['tiling', 'tiling.log']];
export const REPORT_KINDS = [['hata', 'Hata'], ['cokme', 'Çökme'], ['performans', 'Performans'], ['istek', 'İstek'], ['oneri', 'Öneri']];
export const UI_SCALES = [85, 90, 100, 110, 125, 150];
export const expandedToggle = width => width >= 56 * 2 + 6;
export function localTime(now = new Date()) {
  const pad = n => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}T${pad(now.getHours())}:${pad(now.getMinutes())}`;
}
export function newReportDraft() {
  return { kind: 'hata', start: localTime(), end: localTime(), message: '', selected: Object.fromEntries([...REPORT_FILES.map(([k]) => [k, true]), ['device', true]]) };
}
function reportTime(value) {
  const normalized = String(value || '').trim().replace(' ', 'T');
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(normalized)) return null;
  const date = new Date(normalized);
  return Number.isFinite(date.getTime()) && localTime(date) === normalized ? date.toISOString() : null;
}
export function buildReport(draft, files = {}, device = null, context = {}) {
  if (!draft.message?.trim()) throw new Error('Hatayı kısaca açıklayın.');
  const start = reportTime(draft.start), end = draft.end?.trim() ? reportTime(draft.end) : null;
  if (!start || (draft.end?.trim() && (!end || end < start))) throw new Error('Geçerli bir zaman aralığı seçin.');
  if (!REPORT_KINDS.some(([k]) => k === draft.kind)) throw new Error('Geçerli bir rapor türü seçin.');
  const included = k => draft.selected[k] && files[k]?.text?.trim() ? files[k].text : null;
  const missingLogs = REPORT_FILES.filter(([k]) => draft.selected[k] && !included(k)).map(([, name]) => name);
  const d = draft.selected.device ? device : null;
  return {
    bug_type: draft.kind, incident_start: start, incident_end: end, description: draft.message.trim(),
    os_version: d?.os ?? null, cpu: d?.cpu ?? null, gpu: d?.gpu ?? null, ram: d?.ram ?? null,
    device_info: { ...(d ? { appVersion: d.appVersion, ...context } : {}), missingLogs },
    blackbox_log: included('blackbox'), core_log: included('core'), shell_log: included('shell'), tiling_log: included('tiling'), system_log: null,
  };
}
const REPORT_URL = 'https://bygirolbhyziitvnaxln.supabase.co/rest/v1/bug_reports';
const REPORT_KEY = 'sb_publishable_Tn0PWa_PME7JLTu3xHtd2w_b4ywwiAO';
function uploadRequest(url, init, onProgress) {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open(init.method, url); xhr.timeout = 45000;
    Object.entries(init.headers).forEach(([k, v]) => xhr.setRequestHeader(k, v));
    xhr.upload.onprogress = e => { if (e.lengthComputable && e.total > 0) onProgress(`Gönderiliyor ${Math.round(e.loaded * 100 / e.total)}%`); };
    xhr.onload = () => resolve({ ok: xhr.status >= 200 && xhr.status < 300, status: xhr.status, text: async () => xhr.responseText });
    xhr.onerror = () => reject(new ReportUnreachable('Sunucuya ulaşılamadı (bağlantı ya da DNS).'));
    xhr.ontimeout = () => reject(new ReportUnreachable('Sunucu zamanında yanıt vermedi.'));
    xhr.send(init.body);
  });
}
// Why a report did not go: the server could not be reached at all (no connection, DNS, timeout), or it answered
// and refused it. Only an unreachable server keeps the report for later (`queueReport`).
export class ReportUnreachable extends Error {}
export class ReportRefused extends Error {
  constructor(status, text) { super(`Sunucu raporu kabul etmedi, HTTP ${status}: ${String(text || '').slice(0, 240)}`); this.status = status; }
}
export async function submitReport(report, transport, onProgress = () => {}) {
  const headers = { apikey: REPORT_KEY, 'Content-Type': 'application/json', Prefer: 'return=minimal' };
  const send = transport || ((url, init) => uploadRequest(url, init, onProgress));
  const post = async payload => {
    try { return await send(REPORT_URL, { method: 'POST', headers, body: JSON.stringify(payload), signal: AbortSignal.timeout(45000) }); }
    catch (e) { throw e instanceof ReportUnreachable ? e : new ReportUnreachable(`Sunucuya ulaşılamadı (bağlantı ya da DNS). ${e?.message || ''}`.trim()); }
  };
  onProgress('Gönderiliyor…');
  let response = await post(report);
  if (response.ok) return;
  let text = await response.text(), detail;
  try { detail = JSON.parse(text); } catch {}
  const columns = ['core_log', 'shell_log', 'tiling_log'];
  if (response.status === 400 && detail?.code === 'PGRST204' && columns.some(k => detail.message?.includes(k))) {
    const legacy = { ...report };
    legacy.system_log = columns.filter(k => report[k] != null).map(k => `===== ${k.replace('_log', '.log')} =====\n${report[k]}`).join('\n\n') || null;
    columns.forEach(k => delete legacy[k]);
    onProgress('Gönderiliyor…');
    response = await post(legacy);
    if (response.ok) return;
    text = await response.text();
  }
  throw new ReportRefused(response.status, text);
}

// Reports kept while the server was unreachable, sent later: a while after the panel loads, then every ten
// minutes, backing off to an hour while the server stays away. Each goes once the server accepts it; a refused
// one is set aside. At most QUEUE_MAX reports / QUEUE_MAX_BYTES, the oldest dropped first. The list lives in the
// panel's IndexedDB (logs can be larger than localStorage allows).
export const QUEUE_MAX = 20, QUEUE_MAX_BYTES = 50 * 1024 * 1024, QUEUE_FIRST_MS = 30000, QUEUE_EVERY_MS = 600000, QUEUE_MAX_WAIT_MS = 3600000;
export function queueAdd(list, report, id = Date.now()) {
  const size = JSON.stringify(report).length;
  let next = [...list, { id, report, size, refused: false }];
  const total = () => next.reduce((n, e) => n + e.size, 0);
  while (next.length > QUEUE_MAX || (next.length > 1 && total() > QUEUE_MAX_BYTES)) next = next.slice(1);
  return next;
}
export const queueBackoffMs = failures => Math.min(QUEUE_EVERY_MS * 2 ** Math.min(Math.max(failures - 1, 0), 8), QUEUE_MAX_WAIT_MS);
// Sends the waiting reports oldest first; stops at the first one the server can't be reached for. Returns the ids
// that went and the ones refused, applied to the stored list afterwards (`queueApply`), so a report queued
// meanwhile is kept.
export async function flushQueue(list, transport) {
  const sent = [], refused = [];
  for (const entry of list) {
    if (entry.refused) continue;
    try { await submitReport(entry.report, transport); sent.push(entry.id); }
    catch (e) {
      if (e instanceof ReportUnreachable) return { sent, refused, reachable: false };
      refused.push(entry.id);
    }
  }
  return { sent, refused, reachable: true };
}
export const queueApply = (list, { sent, refused }) =>
  list.filter(e => !sent.includes(e.id)).map(e => refused.includes(e.id) ? { ...e, refused: true } : e);
export function idbQueueStore(name = 'll-report-queue') {
  const open = () => new Promise((resolve, reject) => {
    const r = indexedDB.open(name, 1);
    r.onupgradeneeded = () => r.result.createObjectStore('q');
    r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
  });
  const run = (mode, act) => open().then(db => new Promise((resolve, reject) => {
    const tx = db.transaction('q', mode), req = act(tx.objectStore('q'));
    tx.oncomplete = () => { db.close(); resolve(req.result); }; tx.onerror = () => { db.close(); reject(tx.error); };
  }));
  return { load: async () => (await run('readonly', s => s.get('list'))) || [], save: list => run('readwrite', s => s.put(list, 'list')) };
}
export async function queueReport(report, store = idbQueueStore()) {
  await store.save(queueAdd(await store.load(), report));
}
let queueStarted = false;
// Starts the sender once per page; `notify(sent)` reports reports that went out.
export function startReportQueue({ store = idbQueueStore(), transport, notify = () => {}, timer = setTimeout } = {}) {
  if (queueStarted) return;
  queueStarted = true;
  let failures = 0;
  const round = async () => {
    try {
      const list = await store.load();
      if (list.some(e => !e.refused)) {
        const r = await flushQueue(list, transport);
        await store.save(queueApply(await store.load(), r));
        if (r.sent.length) notify(r.sent.length);
        failures = r.reachable ? 0 : failures + 1;
      }
    } catch {}
    timer(round, failures ? queueBackoffMs(failures) : QUEUE_EVERY_MS);
  };
  timer(round, QUEUE_FIRST_MS);
}
async function corePost(path, transport) {
  const response = await transport(`http://127.0.0.1:6131${path}`, { method: 'POST', cache: 'no-store', signal: AbortSignal.timeout(15000) });
  if (response.status !== 204) throw new Error(`İşlem kaydedilemedi (${response.status}).`);
}
export async function removeLibraryItem(kind, path, transport = fetch) {
  if (!['wall', 'live', 'saver'].includes(kind) || !path) throw new Error('Geçersiz kütüphane öğesi.');
  await corePost(`/library-remove?kind=${kind}&path=${encodeURIComponent(path)}`, transport);
}
export async function saveUiScale(value, transport = fetch) {
  if (!UI_SCALES.includes(value)) throw new Error('Geçersiz arayüz ölçeği.');
  await corePost(`/pref?k=uiScale&v=${value}`, transport);
}
export const importedSaver = path => /[\\/]LogicalLunge[\\/]screensavers[\\/]/i.test(path || '');

// Quick settings switches, checked as the native panel checks them: after its request, a switch's source (the
// state the hardware or the core owns) is read until it shows the switch as asked, for up to VERIFY_FOR. The tile
// always ends on what the source says; a card says so when the source shows the opposite or never answers.
export const VERIFY_FOR = 10000, VERIFY_EVERY = 500;

// What a tile's switch flipped, from a read of its source: true / false, or null (neither: a wired adapter
// without a cable is enabled but not up; a tile without a switch of the hardware's or the core's)
export function switchedIn(tile, part) {
  switch (tile) {
    case 'wifi': return part?.radios?.wifi == null ? null : part.radios.wifi === 'On';
    case 'bluetooth': return part?.radios?.bluetooth == null ? null : part.radios.bluetooth === 'On';
    case 'ethernet': return part?.eth?.state === 'up' ? true : part?.eth?.state === 'disabled' ? false : null;
    case 'mic': return typeof part?.mic === 'boolean' ? part.mic : null;
    case 'nightLight': return typeof part?.night?.on === 'boolean' ? part.night.on : null;
    case 'idleInhibitor': return typeof part?.awake === 'boolean' ? part.awake : null;
    default: return null;
  }
}

// seen: undefined (the source gave no answer), null (neither on nor off) or the switch's state. Done once it shows
// the switch as asked; read again while time is left; then failed if the source shows the opposite or never
// answered (neither on nor off is no failure: nothing contradicts it).
export function verdict(want, seen, leftMs) {
  if (seen === want) return 'done';
  if (leftMs > 0) return 'again';
  return seen === null ? 'done' : 'failed';
}

// Reads the switch's source until its verdict; `apply` shows each read on the tile.
export async function checkSwitch({ tile, want, read, apply, waitMs = VERIFY_FOR, sleep = ms => new Promise(r => setTimeout(r, ms)), now = Date.now }) {
  const until = now() + waitMs;
  for (;;) {
    let seen;
    try {
      const part = await read();
      if (part) { apply(part); seen = switchedIn(tile, part); }
    } catch {}
    const v = verdict(want, seen, until - now());
    if (v !== 'again') return { ok: v === 'done', answered: seen !== undefined };
    await sleep(VERIFY_EVERY);
  }
}
