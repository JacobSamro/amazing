try {
  localStorage.setItem('last-visit', String(Date.now()));
  var seen = localStorage.getItem('last-visit');
  var host = document.getElementById('seen');
  if (host) { host.textContent = seen ? 'returning' : 'new'; }
} catch (error) {
  var host = document.getElementById('seen');
  if (host) { host.textContent = 'unknown'; }
}
