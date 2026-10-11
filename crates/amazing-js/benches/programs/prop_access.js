// Property reads/writes on objects that all share one stable shape.
function makePoint(i) { return { x: i, y: i * 2, z: i * 3, w: 1 }; }
var pts = [];
for (var i = 0; i < 200; i++) pts.push(makePoint(i));
var sum = 0;
for (var round = 0; round < 80; round++) {
  for (var j = 0; j < pts.length; j++) {
    var p = pts[j];
    p.x = (p.x + p.y) % 1009;
    p.y = (p.y + p.z) % 1013;
    p.z = (p.z + p.w + p.x) % 1019;
    sum = (sum + p.x + p.y + p.z) % 1000003;
  }
}
print("prop_access " + sum);
