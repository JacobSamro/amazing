// Class instantiation with inheritance, super calls and virtual dispatch.
class Shape {
  constructor(id) { this.id = id; this.scale = 1; }
  area() { return 0; }
  describe() { return this.id * 7 + this.area(); }
}
class Rect extends Shape {
  constructor(id, w, h) { super(id); this.w = w; this.h = h; }
  area() { return this.w * this.h; }
}
class Square extends Rect {
  constructor(id, s) { super(id, s, s); }
  describe() { return super.describe() + 1; }
}
class Circle extends Shape {
  constructor(id, r) { super(id); this.r = r; }
  area() { return 3 * this.r * this.r; }
}
var total = 0;
for (var i = 0; i < 12000; i++) {
  var s;
  switch (i % 3) {
    case 0: s = new Rect(i, i % 11 + 1, i % 7 + 1); break;
    case 1: s = new Square(i, i % 13 + 1); break;
    default: s = new Circle(i, i % 5 + 1);
  }
  total = (total + s.describe()) % 1000003;
}
print("class_inherit " + total);
