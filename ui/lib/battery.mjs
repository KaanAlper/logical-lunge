// Battery provider times are milliseconds; powerConsumption is watts.
export function batteryTime(ms) {
  if (!Number.isFinite(ms) || ms <= 0) return null;
  const minutes = Math.ceil(ms / 60000);
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}`;
}
export function batteryDetails(b) {
  const plugged = b.isPlugged ?? b.isCharging;
  const state = plugged && b.state?.toLowerCase() === 'full' ? 'Tam dolu'
    : b.isCharging ? 'Şarj oluyor' : plugged ? 'Prize takılı' : 'Pil kullanılıyor';
  return {
    percent: Math.round(Math.min(100, Math.max(0, b.chargePercent))),
    state,
    timeLabel: plugged ? 'Dolmasına kalan' : 'Bitmesine kalan',
    time: batteryTime(b.isCharging ? b.timeTillFull : plugged ? null : b.timeTillEmpty) ?? 'Veri yok',
    // Windows' battery library maps an unknown rate to zero. Do not invent a reading.
    power: Number.isFinite(b.powerConsumption) && Math.abs(b.powerConsumption) > 0 ? `${Math.abs(b.powerConsumption).toFixed(1)} W` : 'Veri yok',
  };
}
