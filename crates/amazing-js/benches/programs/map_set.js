// Map and Set insertion, lookup and deletion with integer and string keys.
var m = new Map(), st = new Set();
for (var i = 0; i < 3000; i++) { m.set(i, i * 3); st.add("k" + (i % 1000)); }
var sum = 0;
for (var r = 0; r < 2; r++) {
  for (var j = 0; j < 3000; j++) if (m.has(j)) sum = (sum + m.get(j)) % 1000003;
}
for (var k = 0; k < 3000; k += 2) m.delete(k);
var visit = 0;
m.forEach(function (v, key) { visit = (visit + v + key) % 1000003; });
var hits = 0;
for (var q = 0; q < 2000; q++) if (st.has("k" + q)) hits++;
print("map_set " + sum + " " + m.size + " " + st.size + " " + visit + " " + hits);
