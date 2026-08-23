const title = document.querySelector('#title');
title.textContent = 'after';
title.setAttribute('data-state', 'ready');
title.onclick = (event) => {
  title.textContent = event.type + '!';
  title.style.color = 'red';
};
title.addEventListener('click', () => title.setAttribute('data-clicked', 'yes'));
title.click();
const badge = document.createElement('span');
badge.textContent = '!';
title.append(badge);
console.log(title.textContent, title.getAttribute('data-state'), title.style.color);
