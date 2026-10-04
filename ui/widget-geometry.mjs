// One geometry contract for SVG clipping, content placement and Windows HRGN.
// Coordinates are local DIP until physicalRegions applies the actual CSS/DPI scale.
export const SHAPES = { card: 'Kart', capsule: 'Kapsül', circle: 'Daire', ticket: 'Bilet', bubble: 'Konuşma balonu', hexagon: 'Altıgen', polaroid: 'Polaroid', split: 'Ayrık paneller' };
const minimums = { card: [0, 0], capsule: [320, 144], circle: [224, 224], ticket: [288, 168], bubble: [248, 200], hexagon: [288, 240], polaroid: [248, 288], split: [320, 168] };
export function shapeMinimum(shape, min = [0, 0]) { const m = minimums[shape] || minimums.card; return [Math.max(m[0], min[0]), shape === 'bubble' ? Math.max(168, min[1]+56) : Math.max(m[1], min[1])]; }
export function constrainShapeRect(shape, rect, aw, ah, min) {
  const [mw, mh] = shapeMinimum(shape, min);
  let w = Math.min(aw, Math.max(mw, rect.w)), h = Math.min(ah, Math.max(mh, rect.h));
  if (shape === 'circle') w = h = Math.min(aw, ah, Math.max(w, h, mw, mh));
  if (shape === 'capsule') { w = Math.min(aw, Math.max(w, h * 1.8)); h = Math.min(h, w / 1.8); }
  return { x: Math.max(0, Math.min(rect.x, aw - w)), y: Math.max(0, Math.min(rect.y, ah - h)), w, h };
}
export function normalizeShapeSizes(raw) {
  return Object.fromEntries(Object.keys(SHAPES).filter(shape => {
    const size = raw?.[shape];
    return Array.isArray(size) && size.length === 2 && size.every(n => Number.isFinite(n) && n > 0 && n <= 10000);
  }).map(shape => [shape, [...raw[shape]]]));
}
export function selectShape(spec, shape, aw, ah, min, preferred = [spec.w, spec.h]) {
  shape = Object.hasOwn(SHAPES, shape) ? shape : 'card';
  if (shape === spec.shape) return {};
  const shapeSizes = normalizeShapeSizes(spec.shapeSizes);
  shapeSizes[spec.shape || 'card'] = [spec.w, spec.h];
  const [w,h] = shapeSizes[shape] || preferred;
  return { shape, shapeSizes, ...constrainShapeRect(shape, { ...spec, w, h }, aw, ah, min) };
}
const box = (x, y, w, h) => ({ x, y, w, h });
const rounded = (x, y, w, h, radius) => ({ ...box(x, y, w, h), shape: 'roundRect', radius });
const ellipse = (x, y, w, h) => ({ ...box(x, y, w, h), shape: 'ellipse' });
const polygon = (w, h, points) => ({ ...box(0, 0, w, h), shape: 'polygon', points: points.map(([x, y]) => ({ x, y })) });
export function shapeGeometry(shape = 'card', w, h) {
  const leading = Math.min(100, Math.max(64, w * .28));
  let regions = [rounded(0, 0, w, h, 18)], inner = box(16, 14, w - 32, h - 28);
  let toolbar = box(w - 34, 4, 28, 28), grip = box(w - 28, h - 28, 22, 22);
  if (shape === 'capsule') {
    regions = [rounded(0, 0, w, h, h / 2)]; const inset = Math.max(22, h * .15 + 10);
    inner = box(inset, 20, w - inset * 2, h - 40); toolbar = box(w * .62 - 14, 7, 28, 28); grip = box(w - h * .38 - 22, h * .6, 22, 22);
  } else if (shape === 'circle') {
    regions = [ellipse(0, 0, w, h)]; inner = box(w * .16, h * .16, w * .68, h * .68);
    toolbar = box(w / 2 - 14, 9, 28, 28); grip = box(w * .75 - 11, h * .75 - 11, 22, 22);
  } else if (shape === 'ticket') {
    regions = [{ ...rounded(0, 0, w, h, 12), holes: [ellipse(-14, h / 2 - 14, 28, 28), ellipse(w - 14, h / 2 - 14, 28, 28)] }];
    inner = box(24, 18, w - 48, h - 36); toolbar = box(w - 46, 7, 28, 28); grip = box(w - 42, h - 30, 22, 22);
  } else if (shape === 'bubble') {
    regions = [rounded(0, 0, w, h - 22, 20), polygon(w, h, [[w * .25, h - 22], [w * .32, h], [w * .43, h - 22]])];
    inner = box(20, 20, w - 40, h - 62); toolbar = box(w - 46, 7, 28, 28); grip = box(w - 42, h - 52, 22, 22);
  } else if (shape === 'hexagon') {
    regions = [polygon(w, h, [[0, h / 2], [w * .18, 0], [w * .82, 0], [w, h / 2], [w * .82, h], [w * .18, h]])];
    inner = box(w * .2, h * .16, w * .6, h * .68); toolbar = box(w / 2 - 14, 7, 28, 28); grip = box(w * .7 - 11, h * .78 - 11, 22, 22);
  } else if (shape === 'polaroid') {
    regions = [rounded(8, 18, w - 16, h - 18, 3), polygon(w, h, [[w * .34, 3], [w * .67, 0], [w * .66, 30], [w * .33, 33]])];
    inner = box(24, 40, w - 48, h - 70); toolbar = box(w - 46, 24, 28, 28); grip = box(w - 42, h - 30, 22, 22);
  } else if (shape === 'split') {
    regions = [rounded(0, 0, leading, h, 14), rounded(leading + 12, 0, w - leading - 12, h, 14)];
    inner = box(12, 14, w - 24, h - 28); toolbar = box(w - 42, 5, 28, 28); grip = box(w - 32, h - 30, 22, 22);
  }
  inner.w = Math.max(1, inner.w); inner.h = Math.max(1, inner.h);
  return { shape, w, h, regions, inner, toolbar, grip, leading, path: regions.map(regionPath).join(' ') };
}
export function regionPath(r) {
  const { x, y, w, h } = r;
  let path;
  if (r.shape === 'polygon') path = r.points.map((p, i) => `${i ? 'L' : 'M'}${x + p.x},${y + p.y}`).join(' ') + 'Z';
  else if (r.shape === 'ellipse') path = `M${x},${y + h / 2}a${w / 2},${h / 2} 0 1 0 ${w},0a${w / 2},${h / 2} 0 1 0 ${-w},0Z`;
  else { const a = Math.min(r.radius || 0, w / 2, h / 2); path = `M${x + a},${y}H${x + w - a}A${a},${a} 0 0 1 ${x + w},${y + a}V${y + h - a}A${a},${a} 0 0 1 ${x + w - a},${y + h}H${x + a}A${a},${a} 0 0 1 ${x},${y + h - a}V${y + a}A${a},${a} 0 0 1 ${x + a},${y}Z`; }
  return path + (r.holes || []).map(regionPath).join(' ');
}
export function glyphOutline(color) {
  const values = color.match(/[\d.]+/g)?.map(Number) || [230, 230, 230];
  const rgb = color.startsWith('color(') ? values.slice(0, 3).map(n => n * 255) : values;
  return rgb[0] * .2126 + rgb[1] * .7152 + rgb[2] * .0722 > 140 ? '#101016' : '#ffffff';
}
export function physicalRegions(g, { x, y, scaleX, scaleY }) {
  const transform = r => ({ ...r, x: x + r.x * scaleX, y: y + r.y * scaleY, w: r.w * scaleX, h: r.h * scaleY,
    ...(r.radius != null ? { radius: r.radius * Math.min(scaleX, scaleY) } : {}),
    ...(r.points ? { points: r.points.map(p => ({ x: p.x * scaleX, y: p.y * scaleY })) } : {}),
    ...(r.holes ? { holes: r.holes.map(transform) } : {}) });
  return g.regions.map(transform);
}
export function regionContains(regions, x, y) {
  const contains = r => {
    const px = x - r.x, py = y - r.y;
    if (r.holes?.some(contains)) return false;
    if (r.shape === 'polygon') {
      let inside = false; const p = r.points;
      for (let i = 0, j = p.length - 1; i < p.length; j = i++) if ((p[i].y > py) !== (p[j].y > py) && px < (p[j].x - p[i].x) * (py - p[i].y) / (p[j].y - p[i].y) + p[i].x) inside = !inside;
      return inside;
    }
    if (px < 0 || py < 0 || px > r.w || py > r.h) return false;
    if (r.shape === 'ellipse') return ((px - r.w / 2) / (r.w / 2)) ** 2 + ((py - r.h / 2) / (r.h / 2)) ** 2 <= 1;
    const radius = Math.min(r.radius || 0, r.w / 2, r.h / 2);
    if (!radius) return true;
    const dx = px - Math.max(radius, Math.min(px, r.w - radius)), dy = py - Math.max(radius, Math.min(py, r.h - radius));
    return dx * dx + dy * dy <= radius * radius;
  };
  return regions.some(contains);
}
