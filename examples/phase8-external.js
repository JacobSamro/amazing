const title = document.querySelector('#title');
document.addEventListener('DOMContentLoaded', () => title.setAttribute('data-ready', 'yes'));
document.querySelector('main').addEventListener('click', (event) => title.setAttribute('data-bubbled', event.currentTarget.getAttribute('id') === 'root' ? 'yes' : 'no'));
title.onclick = (event) => { title.textContent = event.type + '!'; title.style.color = 'red'; };
title.addEventListener('click', () => title.setAttribute('data-clicked', 'yes'));
setTimeout(() => title.setAttribute('data-timer', 'yes'), 0);
