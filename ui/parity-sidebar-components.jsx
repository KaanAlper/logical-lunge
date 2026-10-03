import React, { useState, useEffect, useLayoutEffect, useRef } from 'react';
import { createPortal } from 'react-dom';
import { ContextMenu } from './lib/context-menu.jsx';
import { REPORT_FILES, REPORT_KINDS, newReportDraft, buildReport, submitReport, removeLibraryItem } from './parity-sidebar.mjs';

const Icon = ({ name }) => <span className="icon">{name}</span>;

// Preserve the panel's Escape behavior while a nested menu/question owns the keyboard.
export function ParityDialog({ title, role = 'dialog', onClose, children, point }) {
  const ref = useRef(null);
  const closeRef = useRef(onClose); closeRef.current = onClose;
  useLayoutEffect(() => {
    window.__llSidebarModal = (window.__llSidebarModal || 0) + 1;
    const prior = document.activeElement;
    ref.current?.querySelector('button')?.focus();
    const key = e => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); closeRef.current(); }
      if (role === 'menu' && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) {
        const buttons = [...ref.current.querySelectorAll('button:not(:disabled)')];
        const i = buttons.indexOf(document.activeElement);
        const next = e.key === 'Home' ? 0 : e.key === 'End' ? buttons.length - 1 : (i + (e.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
        e.preventDefault(); buttons[next]?.focus();
      }
      if (e.key === 'Tab') {
        const buttons = [...ref.current.querySelectorAll('button:not(:disabled), input:not(:disabled)')];
        if (!buttons.length) return;
        const i = buttons.indexOf(document.activeElement);
        if ((e.shiftKey && i <= 0) || (!e.shiftKey && (i < 0 || i === buttons.length - 1))) {
          e.preventDefault(); buttons[e.shiftKey ? buttons.length - 1 : 0].focus();
        }
      }
    };
    window.addEventListener('keydown', key, true);
    return () => { window.__llSidebarModal--; window.removeEventListener('keydown', key, true); if (prior?.isConnected) prior.focus(); };
  }, [role]);
  const host = document.querySelector('.page.shown') || document.querySelector('.panel');
  const bounds = host.getBoundingClientRect();
  const style = point ? { position: 'absolute', width: Math.min(260, host.clientWidth - 24), left: Math.max(8, Math.min((point.x - bounds.left) * host.clientWidth / bounds.width, host.clientWidth - 284)), top: Math.max(8, Math.min((point.y - bounds.top) * host.clientHeight / bounds.height, host.clientHeight - 350)), maxHeight: 'calc(100% - 16px)' } : undefined;
  return createPortal(<div className="dlg-scrim" onClick={onClose} style={point ? { background: 'transparent', zIndex: 80 } : { zIndex: 80 }}>
    <div className="dlg kb-confirm" ref={ref} role={role} aria-label={title} aria-modal="true" style={style} onClick={e => e.stopPropagation()}>
      <div className="dlg-head"><span className="dlg-title" style={point ? { fontSize: 15 } : undefined}>{title}</span></div>
      {children}
    </div>
  </div>, host);
}

export function GalleryMenu({ item, point, kind, actions, onClose, onRemoved }) {
  const [confirm, setConfirm] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const openingConfirm = useRef(false);
  const actionMap = {};
  const rows = actions.map(function row(a, i) { const id = `action-${Object.keys(actionMap).length}`; actionMap[id] = a; return { id, icon: a.icon, label: a.label, enabled: !a.disabled, ...(a.children ? { children: a.children.map(row) } : {}) }; });
  if (kind && item.path) rows.push({ id: 'remove', icon: 'delete', label: 'Kütüphaneden kaldır' });
  const close = () => { if (!busy) onClose(); };
  const run = async action => {
    if (busy) return;
    setBusy(true); setError('');
    try { await action(); onClose(); } catch (e) { setError(String(e.message || e)); } finally { setBusy(false); }
  };
  return <ParityDialog title={confirm ? 'Kütüphaneden kaldırılsın mı?' : item.name || item.label || 'Galeri'} point={confirm ? null : point} onClose={close}>
    {confirm ? <>
      <p className="kb-confirm-text">{item.name || item.path}</p>
      <div className="dlg-foot">
        <button className="chip" disabled={busy} onClick={close}>Vazgeç</button>
        <button className="chip on" disabled={busy} onClick={() => run(async () => { await removeLibraryItem(kind, item.path); await onRemoved?.(item); })}>{busy ? 'Kaldırılıyor…' : 'Kaldır'}</button>
      </div>
    </> : <ContextMenu embedded items={rows} onClose={() => { if (!openingConfirm.current) close(); }} onAction={async id => {
      if (id === 'remove') { openingConfirm.current = true; setConfirm(true); }
      else await actionMap[id].run(item);
    }} />}
    {error && <div className="kb-msg error" role="alert">{error}</div>}
  </ParityDialog>;
}

export function IssueReport({ helper, load, save, host, onSending }) {
  const [draft, setDraft] = useState(() => {
    const fresh = newReportDraft(), stored = load('ll.bug.draft', null);
    return stored && typeof stored === 'object' ? { ...fresh, ...stored, selected: { ...fresh.selected, ...stored.selected } } : fresh;
  });
  const [files, setFiles] = useState({}), [device, setDevice] = useState(null), [deviceError, setDeviceError] = useState('');
  const [sending, setSending] = useState(false), [sent, setSent] = useState(false), [error, setError] = useState(''), [missing, setMissing] = useState(false), [progress, setProgress] = useState('');
  const alive = useRef(true), generation = useRef({});
  const stage = patch => {
    const next = { ...draft, ...patch }; setDraft(next); save('ll.bug.draft', next); setError('');
  };
  const collect = async kind => {
    const id = (generation.current[kind] || 0) + 1; generation.current[kind] = id;
    setFiles(f => ({ ...f, [kind]: { loading: true } }));
    let result;
    try {
      const r = JSON.parse(await helper(['--bug-report-file', kind]));
      if (!r.ok || !r.text?.trim()) throw new Error(r.error || 'Bulunamadı veya okunamadı');
      result = { text: r.text, bytes: r.bytes ?? new TextEncoder().encode(r.text).length };
    } catch (e) { result = { error: String(e.message || e) }; }
    if (alive.current && generation.current[kind] === id) setFiles(f => ({ ...f, [kind]: result }));
  };
  const collectDevice = async () => {
    setDeviceError('');
    try {
      const r = JSON.parse(await helper(['--bug-report-device']));
      if (!r || typeof r !== 'object' || Array.isArray(r) || r.error) throw new Error('Cihaz bilgisi okunamadı');
      if (alive.current) setDevice(r);
    } catch (e) { if (alive.current) setDeviceError(String(e.message || e)); }
  };
  useEffect(() => {
    alive.current = true; collectDevice();
    // The new black box record must be present in the subsequent core log excerpt.
    collect('blackbox').then(() => { if (alive.current) REPORT_FILES.slice(1).forEach(([k]) => collect(k)); });
    return () => { alive.current = false; };
  }, []);
  const loading = REPORT_FILES.some(([k]) => draft.selected[k] && (!files[k] || files[k].loading));
  const send = async allowMissing => {
    if (sending || loading || sent) return;
    let report;
    try {
      report = buildReport(draft, files, device, { resolution: `${screen.width}x${screen.height}`, uptimeMs: host?.uptime == null ? null : host.uptime * 1000, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone });
      if (report.device_info.missingLogs.length && !allowMissing) { setMissing(true); return; }
    } catch (e) { setError(e.message); return; }
    setMissing(false); setError(''); setSending(true); onSending?.(true);
    const submitted = JSON.stringify(draft);
    try {
      await submitReport(report, undefined, p => alive.current && setProgress(p));
      if (JSON.stringify(load('ll.bug.draft', draft)) === submitted) save('ll.bug.draft', null);
      if (alive.current) { setSent(true); setProgress('Rapor gönderildi'); }
    } catch (e) { if (alive.current) { setError(`Gönderilemedi: ${e.message}`); setProgress(''); } }
    finally { onSending?.(false); if (alive.current) setSending(false); }
  };
  const select = (kind, checked) => stage({ selected: { ...draft.selected, [kind]: checked } });
  return <form onSubmit={e => { e.preventDefault(); send(false); }} className="parity-report">
    <fieldset disabled={sending || sent} style={{ border: 0, padding: 0, margin: 0, minWidth: 0 }}>
      <div className="wall-target" role="group" aria-label="Rapor türü">{REPORT_KINDS.map(([k, label]) => <button type="button" key={k} className={`chip ${draft.kind === k ? 'on' : ''}`} aria-pressed={draft.kind === k} onClick={() => stage({ kind: k })}>{label}</button>)}</div>
      <div style={{ display: 'flex', gap: 10, margin: '12px 0' }}>
        <label style={{ minWidth: 0, flex: 1 }}>Başlangıç<input aria-label="Başlangıç" type="datetime-local" value={draft.start} onChange={e => stage({ start: e.target.value })} style={{ width: '100%', marginTop: 6 }} /></label>
        <label style={{ minWidth: 0, flex: 1 }}>Bitiş<input aria-label="Bitiş" type="datetime-local" value={draft.end} onChange={e => stage({ end: e.target.value })} style={{ width: '100%', marginTop: 6 }} /></label>
      </div>
      <label>Hatayı açıkla<textarea aria-label="Hatayı açıkla" value={draft.message} onChange={e => stage({ message: e.target.value })} placeholder="Ne yapıyordun, ne bekliyordun, ne oldu?" rows={5} style={{ width: '100%', resize: 'vertical', margin: '6px 0 12px' }} /></label>
      <div className="kb-gtitle">Tanı ekleri</div><div className="kb-hint">Son günlük kesitleri gönderilir.</div>
      {REPORT_FILES.map(([k, name]) => <div key={k} className="saver-row" style={{ flexWrap: 'wrap', margin: '6px 0' }}>
        <label className="kb-check"><input type="checkbox" checked={draft.selected[k]} onChange={e => select(k, e.target.checked)} />{name}</label>
        <span title={files[k]?.error}>{!files[k] || files[k].loading ? 'Toplanıyor…' : files[k].error ? 'Bulunamadı veya okunamadı' : `${Math.ceil(files[k].bytes / 1024)} KB · Hazır`}</span>
        {files[k]?.error && <button type="button" className="chip" onClick={() => collect(k)} aria-label={`${name} tekrar dene`}>Tekrar dene</button>}
      </div>)}
      <label className="kb-check"><input type="checkbox" checked={draft.selected.device} onChange={e => select('device', e.target.checked)} />Cihaz bilgileri</label>
      <div className="kb-hint">{device ? [device.os, device.cpu, device.gpu, device.ram].filter(Boolean).join(' · ') : deviceError ? 'Cihaz bilgisi okunamadı; rapor yine gönderilebilir.' : 'Cihaz bilgisi alınıyor…'}</div>
      {deviceError && <button type="button" className="chip" onClick={collectDevice}>Cihaz bilgisini tekrar dene</button>}
    </fieldset>
    {error && <div className="kb-msg error" role="alert">{error}</div>}
    {progress && <div className="kb-msg" role="status">{progress}</div>}
    <div className="dlg-foot"><button className="chip on" type="submit" aria-label={sent ? 'Rapor gönderildi' : sending ? 'Gönderiliyor…' : 'Gönder'} disabled={sending || loading || sent}><Icon name={sent ? 'check_circle' : 'send'} />{sent ? 'Rapor gönderildi' : sending ? 'Gönderiliyor…' : 'Gönder'}</button></div>
    {missing && <ParityDialog title="Bazı günlükler eklenemedi" onClose={() => setMissing(false)}>
      <p className="kb-confirm-text">Kırmızı işaretli ekleri yeniden deneyebilir veya raporu onlar olmadan gönderebilirsin.</p>
      <div className="dlg-foot"><button type="button" className="chip" onClick={() => setMissing(false)}>Geri dön</button><button type="button" className="chip on" onClick={() => send(true)}>Eksik günlüklerle gönder</button></div>
    </ParityDialog>}
  </form>;
}
