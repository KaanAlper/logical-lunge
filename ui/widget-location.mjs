import { normalizeLocation, normalizeRecents } from './desktop-widgets-model.mjs';

export function createLocationDraft(spec) {
  const place = normalizeLocation(spec.location);
  return {
    country: place ? { code: place.countryCode, name: place.country } : null,
    city: place ? { ...place, district: '', latitude: place.cityLatitude, longitude: place.cityLongitude } : null,
    district: place?.district ? place : null,
    texts: { country: place?.country ?? '', city: place?.city ?? spec.city ?? '', district: place?.district ?? '' },
  };
}
export function editLocationDraft(draft, field, text) {
  const next = { ...draft, texts: { ...draft.texts, [field]: text }, [field]: null };
  if (field === 'country') { next.city = null; next.texts.city = ''; }
  if (field !== 'district') { next.district = null; next.texts.district = ''; }
  return next;
}
export function choosePlace(draft, field, result) {
  if (field === 'country') {
    if (!result?.code || !result?.name) return draft;
    return { country: { code: result.code, name: result.name }, city: null, district: null, texts: { country: result.name, city: '', district: '' } };
  }
  if (!draft.country || result.countryCode !== draft.country.code) return draft;
  if (field === 'district' && (!draft.city || result.city !== draft.city.city)) return draft;
  const place = normalizeLocation({ ...result, country: draft.country.name, city: field === 'city' ? result.city || result.name : draft.city.city,
    district: field === 'district' ? result.district || result.name : '',
    cityLatitude: field === 'city' ? result.latitude : draft.city.latitude, cityLongitude: field === 'city' ? result.longitude : draft.city.longitude });
  if (!place || (field === 'district' && !place.district)) return draft;
  return field === 'city' ? { ...draft, city: place, district: null, texts: { ...draft.texts, city: place.city, district: '' } }
    : { ...draft, district: place, texts: { ...draft.texts, district: place.district } };
}
export function locationPatch(draft, recents) {
  if (!draft.country || !draft.city || (draft.texts.district.trim() && !draft.district)) return null;
  const location = normalizeLocation(draft.district || draft.city);
  if (!location) return null;
  const recentLocations = normalizeRecents([location, ...(Array.isArray(recents) ? recents : [])]);
  return { location, city: location.district ? `${location.city} / ${location.district}` : location.city, recentLocations };
}
export function weatherParams(spec, language) {
  const params = new URLSearchParams({ language, fahrenheit: spec.fahrenheit ? '1' : '0' });
  const place = normalizeLocation(spec.location);
  if (place) {
    params.set('latitude', place.latitude); params.set('longitude', place.longitude);
    params.set('place', place.district ? `${place.city} / ${place.district}` : place.city);
  } else params.set('city', spec.city || '');
  return params;
}

// One request at a time, even if a transport takes time to acknowledge abort.
// A generation invalidates results immediately; the latest debounced query waits
// for that request to settle. cancel/dispose cover scope changes and unmount.
export function createLatestSearch(load, publish, { setTimer = setTimeout, clearTimer = clearTimeout, delay = 350 } = {}) {
  let version = 0, pending = null, inflight = null, timer = null, disposed = false;
  const state = (results = [], loading = false, error = null) => ({ results, loading, error });
  const run = () => {
    if (disposed || inflight || !pending?.ready) return;
    const job = pending; pending = null;
    const controller = new AbortController(); inflight = controller;
    Promise.resolve().then(() => load(job.query, controller.signal)).then(results => {
      if (!disposed && job.version === version) publish(state(Array.isArray(results) ? results : []));
    }, error => {
      if (!disposed && job.version === version) publish(state([], false, String(error)));
    }).finally(() => { inflight = null; run(); });
  };
  const cancel = () => { version++; clearTimer(timer); pending = null; inflight?.abort(); };
  const query = (value, wait = delay) => {
    if (disposed) return;
    cancel();
    if (!value) { publish(state()); return; }
    pending = { query: value, version, ready: false }; publish(state([], true));
    timer = setTimer(() => { if (pending) pending.ready = true; run(); }, wait);
  };
  return { query, retry: value => query(value, 0), cancel, dispose: () => { cancel(); disposed = true; } };
}
