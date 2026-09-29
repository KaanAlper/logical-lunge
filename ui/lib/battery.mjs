// Battery provider times are milliseconds; powerConsumption is watts.
export function batteryTime(ms) {
  if (!Number.isFinite(ms) || ms <= 0) return null;
  const minutes = Math.ceil(ms / 60000);
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}`;
}
export function batteryDetails(b) {
  const state = b.isCharging ? 'Şarj oluyor' : b.state?.toLowerCase() === 'full' ? 'Tam dolu'
    : ['discharging', 'empty'].includes(b.state?.toLowerCase()) ? 'Pil kullanılıyor' : 'Veri yok';
  return {
    percent: Math.round(Math.min(100, Math.max(0, b.chargePercent))),
    state,
    timeLabel: b.isCharging ? 'Dolmasına kalan' : 'Bitmesine kalan',
    time: batteryTime(b.isCharging ? b.timeTillFull : b.timeTillEmpty) ?? 'Veri yok',
    // Windows' battery library maps an unknown rate to zero. Do not invent a reading.
    power: Number.isFinite(b.powerConsumption) && Math.abs(b.powerConsumption) > 0 ? `${Math.abs(b.powerConsumption).toFixed(1)} W` : 'Veri yok',
  };
}
