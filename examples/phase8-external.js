const title = document.querySelector('#title');
document.addEventListener('DOMContentLoaded', () => title.setAttribute('data-ready', 'yes'));
document.querySelector('main').addEventListener('click', (event) => title.setAttribute('data-bubbled', event.currentTarget.getAttribute('id') === 'root' ? 'yes' : 'no'));
if (document.querySelectorAll('p').length === 1 && document.getElementById('title') !== null) title.setAttribute('data-dom-api', 'yes');
if (title.innerHTML === 'before') { title.innerHTML = 'before'; title.setAttribute('data-inner', 'yes'); }
title.onclick = (event) => { title.textContent = event.type + '!'; title.style.color = 'red'; };
title.addEventListener('click', () => title.setAttribute('data-clicked', 'yes'));
setTimeout(() => title.setAttribute('data-timer', 'yes'), 0);
