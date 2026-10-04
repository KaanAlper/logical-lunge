// Run the complete UI bundler on an isolated source copy so its generated i18n
// and .build files cannot change files outside the desktop-widget write scope.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const ui = path.join(root, 'ui');
const out = path.join(root, `build/tests/desktop-widgets-ui-build-${process.pid}`);
const copy = path.join(out, 'ui'), dist = path.join(out, 'dist');
assert.ok(copy.startsWith(root + path.sep) && dist.startsWith(root + path.sep));
assert.ok(!fs.existsSync(out), 'each build uses a fresh, isolated output');
fs.cpSync(ui, copy, { recursive: true, filter: source => !['node_modules', '.build', 'dist', 'logical-lunge'].includes(path.basename(source)) });
fs.symlinkSync(path.join(ui, 'node_modules'), path.join(copy, 'node_modules'), 'junction');
const result = spawnSync(process.execPath, [path.join(copy, 'build.mjs'), dist], { cwd: copy, encoding: 'utf8' });
if (result.error) throw result.error;
process.stdout.write(result.stdout); process.stderr.write(result.stderr);
assert.equal(result.status, 0, 'complete production UI bundle');
for (const file of ['desktop-widgets.html', 'desktop-widgets.js', 'desktop-widgets.css']) assert.ok(fs.statSync(path.join(dist, file)).size > 0, file);
for (const file of ['fonts/pixelify-sans-400-700.woff2','fonts/pixelify-sans-OFL.txt']) assert.ok(fs.statSync(path.join(dist,file)).size > 0,file);
assert.ok(!fs.readFileSync(path.join(dist, 'desktop-widgets.html'), 'utf8').includes("import '../desktop-widgets-app.mjs'"));
console.log(`PASS isolated complete UI build: ${dist}`);
