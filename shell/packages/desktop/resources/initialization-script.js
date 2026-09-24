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

// Clear console every 15 minutes.
setInterval(
  () => {
    console.clear();
    console.info(
      '%c[Zebar]%c Console is cleared every 15 minutes to prevent memory buildup from logged data.',
      'color: #4ade80',
      'color: inherit',
    );
  },
  1000 * 60 * 15,
);

if (window.location.host === '127.0.0.1:6124') {
  if ('serviceWorker' in navigator) {
    navigator.serviceWorker
      .register('/__zebar/sw.js', { scope: '/' })
      .then(sw => {
        console.info(
          '%c[Zebar]%c Service Worker registered.',
          'color: #4ade80',
          'color: inherit',
        );

        const message = {
          type: 'SET_CONFIG',
          config: window.__ZEBAR_STATE.config.caching,
        };

        sw.active?.postMessage(message);
        sw.installing?.postMessage(message);
        sw.waiting?.postMessage(message);
      })
      .catch(err =>
        console.error(
          '%c[Zebar]%c Service Worker failed to register:',
          'color: #4ade80',
          'color: inherit',
          err,
        ),
      );
  }

  document.addEventListener('DOMContentLoaded', () => {
    addFavicon();
    loadCss('/__zebar/normalize.css');
  });
}

/**
 * Adds a CSS file with the given path to the head element.
 */
function loadCss(path) {
  const link = document.createElement('link');
  link.setAttribute('data-zebar', '');
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
    link.setAttribute('data-zebar', '');
    link.rel = 'icon';
    link.href = 'data:;';
    insertIntoHead(link);
  }
}

/**
 * Inserts the element before any other resource tags in the head element.
 * Ensures that user-defined stylesheets or favicons are prioritized over
 * Zebar's defaults.
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
