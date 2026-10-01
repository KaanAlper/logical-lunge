import React, { useEffect, useRef, useState } from 'react';
import { emit } from '@tauri-apps/api/event';
import * as lunge from 'lunge/shell';

const HELPER = '{{INSTALL_ESC}}\\lunge.exe';
const SUPABASE_URL = 'https://bygirolbhyziitvnaxln.supabase.co';
// A publishable key is safe in a shipped client. The table must allow INSERT
// through RLS while denying public SELECT, UPDATE, and DELETE.
const SUPABASE_KEY = 'sb_publishable_Tn0PWa_PME7JLTu3xHtd2w_b4ywwiAO';
const FILES = [
  { kind: 'blackbox', name: 'blackbox_record.txt', icon: 'monitor_heart' },
  { kind: 'core', name: 'core.log', icon: 'description' },
  { kind: 'shell', name: 'shell.log', icon: 'description' },
  { kind: 'tiling', name: 'tiling.log', icon: 'view_quilt' },
];
const initialFiles = () => Object.fromEntries(FILES.map(file => [file.kind, { phase: 'loading', text: '', bytes: 0, error: '' }]));
const initialSelection = () => Object.fromEntries(FILES.map(file => [file.kind, true]));
const nowLocal = () => {
  const d = new Date();
  return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
};
const parse = value => { try { return JSON.parse(value); } catch { return null; } };
const Icon = ({ name }) => <span className="icon" aria-hidden="true">{name}</span>;
const labels = {
  hata: ['Hata', 'Bug'], cokme: ['Çökme', 'Crash'], performans: ['Performans', 'Performance'],
  istek: ['İstek', 'Request'], oneri: ['Öneri', 'Suggestion'],
};

export default function BugReportDialog({ host, onClose }) {
  const english = !String(window.LL_LOCALE || 'tr').toLowerCase().startsWith('tr');
  const T = (tr, en) => english ? en : tr;
  const [bugType, setBugType] = useState('hata');
  const [start, setStart] = useState(nowLocal);
  const [end, setEnd] = useState(nowLocal);
  const [description, setDescription] = useState('');
  const [files, setFiles] = useState(initialFiles);
  const [selected, setSelected] = useState(initialSelection);
  const [device, setDevice] = useState(null);
  const [deviceError, setDeviceError] = useState(false);
  const [sending, setSending] = useState(false);
  const [uploadProgress, setUploadProgress] = useState(null);
  const [sent, setSent] = useState(false);
  const [confirmMissing, setConfirmMissing] = useState(false);
  const [error, setError] = useState('');
  const alive = useRef(true);
  const sendingRef = useRef(false);

  const collect = async kind => {
    setFiles(previous => ({ ...previous, [kind]: { phase: 'loading', text: '', bytes: 0, error: '' } }));
    try {
      const run = await lunge.shellExec(HELPER, ['--bug-report-file', kind]);
      const result = parse((run.stdout || '').trim());
      if (!result?.ok || !result.text?.trim()) throw new Error(result?.error || T('Günlük okunamadı.', 'Log could not be read.'));
      if (alive.current) setFiles(previous => ({ ...previous, [kind]: {
        phase: 'ready', text: result.text, bytes: result.bytes || new Blob([result.text]).size, error: '',
      } }));
    } catch (cause) {
      if (alive.current) setFiles(previous => ({ ...previous, [kind]: {
        phase: 'error', text: '', bytes: 0, error: cause.message || String(cause),
      } }));
    }
  };

  useEffect(() => {
    alive.current = true;
    (async () => {
      try {
        const run = await lunge.shellExec(HELPER, ['--bug-report-device']);
        const data = parse((run.stdout || '').trim());
        if (!data || typeof data !== 'object') throw new Error('device');
        if (alive.current) setDevice(data);
      } catch { if (alive.current) setDeviceError(true); }
      // Core log should include the black-box record just created.
      await collect('blackbox');
      await Promise.all(['core', 'shell', 'tiling'].map(collect));
    })();
    return () => { alive.current = false; };
  }, []);

  const post = async (payload, allowLegacyFallback = false) => {
    setUploadProgress(null);
    const response = await new Promise((resolve, reject) => {
      const request = new XMLHttpRequest();
      request.open('POST', `${SUPABASE_URL}/rest/v1/bug_reports`);
      request.timeout = 45000;
      request.setRequestHeader('apikey', SUPABASE_KEY);
      request.setRequestHeader('Content-Type', 'application/json');
      request.setRequestHeader('Prefer', 'return=minimal');
      request.upload.onprogress = event => {
        if (alive.current && event.lengthComputable && event.total > 0) {
          // 100% of bytes uploaded is not yet a server acknowledgement.
          setUploadProgress(Math.min(99, Math.floor(event.loaded / event.total * 100)));
        }
      };
      request.onload = () => resolve({ ok: request.status >= 200 && request.status < 300,
        status: request.status, body: request.responseText, statusText: request.statusText });
      request.onerror = () => reject(new Error(T('Ağ bağlantısı kurulamadı.', 'Network connection failed.')));
      request.ontimeout = () => reject(new Error(T('Gönderim zaman aşımına uğradı.', 'Upload timed out.')));
      request.onabort = () => reject(new Error(T('Gönderim iptal edildi.', 'Upload was cancelled.')));
      request.send(JSON.stringify(payload));
    });
    if (!response.ok) {
      const body = response.body || '';
      const detail = parse(body);
      if (allowLegacyFallback && response.status === 400 && detail?.code === 'PGRST204'
          && /core_log|shell_log|tiling_log/.test(detail.message || '')) return false;
      throw new Error(`${response.status}: ${body.slice(0, 240) || response.statusText}`);
    }
    return true;
  };

  const send = async allowMissing => {
    if (sendingRef.current) return;
    if (!description.trim()) { setError(T('Hatayı kısaca açıklayın.', 'Please describe the issue.')); return; }
    if (!start || (end && new Date(end) < new Date(start))) {
      setError(T('Geçerli bir zaman aralığı seçin.', 'Choose a valid time range.')); return;
    }
    if (FILES.some(file => selected[file.kind] && files[file.kind].phase === 'loading')) return;
    const missing = FILES.filter(file => selected[file.kind] && files[file.kind].phase === 'error');
    if (missing.length && !allowMissing) { setConfirmMissing(true); return; }
    setConfirmMissing(false);
    setError('');
    sendingRef.current = true;
    setSending(true);
    try {
      const included = kind => selected[kind] && files[kind].phase === 'ready' ? files[kind].text : null;
      const core = included('core'), shell = included('shell'), tiling = included('tiling');
      const base = {
        bug_type: bugType,
        incident_start: new Date(start).toISOString(),
        incident_end: end ? new Date(end).toISOString() : null,
        description: description.trim(),
        os_version: device?.os || host?.friendlyOsVersion || host?.osVersion || null,
        cpu: device?.cpu || null,
        gpu: device?.gpu || null,
        ram: device?.ram || null,
        blackbox_log: included('blackbox'),
        device_info: {
          appVersion: device?.appVersion || null,
          resolution: `${window.screen.width}x${window.screen.height}`,
          uptimeMs: host?.uptime || null,
          timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
          missingLogs: missing.map(file => file.name),
        },
      };
      // New schema stores each attachment separately. The old two-column
      // table rejects unknown fields without inserting anything; then retry
      // once with its combined system_log field.
      const separateColumns = await post({ ...base, core_log: core, shell_log: shell, tiling_log: tiling, system_log: null }, true);
      if (!separateColumns) {
        const sections = [['core.log', core], ['shell.log', shell], ['tiling.log', tiling]]
          .filter(([, text]) => text).map(([name, text]) => `===== ${name} =====\n${text}`);
        await post({ ...base, system_log: sections.length ? sections.join('\n\n') : null });
      }
      if (alive.current) setSent(true);
      emit('ll:toast', {
        kind: 'ok', title: T('Rapor gönderildi', 'Report sent'),
        body: T('Hata raporu kaydedildi.', 'The issue report was saved.'), icon: 'check_circle',
      }).catch(() => {});
    } catch (cause) {
      const message = T('Gönderilemedi: ', 'Could not send: ') + (cause.message || String(cause));
      if (alive.current) setError(message);
      else emit('ll:toast', { kind: 'error', title: T('Rapor gönderilemedi', 'Report failed'),
        body: message, icon: 'error' }).catch(() => {});
    } finally {
      sendingRef.current = false;
      if (alive.current) { setSending(false); setUploadProgress(null); }
    }
  };

  return (
    <div className="bug-overlay" onMouseDown={event => { if (event.target === event.currentTarget && !sending) onClose(); }}>
      <section className="bug-dialog" role="dialog" aria-modal="true" aria-label={T('Hata bildirimi', 'Issue report')}>
        <header className="bug-head">
          <div className="bug-head-icon"><Icon name="bug_report" /></div>
          <div><strong>{T('Hata bildirimi', 'Issue report')}</strong><small>{T('Ne oldu? İlgili günlükleri ekleyerek anlat.', 'Describe what happened and attach the relevant logs.')}</small></div>
          <button className="qbtn" onClick={onClose} disabled={sending} title={T('Kapat', 'Close')}><Icon name="close" /></button>
        </header>
        {sent ? (
          <div className="bug-success">
            <Icon name="check_circle" />
            <strong>{T('Rapor gönderildi', 'Report sent')}</strong>
            <p>{T('Seçtiğin ekler rapora eklendi.', 'The selected attachments were included.')}</p>
            <button className="bug-submit-btn" onClick={onClose}>{T('Kapat', 'Close')}</button>
          </div>
        ) : (
          <>
            <div className="bug-scroll">
              <div className="bug-type-row">
                {Object.entries(labels).map(([kind, label]) => (
                  <button key={kind} className={`bug-type-chip ${bugType === kind ? 'active' : ''}`}
                    onClick={() => setBugType(kind)} disabled={sending}>{english ? label[1] : label[0]}</button>
                ))}
              </div>
              <div className="bug-time-row">
                <label className="bug-field"><span className="bug-label">{T('Başlangıç', 'Started')}</span>
                  <input className="bug-input" type="datetime-local" value={start} onChange={event => setStart(event.target.value)} disabled={sending} /></label>
                <label className="bug-field"><span className="bug-label">{T('Bitiş', 'Ended')}</span>
                  <input className="bug-input" type="datetime-local" value={end} onChange={event => setEnd(event.target.value)} disabled={sending} /></label>
              </div>
              <label className="bug-field"><span className="bug-label">{T('Hatayı açıkla', 'Describe the issue')}</span>
                <textarea className="bug-textarea" value={description} onChange={event => setDescription(event.target.value)}
                  placeholder={T('Ne yapıyordun, ne bekliyordun, ne oldu?', 'What were you doing, what did you expect, and what happened?')}
                  disabled={sending} /></label>
              <div className="bug-attachments-head"><span>{T('Tanı ekleri', 'Diagnostic attachments')}</span>
                <small>{T('Son günlük kesitleri gönderilir.', 'Recent log excerpts are sent.')}</small></div>
              <div className="bug-files">
                {FILES.map(file => {
                  const state = files[file.kind];
                  const included = selected[file.kind];
                  return <div className={`bug-file-row ${state.phase} ${!included ? 'excluded' : ''}`} key={file.kind}>
                    <input className="bug-checkbox" type="checkbox" checked={included} disabled={sending}
                      onChange={event => setSelected(previous => ({ ...previous, [file.kind]: event.target.checked }))}
                      aria-label={`${file.name} ${T('ekle', 'include')}`} />
                    <Icon name={file.icon} />
                    <div className="bug-file-info"><strong>{file.name}</strong><small>
                      {state.phase === 'loading' ? T('Toplanıyor…', 'Collecting…') :
                        state.phase === 'error' ? T('Bulunamadı veya okunamadı', 'Missing or unreadable') :
                        `${Math.ceil(state.bytes / 1024)} KB · ${sent ? T('Gönderildi', 'Sent') :
                          sending && included ? T('Gönderiliyor', 'Uploading') + (uploadProgress == null ? '…' : ` ${uploadProgress}%`) : T('Hazır', 'Ready')}`}
                    </small></div>
                    {included && state.phase === 'loading' && <span className="bug-ring" aria-label={T('Toplanıyor', 'Collecting')} />}
                    {included && sending && state.phase === 'ready' && (uploadProgress == null
                      ? <span className="bug-ring" aria-label={T('Gönderiliyor', 'Uploading')} />
                      : <span className="bug-progress" style={{ '--progress': `${uploadProgress}%` }}
                          aria-label={`${T('Toplam gönderim', 'Overall upload')} ${uploadProgress}%`} />)}
                    {state.phase === 'error' && <><Icon name="cancel" /><button className="bug-retry" onClick={() => collect(file.kind)} disabled={sending}>{T('Tekrar dene', 'Retry')}</button></>}
                    {state.phase === 'ready' && !sending && <Icon name={sent ? 'check_circle' : 'check'} />}
                  </div>;
                })}
              </div>
              <div className="bug-device-card">
                <strong>{T('Cihaz', 'Device')}</strong>
                {deviceError && <small>{T('Cihaz bilgisi okunamadı; rapor yine gönderilebilir.', 'Device details unavailable; the report can still be sent.')}</small>}
                <span>{device?.os || host?.friendlyOsVersion || host?.osVersion || T('Alınıyor…', 'Loading…')}</span>
                <span>{[device?.cpu, device?.gpu, device?.ram].filter(Boolean).join(' · ')}</span>
              </div>
              {error && <div className="bug-error" role="alert"><Icon name="error" /><span>{error}</span></div>}
            </div>
            <footer className="bug-footer">
              <small>{T('Seçtiğin günlüklerde pencere adları ve dosya yolları bulunabilir.', 'Selected logs may contain window titles and file paths.')}</small>
              <button className="bug-submit-btn" disabled={sending || FILES.some(file => selected[file.kind] && files[file.kind].phase === 'loading')}
                onClick={() => send(false)}>
                {sending ? <><span className="bug-ring" />{T('Gönderiliyor…', 'Sending…')}</> : <><Icon name="send" />{T('Raporu gönder', 'Send report')}</>}
              </button>
            </footer>
          </>
        )}
        {confirmMissing && <div className="bug-confirm-backdrop" role="alertdialog" aria-modal="true">
          <div className="bug-confirm"><Icon name="warning" /><strong>{T('Bazı günlükler eklenemedi', 'Some logs could not be attached')}</strong>
            <p>{T('Kırmızı işaretli ekleri yeniden deneyebilir veya raporu onlar olmadan gönderebilirsin.',
              'Retry the marked attachments or send the report without them.')}</p>
            <div className="bug-confirm-actions">
              <button onClick={() => setConfirmMissing(false)}>{T('Geri dön', 'Go back')}</button>
              <button onClick={() => send(true)}>{T('Eksik günlüklerle gönder', 'Send without missing logs')}</button>
            </div>
          </div>
        </div>}
      </section>
    </div>
  );
}
