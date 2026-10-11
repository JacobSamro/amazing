// Plain recursion: exponential (fib) and linear-depth recursion.
function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
function depth(n) { return n === 0 ? 0 : 1 + depth(n - 1); }
var d = 0;
for (var i = 0; i < 300; i++) d += depth(150);
print("fib " + fib(23) + " " + d);
