const title = document.querySelector('#title');
document.title = 'Ferric test';
if (document.title === 'Ferric test' && window === globalThis) title.setAttribute('data-window', 'yes');
document.addEventListener('DOMContentLoaded', () => title.setAttribute('data-ready', 'yes'));
document.querySelector('main').addEventListener('click', (event) => title.setAttribute('data-bubbled', event.currentTarget.getAttribute('id') === 'root' ? 'yes' : 'no'));
if (document.querySelectorAll('p').length === 1 && document.getElementById('title') !== null) title.setAttribute('data-dom-api', 'yes');
if (title.innerHTML === 'before') { title.innerHTML = 'before'; title.setAttribute('data-inner', 'yes'); }
title.onclick = (event) => { title.textContent = event.type + '!'; title.style.color = 'red'; };
title.addEventListener('click', () => title.setAttribute('data-clicked', 'yes'));
setTimeout(() => title.setAttribute('data-timer', 'yes'), 0);
fetch('phase8-data.txt').then((response) => { if (response.headers.has('content-type')) title.setAttribute('data-header', response.headers.get('content-type')); return response.text(); }).then((text) => title.setAttribute('data-fetch', text));
fetch('phase8-data.json').then((response) => response.json()).then((value) => title.setAttribute('data-json', value.value));
