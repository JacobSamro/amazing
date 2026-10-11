// Numeric loops over typed arrays: sieve, dot products and an in-place smooth.
var N = 20000;
var sieve = new Uint8Array(N + 1);
var primes = 0;
for (var i = 2; i <= N; i++) {
  if (sieve[i] === 0) { primes++; for (var j = i * 2; j <= N; j += i) sieve[j] = 1; }
}
var f = new Float64Array(2048), g = new Int32Array(2048);
for (var k = 0; k < 2048; k++) { f[k] = (k % 17) * 0.5; g[k] = (k * 31) % 101; }
var dot = 0;
for (var r = 0; r < 8; r++) for (var t = 0; t < 2048; t++) dot += f[t] * g[t];
for (var p = 0; p < 4; p++) for (var u = 1; u < 2047; u++) f[u] = (f[u - 1] + f[u] + f[u + 1]) / 3;
print("typed_array " + primes + " " + dot + " " + Math.round(f[1000] * 1e6));
