import React, { useState, useEffect, useLayoutEffect, useRef } from 'react';
import { createLocationDraft, editLocationDraft, choosePlace, locationPatch, createLatestSearch } from './widget-location.mjs';
const h = React.createElement;

export function LocationPicker({ spec, load, language, translate: T, editing, save, cancel, position, elementRef }) {
  // This draft belongs to the popup session. Store events and weather polls never
  // replace it; reopening creates a new session from the saved location.
  const [draft, setDraft] = useState(() => createLocationDraft(spec));
  const [field, setField] = useState('country'), [index, setIndex] = useState(-1);
  const [search, setSearch] = useState({ results: [], loading: false, error: null });
  const [saving, setSaving] = useState(false), [saveError, setSaveError] = useState(null);
  const inputs = useRef({}), loader = useRef(load), worker = useRef(null), pendingFocus = useRef(null);
  loader.current = load;
  if (!worker.current) worker.current = createLatestSearch(async (query, signal) => {
    const params = new URLSearchParams({ q: query.text, language: query.language });
    if (query.kind !== 'country') {
      params.set('countryCode', query.countryCode); params.set('kind', query.kind);
      if (query.kind === 'district') { params.set('city', query.city); params.set('latitude', query.latitude); params.set('longitude', query.longitude); }
    }
    const data = await loader.current(`/widgets/${query.kind === 'country' ? 'countries' : 'places'}?${params}`, signal);
    return Array.isArray(data.results) ? data.results : [];
  }, state => { setSearch(state); setIndex(-1); });
  useEffect(() => () => worker.current.dispose(), []);
  useEffect(() => {
    const text = field ? draft.texts[field].trim() : '';
    const allowed = field === 'country' || (draft.country && (field === 'city' || draft.city));
    worker.current.query(text && allowed && !draft[field] ? { kind: field, text, language,
      countryCode: draft.country?.code, city: draft.city?.city, latitude: draft.city?.cityLatitude, longitude: draft.city?.cityLongitude } : null);
    return () => worker.current.cancel();
  }, [field, draft, language]);
  useEffect(() => { let live = true; editing(true).then(() => { if (live) inputs.current.country?.focus(); }).catch(e => setSaveError(String(e))); return () => { live = false; }; }, []);
  useLayoutEffect(() => {
    const next = pendingFocus.current; pendingFocus.current = null;
    if (next) inputs.current[next]?.focus();
  }, [draft]);
  useEffect(() => { if (index >= 0) document.getElementById(`location-option-${spec.id}-${index}`)?.scrollIntoView({ block: 'nearest' }); }, [index, spec.id]);
  const focus = next => { pendingFocus.current = next; setField(next); };
  const choose = result => {
    const next = choosePlace(draft, field, result); if (next === draft) return;
    worker.current.query(null); setDraft(next); setSaveError(null);
    if (field === 'country') focus('city'); else if (field === 'city') focus('district'); else setField(null);
  };
  const valid = locationPatch(draft, spec.recentLocations);
  const submit = async e => {
    e.preventDefault(); if (!valid || saving) return;
    worker.current.query(null); setSaving(true); setSaveError(null);
    try { await save(valid); } catch (error) { setSaveError(String(error)); setSaving(false); }
  };
  const labels = { country: 'Ülke', city: 'Şehir', district: 'İlçe (isteğe bağlı)' };
  return h('form', { ref: elementRef, className: 'widget-location-popup', role: 'dialog', 'aria-label': T('Konum'), style: position, onSubmit: submit,
    onPointerDown: e => { e.stopPropagation(); const target = e.target.closest('input, button'); editing(true).then(() => { if (target?.isConnected && document.activeElement !== target) target.focus({ preventScroll: true }); }).catch(error => setSaveError(String(error))); },
    onFocusCapture: () => editing(true).catch(error => setSaveError(String(error))),
    onKeyDown: e => { if (e.key === 'Escape') { e.stopPropagation(); e.preventDefault(); if (!saving) cancel(); } } },
    h('div', { className: 'location-heading' },
      h('div', { className: 'location-heading-text' }, h('strong', null, T('Konumu değiştir')), h('span', { className: 'subtext' }, T('Ülke, şehir ve isteğe bağlı ilçe'))),
      h('button', { className: 'location-close', type: 'button', disabled: saving, 'aria-label': T('Kapat'), title: T('Kapat'), onClick: cancel }, '×')),
    ...Object.keys(labels).map(kind => h('div', { className: 'location-field', key: kind, 'data-location-field': kind },
      h('label', { htmlFor: `location-${spec.id}-${kind}` }, T(labels[kind])),
      h('input', { id: `location-${spec.id}-${kind}`, ref: element => { inputs.current[kind] = element; }, role: 'combobox', autoComplete: 'off', spellCheck: false,
        'aria-autocomplete': 'list', 'aria-expanded': field === kind && search.results.length > 0, 'aria-controls': `location-results-${spec.id}`,
        'aria-activedescendant': field === kind && index >= 0 ? `location-option-${spec.id}-${index}` : undefined,
        disabled: saving || (kind !== 'country' && !draft.country) || (kind === 'district' && !draft.city), value: draft.texts[kind],
        onFocus: () => setField(kind), onChange: e => { worker.current.query(null); setField(kind); setDraft(editLocationDraft(draft, kind, e.target.value)); setSaveError(null); },
        onKeyDown: e => {
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); setIndex(i => !search.results.length ? -1 : i < 0 ? (e.key === 'ArrowDown' ? 0 : search.results.length - 1) : (i + (e.key === 'ArrowDown' ? 1 : -1) + search.results.length) % search.results.length); }
          if (e.key === 'Enter') { e.preventDefault(); if (field === kind && search.results.length) choose(search.results[Math.max(0, index)]); }
        } }),
      field === kind && search.results.length > 0 && h('div', { className: 'location-results', role: 'listbox', id: `location-results-${spec.id}`, 'aria-label': T(labels[kind]) },
        ...search.results.map((result, i) => h('button', { type: 'button', role: 'option', id: `location-option-${spec.id}-${i}`, key: `${result.code || result.label || result.name}-${i}`, 'aria-selected': i === index,
          'aria-label': kind === 'country' ? result.name : result.label || result.name,
          onPointerDown: e => e.preventDefault(), onClick: () => choose(result) },
          h('span', { className: 'location-result-name' }, result.name || result.label),
          h('span', { className: 'location-result-detail' }, kind === 'country' ? [result.code, result.englishName !== result.name && result.englishName].filter(Boolean).join(' · ') : result.label || result.country)))))),
    h('div', { className: 'location-search-status', role: 'status', 'aria-live': 'polite' }, search.loading ? T('Aranıyor…') : search.error ? h(React.Fragment, null,
      h('span', null, T('Konumlar alınamadı')), h('button', { type: 'button', onClick: () => worker.current.retry({ kind: field, text: draft.texts[field].trim(), language,
        countryCode: draft.country?.code, city: draft.city?.city, latitude: draft.city?.cityLatitude, longitude: draft.city?.cityLongitude }) }, T('Yeniden dene')))
      : field && draft.texts[field].trim() && !draft[field] ? T('Sonuçlardan bir konum seçin') : null),
    spec.recentLocations.length > 0 && h('div', { className: 'location-recents' }, h('span', { className: 'subtext' }, T('Son konumlar')),
      ...spec.recentLocations.map((place, i) => h('button', { type: 'button', key: i, disabled: saving,
        'aria-label': `${place.city}${place.district ? ' / ' + place.district : ''}, ${place.country}`,
        onClick: () => { worker.current.query(null); setDraft(createLocationDraft({ location: place })); setField(null); setSaveError(null); } },
        h('span', { className: 'location-result-name' }, `${place.city}${place.district ? ' / ' + place.district : ''}`), h('span', { className: 'location-result-detail' }, place.country)))),
    saveError && h('div', { role: 'alert', className: 'location-save-error' }, T('Kaydedilemedi'), ': ', saveError),
    h('div', { className: 'location-actions' }, h('button', { type: 'button', disabled: saving, onClick: cancel }, T('İptal')), h('button', { type: 'submit', className: 'location-save', disabled: !valid || saving }, T(saving ? 'Kaydediliyor…' : 'Kaydet'))),
    h('small', { className: 'location-attribution' }, '© ', h('a', { href: 'https://www.openstreetmap.org/copyright', target: '_blank', rel: 'noopener noreferrer' }, 'OpenStreetMap contributors')));
}
