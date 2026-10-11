// Array push, indexed reads/writes, and a prefix-sum pass.
var a = [];
for (var i = 0; i < 8000; i++) a.push((i * 7919) % 10007);
var b = new Array(a.length);
var run = 0;
for (var j = 0; j < a.length; j++) { run = (run + a[j]) % 1000003; b[j] = run; }
var s = 0;
for (var r = 0; r < 2; r++) {
  for (var k = 1; k < b.length; k++) { s = (s + b[k] - b[k - 1] + 10007) % 1000003; }
}
var c = a.slice(0, 1000);
c.sort(function (x, y) { return x - y; });
print("array_loop " + s + " " + b[b.length - 1] + " " + c[0] + " " + c[999]);
