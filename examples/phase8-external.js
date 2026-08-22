const title = document.querySelector('#title');
document.addEventListener('DOMContentLoaded', () => title.setAttribute('data-ready', 'yes'));
title.onclick = (event) => { title.textContent = event.type + '!'; title.style.color = 'red'; };
title.addEventListener('click', () => title.setAttribute('data-clicked', 'yes'));
setTimeout(() => title.setAttribute('data-timer', 'yes'), 0);
