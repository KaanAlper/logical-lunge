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
    xhr.onerror = () => reject(new Error('Ağ bağlantısı kurulamadı.'));
    xhr.ontimeout = () => reject(new Error('Gönderim zaman aşımına uğradı.'));
    xhr.send(init.body);
  });
}
export async function submitReport(report, transport, onProgress = () => {}) {
  const headers = { apikey: REPORT_KEY, 'Content-Type': 'application/json', Prefer: 'return=minimal' };
  const post = payload => (transport || ((url, init) => uploadRequest(url, init, onProgress)))(REPORT_URL, { method: 'POST', headers, body: JSON.stringify(payload), signal: AbortSignal.timeout(45000) });
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
  throw new Error(`${response.status}: ${text.slice(0, 240)}`);
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
