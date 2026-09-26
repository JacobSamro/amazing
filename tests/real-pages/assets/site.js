(function () {
  'use strict';
  document.documentElement.classList.add('js');
  var toggle = document.getElementById('nav-toggle');
  if (toggle) {
    toggle.addEventListener('click', function () {
      document.body.classList.toggle('nav-open');
    });
  }
  var links = document.querySelectorAll('nav.main a');
  for (var i = 0; i < links.length; i++) {
    if (links[i].getAttribute('href') === document.location.pathname) {
      links[i].setAttribute('aria-current', 'page');
    }
  }
  window.__siteReady = true;
})();
