import test from 'node:test';
import assert from 'node:assert/strict';
import { buildReport, submitReport, removeLibraryItem, saveUiScale, expandedToggle } from './parity-sidebar.mjs';

const draft = { kind: 'performans', start: '2026-10-03T12:30', end: '', message: '  slow  ', selected: { blackbox: true, core: true, shell: false, tiling: true, device: false } };
const files = { blackbox: { text: 'snapshot' }, core: { text: 'core tail' }, shell: { text: 'private' }, tiling: { error: 'missing' } };
test('report validates dates/message and includes only selected diagnostics', () => {
  const report = buildReport(draft, files, { cpu: 'private cpu' }, { timezone: 'Europe/Istanbul' });
  assert.equal(report.description, 'slow');
  assert.equal(report.incident_end, null);
  assert.equal(report.core_log, 'core tail');
  assert.equal(report.shell_log, null);
  assert.equal(report.cpu, null);
  assert.deepEqual(report.device_info.missingLogs, ['tiling.log']);
  assert.throws(() => buildReport({ ...draft, message: ' ' }, files), /açıklayın/);
  assert.throws(() => buildReport({ ...draft, end: '2026-10-03T11:30' }, files), /zaman/);
  assert.throws(() => buildReport({ ...draft, start: '2026-02-30T10:00' }, files), /zaman/);
});
test('only the native missing-column response retries with combined logs', async () => {
  const requests = [];
  const report = buildReport(draft, files);
  const transport = async (url, init) => {
    requests.push({ url, init, body: JSON.parse(init.body) });
    return requests.length === 1 ? new Response(JSON.stringify({ code: 'PGRST204', message: 'core_log missing' }), { status: 400 }) : new Response(null, { status: 201 });
  };
  await submitReport(report, transport);
  assert.equal(requests.length, 2);
  assert.equal(requests[0].url, 'https://bygirolbhyziitvnaxln.supabase.co/rest/v1/bug_reports');
  assert.equal(requests[0].init.headers.Prefer, 'return=minimal');
  assert.equal(requests[1].body.core_log, undefined);
  assert.match(requests[1].body.system_log, /===== core.log =====\ncore tail/);
  assert.doesNotMatch(requests[1].body.system_log, /private/);
  let count = 0;
  await assert.rejects(submitReport(report, async () => { count++; return new Response('denied', { status: 403 }); }), /403/);
  assert.equal(count, 1);
});
test('library removal and scale use POST/204 contracts and report errors', async () => {
  const requests = [];
  const transport = async (url, init) => { requests.push({ url, init }); return new Response(null, { status: 204 }); };
  await removeLibraryItem('live', 'C:\\library\\a & b.mp4', transport);
  assert.equal(new URL(requests[0].url).searchParams.get('path'), 'C:\\library\\a & b.mp4');
  assert.equal(requests[0].init.method, 'POST');
  for (const value of [85, 90, 100, 110, 125, 150]) await saveUiScale(value, transport);
  assert.match(requests.at(-1).url, /pref\?k=uiScale&v=150$/);
  await assert.rejects(saveUiScale(95, transport));
  await assert.rejects(removeLibraryItem('wall', 'x', async () => new Response(null, { status: 500 })), /500/);
  assert.equal(expandedToggle(117), false);
  assert.equal(expandedToggle(118), true);
});

test('a switch is judged by what its source shows (every combination)', async () => {
  const { verdict } = await import('./parity-sidebar.mjs');
  for (const want of [false, true]) for (const seen of [undefined, null, false, true]) for (const left of [0, 500]) {
    const shown = seen === want;
    const expected = shown ? 'done' : left > 0 ? 'again' : seen === null ? 'done' : 'failed';
    assert.equal(verdict(want, seen, left), expected, `want ${want}, seen ${seen}, left ${left}`);
  }
});

test('every switch tile reads its state back and nothing else does', async () => {
  const { switchedIn } = await import('./parity-sidebar.mjs');
  const allOn = { radios: { wifi: 'On', bluetooth: 'On' }, eth: { state: 'up' }, mic: true, night: { on: true }, awake: true };
  const allOff = { radios: { wifi: 'Off', bluetooth: 'Off' }, eth: { state: 'disabled' }, mic: false, night: { on: false }, awake: false };
  for (const tile of ['wifi', 'bluetooth', 'ethernet', 'mic', 'nightLight', 'idleInhibitor']) {
    assert.equal(switchedIn(tile, allOn), true, tile);
    assert.equal(switchedIn(tile, allOff), false, tile);
    assert.equal(switchedIn(tile, {}), null, tile);
  }
  for (const tile of ['audio', 'darkMode', 'screenSnip', 'onScreenKeyboard', 'notifications']) assert.equal(switchedIn(tile, allOn), null, tile);
  assert.equal(switchedIn('ethernet', { eth: { state: 'disconnected' } }), null, 'enabled without a cable is neither');
});

test('a switch is read again until its source agrees, and fails when it never does', async () => {
  const { checkSwitch, VERIFY_EVERY } = await import('./parity-sidebar.mjs');
  const run = async reads => {
    let t = 0, i = 0; const shown = [];
    const r = await checkSwitch({ tile: 'mic', want: true, waitMs: 2000, now: () => t, sleep: async ms => { t += ms; },
      read: async () => { const v = reads[Math.min(i++, reads.length - 1)]; if (v instanceof Error) throw v; return v; }, apply: p => shown.push(p.mic) });
    return { ...r, shown, reads: i, elapsed: t };
  };
  const late = await run([{ mic: false }, { mic: false }, { mic: true }]);
  assert.deepEqual([late.ok, late.answered, late.shown, late.reads], [true, true, [false, false, true], 3]);
  const opposite = await run([{ mic: false }]);
  assert.equal(opposite.ok, false); assert.equal(opposite.answered, true); assert.ok(opposite.elapsed >= 2000 && opposite.elapsed < 2000 + VERIFY_EVERY * 2);
  const silent = await run([new Error('no core')]);
  assert.deepEqual([silent.ok, silent.answered, silent.shown.length], [false, false, 0]);
  const neither = await run([{}]);
  assert.equal(neither.ok, true, 'neither on nor off is not a failure');
});
