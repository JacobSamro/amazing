(function () {
  var now = Date.now();
  var host = document.getElementById('clock');
  if (host) { host.textContent = String(now); }
})();
