// Logical Lunge: widgets are part of the shell, not web pages. The browser's own
// context menu (Reload / Save as / Print / Inspect) and its reload, print, save,
// find, view-source, zoom and back/forward keys are disabled. Only the default
// action is prevented, so a widget's own right-click and key handlers still run;
// text editing keys (Ctrl+A/C/V/X/Z/Y, Ctrl+arrows) are untouched.
(() => {
  const BLOCKED_KEYS = new Set(['F3', 'F5', 'F7', 'F12', 'BrowserBack', 'BrowserForward', 'BrowserRefresh']);
  const BLOCKED_CTRL = new Set(['r', 'p', 's', 'f', 'g', 'u', 'o', 'j', 'h', 'd', 'l', 'n', 't', 'w', '+', '=', '-', '0']);

  document.addEventListener('contextmenu', e => e.preventDefault(), true);
  document.addEventListener(
    'keydown',
    e => {
      const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
      const ctrl = e.ctrlKey || e.metaKey;
      if (
        BLOCKED_KEYS.has(key) ||
        (ctrl && BLOCKED_CTRL.has(key)) ||
        (ctrl && e.shiftKey && ['i', 'j', 'c'].includes(key)) ||
        (e.altKey && !ctrl && (key === 'ArrowLeft' || key === 'ArrowRight'))
      ) {
        e.preventDefault();
      }
    },
    true,
  );
  document.addEventListener('wheel', e => e.ctrlKey && e.preventDefault(), { capture: true, passive: false });
})();

// The widgets and all their libraries and fonts are served by the shell itself
// (no network requests), so no service worker cache is needed; widgets don't log
// provider output, so the console doesn't need periodic clearing either.
if (window.location.host === '127.0.0.1:6124') {
  document.addEventListener('DOMContentLoaded', () => {
    addFavicon();
    loadCss('/__shell/normalize.css');
  });
}

/**
 * Adds a CSS file with the given path to the head element.
 */
function loadCss(path) {
  const link = document.createElement('link');
  link.setAttribute('data-shell', '');
  link.rel = 'stylesheet';
  link.type = 'text/css';
  link.href = path;
  insertIntoHead(link);
}

/**
 * Adds a favicon to the head element if one is not already present.
 */
function addFavicon() {
  if (!document.querySelector('link[rel="icon"]')) {
    const link = document.createElement('link');
    link.setAttribute('data-shell', '');
    link.rel = 'icon';
    link.href = 'data:;';
    insertIntoHead(link);
  }
}

/**
 * Inserts the element before any other resource tags in the head element.
 * Ensures that user-defined stylesheets or favicons are prioritized over
 * the shell's defaults.
 */
function insertIntoHead(element) {
  const resources = document.head.querySelectorAll('link, script, style');
  const target = resources[0]?.previousElementSibling;

  if (target) {
    target.after(element);
  } else {
    document.head.appendChild(element);
  }
}
