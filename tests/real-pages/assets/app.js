(function () {
  'use strict';
  var state = { count: 0 };
  function render() {
    var out = document.getElementById('counter');
    if (out) { out.textContent = String(state.count); }
  }
  var button = document.getElementById('increment');
  if (button) {
    button.addEventListener('click', function () { state.count += 1; render(); });
  }
  render();
  requestAnimationFrame(function () { document.body.dataset.frame = '1'; });
})();
