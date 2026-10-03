import React, { useEffect, useRef, useState } from 'react';

export function ContextMenu({ items, position, onAction, onClose, embedded = false }) {
  const root = useRef(null);
  const [trail, setTrail] = useState([]);
  const [selected, setSelected] = useState(0);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const rows = trail.length ? trail.at(-1).children : items;
  useEffect(() => { setTrail([]); setSelected(0); setError(''); root.current?.focus(); }, [items]);
  useEffect(() => {
    const outside = e => { if (!root.current?.contains(e.target)) onClose(); };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [onClose]);
  const pick = async row => {
    if (!row || row.enabled === false || busy) return;
    if (row.children?.length) { setTrail(t => [...t, row]); setSelected(0); return; }
    setBusy(true); setError('');
    try { await onAction(row.id); onClose(); }
    catch (e) { setError(String(e.message || e)); }
    finally { setBusy(false); root.current?.focus(); }
  };
  const back = () => { setTrail(t => t.slice(0, -1)); setSelected(0); };
  const key = e => {
    if (e.key === 'Escape') { e.preventDefault(); onClose(); }
    if (e.key === 'ArrowLeft' && trail.length) { e.preventDefault(); back(); }
    if (e.key === 'Enter' || e.key === 'ArrowRight') { e.preventDefault(); pick(rows[selected]); }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault(); const step=e.key === 'ArrowDown' ? 1 : -1;
      let next=selected;
      for (let n=0;n<rows.length;n++) { next=(next+step+rows.length)%rows.length; if (rows[next].enabled !== false) break; }
      setSelected(next);
    }
  };
  const style = embedded ? undefined : {
    left: Math.max(4, Math.min(position?.x || 4, window.innerWidth - 314)),
    top: Math.max(4, Math.min(position?.y || 4, window.innerHeight - (rows.length * 35 + 16))),
    maxHeight: Math.max(40, window.innerHeight - Math.max(4, Math.min(position?.y || 4, window.innerHeight - (rows.length * 35 + 16))) - 4),
  };
  return <div ref={root} className={`ll-menu ${embedded ? 'embedded' : ''}`} style={style} role="menu" tabIndex={-1} onKeyDown={key} aria-busy={busy}>
    {trail.length > 0 && <button role="menuitem" onClick={back}><span className="icon">chevron_left</span><span>{trail.at(-1).label}</span></button>}
    {rows.map((row, i) => <button key={row.id} role={row.checked !== undefined ? 'menuitemcheckbox' : 'menuitem'} aria-checked={row.checked || undefined}
      disabled={row.enabled === false || busy} className={selected === i ? 'selected' : ''} onMouseEnter={() => setSelected(i)} onClick={() => pick(row)}>
      <span className="icon">{row.checked ? 'check' : row.icon || ''}</span><span>{row.label}</span>
      {row.children?.length > 0 && <span className="icon">chevron_right</span>}
    </button>)}
    {error && <div className="menu-error" role="alert">{error}</div>}
  </div>;
}
