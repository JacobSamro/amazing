// Closure creation and invocation, counters captured by inner functions.
function makeCounter(step) {
  var n = 0;
  return { next: function () { n += step; return n; }, get: function () { return n; } };
}
function compose(f, g) { return function (x) { return f(g(x)); }; }
var counters = [];
for (var i = 1; i <= 50; i++) counters.push(makeCounter(i));
var total = 0;
for (var r = 0; r < 300; r++) {
  for (var k = 0; k < counters.length; k++) total = (total + counters[k].next()) % 1000003;
}
var add3 = function (x) { return x + 3; };
var mul2 = function (x) { return (x * 2) % 10007; };
var h = compose(add3, mul2);
for (var m = 0; m < 8000; m++) {
  var h2 = compose(h, add3);
  total = (total + h2(m)) % 1000003;
}
print("closures " + total + " " + counters[49].get());
