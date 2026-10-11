// Method calls through a prototype-free object and a constructor prototype.
function Acc() { this.total = 0; this.count = 0; }
Acc.prototype.add = function (v) { this.total = (this.total + v) % 1000003; this.count++; return this; };
Acc.prototype.mean = function () { return this.count ? Math.floor(this.total / this.count) : 0; };
var ops = {
  inc: function (v) { return v + 1; },
  dbl: function (v) { return (v * 2) % 65521; },
  sub: function (v) { return (v + 65521 - 3) % 65521; }
};
var acc = new Acc();
var v = 1;
for (var i = 0; i < 24000; i++) {
  v = ops.inc(v);
  v = ops.dbl(v);
  v = ops.sub(v);
  acc.add(v);
}
print("method_calls " + acc.total + " " + acc.count + " " + acc.mean());
