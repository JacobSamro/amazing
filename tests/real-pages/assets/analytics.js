var analyticsQueue = analyticsQueue || [];
function track(name, props) {
  var t = (typeof Date.now === 'function') ? Date.now() : 0;
  analyticsQueue.push({ name: name, props: props || {}, t: t });
}
track('pageview', { title: document.title });
