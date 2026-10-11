// String concatenation, slicing, indexOf and charCodeAt.
var s = "";
for (var i = 0; i < 6000; i++) s += String.fromCharCode(97 + (i * 7) % 26);
var h = 0;
for (var r = 0; r < 10; r++) {
  for (var j = 0; j + 8 <= s.length; j += 3) {
    var piece = s.slice(j, j + 8);
    h = (h * 31 + piece.charCodeAt(0) + piece.charCodeAt(7)) % 1000003;
  }
}
var parts = [];
for (var k = 0; k < 2000; k++) parts.push("item" + k);
var joined = parts.join(",");
var found = 0, at = 0;
while ((at = joined.indexOf("item1", at)) !== -1) { found++; at++; }
print("string_ops " + h + " " + s.length + " " + joined.length + " " + found + " " + joined.split(",").length);
