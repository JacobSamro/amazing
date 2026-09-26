function renderCard(title, body) {
  var card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = '<h3>' + title + '</h3><p>' + body + '</p>';
  return card;
}
document.addEventListener('DOMContentLoaded', function () {
  var host = document.getElementById('widgets');
  if (host) {
    host.appendChild(renderCard('Related', 'Rendered by widgets.js'));
    host.appendChild(renderCard('Popular', 'Second generated card'));
  }
});
