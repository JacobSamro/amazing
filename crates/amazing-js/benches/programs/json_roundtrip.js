// JSON.stringify / JSON.parse round trips of a nested document.
var doc = { id: 1, name: "widget", tags: ["a", "b", "c"], dims: { w: 10, h: 20.5, d: -3 }, items: [] };
for (var i = 0; i < 40; i++) doc.items.push({ n: i, label: "item-" + i, ok: i % 2 === 0, v: [i, i * 2, i * 3] });
var text = JSON.stringify(doc);
var check = 0;
for (var r = 0; r < 360; r++) {
  var back = JSON.parse(text);
  back.id = back.id + 1;
  text = JSON.stringify(back);
  check = (check + text.length + back.items[r % 40].v[2]) % 1000003;
}
print("json_roundtrip " + check + " " + text.length + " " + JSON.parse(text).id);
