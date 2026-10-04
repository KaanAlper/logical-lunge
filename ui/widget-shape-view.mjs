import React from 'react';
const h = React.createElement;
export function ShapeChrome({ geometry: g, id }) {
  return h('svg', { className: 'widget-shape-svg', viewBox: `0 0 ${g.w} ${g.h}`, 'aria-hidden': true },
    h('defs', null, h('clipPath', { id, clipPathUnits: 'userSpaceOnUse' }, h('path', { d: g.path, clipRule: 'evenodd' }))),
    g.shape !== 'card' && h('path', { className: 'shape-outline', d: g.path, fillRule: 'evenodd' }),
    g.shape === 'ticket' && h('path', { className: 'shape-perforation', d: `M${g.w * .3},12V${g.h - 12}` }),
    g.shape === 'circle' && h('ellipse', { className: 'shape-orbit', cx: g.w / 2, cy: g.h / 2, rx: g.w * .44, ry: g.h * .44 }),
    g.shape === 'polaroid' && h('path', { className: 'shape-tape', d: `M${g.w * .34},3L${g.w * .67},0L${g.w * .66},30L${g.w * .33},33Z` }));
}
export function shapeContentStyle(g) {
  return g.shape === 'card' ? undefined : { left: g.inner.x, top: g.inner.y, width: g.inner.w, height: g.inner.h,
    '--shape-leading-width': `${g.leading - 24}px`, '--shape-pane-gap': '36px' };
}
export function shapeControlStyle(rect) { return { left: rect.x, top: rect.y, right: 'auto', bottom: 'auto' }; }
