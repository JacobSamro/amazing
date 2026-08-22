const title = document.querySelector('#title');
title.onclick = (event) => { title.textContent = event.type + '!'; title.style.color = 'red'; };
title.addEventListener('click', () => title.setAttribute('data-clicked', 'yes'));
