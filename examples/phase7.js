const title = document.querySelector('#title');
title.textContent = 'after';
title.setAttribute('data-state', 'ready');
const badge = document.createElement('span');
badge.textContent = '!';
title.append(badge);
console.log(title.textContent, title.getAttribute('data-state'));
