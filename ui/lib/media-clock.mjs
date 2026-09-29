export function trackMedia(clock, s, now = Date.now()) {
  const key = JSON.stringify([s?.sessionId, s?.title, s?.position, s?.positionSeconds, s?.timelineUpdatedAt, s?.isPlaying, s?.playbackRate]);
  if (key !== clock.key) {
    const rate = Number.isFinite(s?.playbackRate) ? s.playbackRate : 1;
    const elapsed = s?.isPlaying && s?.timelineUpdatedAt > 0 ? Math.max(0, (now - s.timelineUpdatedAt) / 1000) * rate : 0;
    Object.assign(clock, { key, pos: (s?.positionSeconds ?? s?.position ?? 0) + elapsed, at: now });
  }
}
export function mediaPosition(clock, s, now = Date.now()) {
  const rate = Number.isFinite(s?.playbackRate) ? s.playbackRate : 1;
  const pos = clock.pos + (s?.isPlaying ? Math.max(0, now - clock.at) / 1000 * rate : 0);
  return Math.max(s?.startTime ?? 0, Math.min(s?.endTime || Infinity, pos));
}
