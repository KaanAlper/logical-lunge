import { glyphOutline } from './widget-geometry.mjs';

// One observer per outlined widget. Async insertions and text/icon updates only
// scan their affected branch; a theme change is the only full content rescan.
export function observeGlyphOutlines(root) {
  const update = element => {
    if (!element?.isConnected || !root.contains(element)) return;
    for (const el of [element, ...element.querySelectorAll('*')]) {
      if (el === root || el.matches('textarea, input') || [...el.childNodes].some(node => node.nodeType === Node.TEXT_NODE && node.textContent.trim())) {
        const value = glyphOutline(getComputedStyle(el).color);
        if (el.style.getPropertyValue('--widget-glyph-outline') !== value) el.style.setProperty('--widget-glyph-outline', value);
      }
    }
  };
  update(root);
  const contentObserver = new MutationObserver(records => {
    const branches = new Set();
    for (const record of records) {
      if (record.type === 'childList') {
        record.addedNodes.forEach(node => branches.add(node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement));
      } else branches.add(record.target.nodeType === Node.ELEMENT_NODE ? record.target : record.target.parentElement);
    }
    for (const branch of branches) if (branch && ![...branches].some(other => other && other !== branch && other.contains(branch))) update(branch);
  });
  contentObserver.observe(root, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['class', 'style'] });
  const themeObserver = new MutationObserver(() => update(root));
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class', 'style'] });
  return () => { contentObserver.disconnect(); themeObserver.disconnect(); };
}
