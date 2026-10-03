import assert from 'node:assert/strict';
import '../../ui/file-search.js';

const calls = [], replies = [];
let state;
const pager = new globalThis.LLFileSearchPager((command, args) => {
  assert.equal(command, 'everything_search_page');
  calls.push(args);
  return new Promise((resolve, reject) => replies.push({ resolve, reject }));
}, value => { state = value; });
const page = (offset, count, total = 60) => ({ offset, total, hits: Array.from({length:count}, (_, i) => ({fullPath:`C:/files/${offset+i}`, name:`${offset+i}`})) });
pager.reset('# files', 'files');
for (let offset = 0; offset < 60; offset += 10) {
  const work = pager.load();
  pager.load(); // Wheel/keyboard events during the request do not duplicate it.
  assert.equal(calls.length, offset / 10 + 1);
  assert.deepEqual(calls.at(-1), {query:'files', maxResults:10, offset});
  assert.equal(state.loading, true);
  replies.shift().resolve(page(offset, 10));
  await work;
  assert.equal(state.hits.length, offset + 10);
  assert.equal(state.hits[0].name, '0');
}
await pager.load();
assert.equal(calls.length, 6);
assert.equal(state.done, true);

pager.reset('# old', 'old');
const stale = pager.load(), oldReply = replies.shift();
pager.cancel();
pager.reset('# new', 'new');
const current = pager.load();
oldReply.resolve(page(0, 10));
await stale;
assert.equal(state.query, '# new');
assert.equal(state.hits.length, 0);
assert.equal(state.loading, true);
replies.shift().resolve(page(0, 10, 100));
await current;
const failing = pager.load();
replies.shift().reject(new Error('timeout'));
await failing;
assert.equal(state.hits.length, 10);
assert.equal(state.done, true);

pager.reset('# empty', 'empty');
const empty = pager.load();
replies.shift().resolve(page(0, 0, 100));
await empty;
await pager.load();
assert.equal(state.done, true);
assert.equal(state.loading, false);
console.log('File paging: 60 rows, concurrent requests, stale query, retained rows and empty last page passed.');
