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
