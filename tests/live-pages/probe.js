// IA-LIVE-PAGES API probe.
//
// Runs in the page's own interpreter after the page's scripts, events and
// bounded event-loop steps. For each path it prints one machine-readable line:
//
//     API|<path>|<typeof value>
//
// where <typeof value> is "undefined" when the API is absent, "error" when the
// traversal itself threw, or the JS typeof string otherwise. The live harness
// turns "undefined" into the top-missing-APIs report. Nothing here reads or
// writes page state; it only inspects globals.
(function () {
  'use strict';
  var paths = [
    // Fetch and transport
    'fetch', 'XMLHttpRequest', 'Request', 'Response', 'Headers', 'FormData',
    'URL', 'URLSearchParams', 'AbortController', 'AbortSignal',
    'WebSocket', 'EventSource', 'Worker', 'SharedWorker', 'MessageChannel',
    'BroadcastChannel',
    // Observation and scheduling
    'IntersectionObserver', 'ResizeObserver', 'MutationObserver',
    'PerformanceObserver', 'matchMedia', 'getComputedStyle',
    'requestAnimationFrame', 'requestIdleCallback', 'queueMicrotask',
    // Storage and crypto
    'localStorage', 'sessionStorage', 'indexedDB', 'caches',
    'crypto', 'crypto.subtle', 'crypto.getRandomValues', 'structuredClone',
    // Navigator surface
    'navigator', 'navigator.clipboard', 'navigator.geolocation',
    'navigator.serviceWorker', 'navigator.sendBeacon', 'navigator.userAgent',
    'navigator.language', 'navigator.cookieEnabled', 'navigator.share',
    'navigator.permissions', 'navigator.storage',
    // History, location, events
    'history', 'history.pushState', 'history.replaceState', 'location',
    'CustomEvent', 'Event', 'EventTarget', 'DOMException', 'postMessage',
    // Encoding and text
    'TextEncoder', 'TextDecoder', 'atob', 'btoa', 'DOMParser', 'XMLSerializer',
    // Performance and CSSOM
    'performance', 'performance.now', 'performance.mark', 'CSS',
    'getSelection', 'document.createRange',
    // DOM and elements
    'Element.prototype.animate', 'Element.prototype.closest',
    'Element.prototype.attachShadow', 'customElements', 'customElements.define',
    'Node', 'NodeList', 'HTMLCollection', 'SVGElement',
    // Capabilities
    'Notification', 'Permissions', 'URLPattern', 'WebAssembly',
    'WebGLRenderingContext', 'OffscreenCanvas', 'CanvasRenderingContext2D',
    'AudioContext', 'RTCPeerConnection', 'MediaQueryList',
    // Internationalization
    'Intl', 'Intl.DateTimeFormat', 'Intl.NumberFormat', 'Intl.Collator'
  ];
  function kind(path) {
    try {
      var value = globalThis;
      var parts = path.split('.');
      for (var i = 0; i < parts.length; i++) {
        if (value === null || value === undefined) { return 'undefined'; }
        value = value[parts[i]];
      }
      return typeof value;
    } catch (error) {
      return 'error';
    }
  }
  for (var i = 0; i < paths.length; i++) {
    console.log('API|' + paths[i] + '|' + kind(paths[i]));
  }
})();
