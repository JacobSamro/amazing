if (typeof IntersectionObserver === 'function') {
  var observer = new IntersectionObserver(function (entries) {
    entries.forEach(function (entry) {
      if (entry.isIntersecting) { entry.target.classList.add('visible'); }
    });
  });
  document.querySelectorAll('.lazy').forEach(function (node) { observer.observe(node); });
} else {
  document.querySelectorAll('.lazy').forEach(function (node) { node.classList.add('visible'); });
}
