import assert from 'node:assert/strict';
import { batteryTime, batteryDetails } from '../../ui/lib/battery.mjs';
import { trackMedia, mediaPosition } from '../../ui/lib/media-clock.mjs';
import { appIconFor } from '../../ui/lib/app-icons.js';
assert.equal(appIconFor([
  { name: 'Internet Explorer', path: 'shell:AppsFolder\\Microsoft.InternetExplorer.Default', icon: 'browser' },
  { name: 'Dosya Gezgini', also: 'File Explorer', path: 'shell:AppsFolder\\Microsoft.Windows.Explorer', icon: 'folder' },
], 'explorer'), 'folder');
assert.equal(batteryTime(5400000), '1:30');
for (const value of [undefined, null, 0, -1, NaN, Infinity]) assert.equal(batteryTime(value), null);
const battery = { chargePercent: 48.7, isCharging: true, state: 'charging', powerConsumption: 25.6, timeTillFull: 5400000 };
assert.deepEqual(batteryDetails(battery), { percent: 49, state: 'Şarj oluyor', timeLabel: 'Dolmasına kalan', time: '1:30', power: '25.6 W' });
assert.equal(batteryDetails({ ...battery, isCharging: false, timeTillEmpty: 3600000 }).time, '1:00');
assert.equal(batteryDetails({ ...battery, isPlugged: true, isCharging: false, timeTillEmpty: 3600000 }).state, 'Prize takılı');
assert.equal(batteryDetails({ ...battery, isPlugged: true, isCharging: false, timeTillEmpty: 3600000 }).time, 'Veri yok');
assert.equal(batteryDetails({ ...battery, isPlugged: false, isCharging: false, state: 'full' }).state, 'Pil kullanılıyor');
assert.equal(batteryDetails({ ...battery, powerConsumption: 0 }).power, 'Veri yok');
const clock = {};
let s = { sessionId: 'browser', title: 'video', position: 10, positionSeconds: 10.5, timelineUpdatedAt: 100000, isPlaying: true, playbackRate: 2, endTime: 500 };
trackMedia(clock, s, 102000);
assert.equal(mediaPosition(clock, s, 103000), 16.5);
trackMedia(clock, s, 105000); // polling the same Windows snapshot must not reset the clock
assert.equal(mediaPosition(clock, s, 105000), 20.5);
s = { ...s, position: 100, positionSeconds: 100, timelineUpdatedAt: 105000 };
trackMedia(clock, s, 105100);
assert.equal(mediaPosition(clock, s, 106000), 102);
s = { ...s, position: 10, positionSeconds: 10, timelineUpdatedAt: 106000 }; // seek back
trackMedia(clock, s, 106000);
assert.equal(mediaPosition(clock, s, 106000), 10);
s = { ...s, sessionId: 'other', isPlaying: false };
trackMedia(clock, s, 107000);
assert.equal(mediaPosition(clock, s, 109000), 10);
console.log('PASS battery units, missing readings, media seek, stale snapshots, playback rate and session changes');
