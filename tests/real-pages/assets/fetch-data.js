fetch('../assets/data.json').then(function (response) {
  return response.json();
}).then(function (data) {
  var host = document.getElementById('remote');
  if (host) { host.textContent = data.title; }
}).catch(function (error) {
  var host = document.getElementById('remote');
  if (host) { host.textContent = 'unavailable'; }
});
