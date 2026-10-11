// Regular expression matching, global replace and captures.
var lines = [];
for (var i = 0; i < 400; i++) lines.push("2026-10-" + (10 + i % 20) + " user" + i + "@example" + (i % 7) + ".com GET /path/" + i + "?q=" + (i * 13) + " 200");
var text = lines.join("\n");
var dateRe = /(\d{4})-(\d{2})-(\d{2})/;
var mailRe = /([a-z0-9]+)@([a-z0-9]+)\.com/g;
var count = 0, sum = 0;
for (var r = 0; r < 12; r++) {
  for (var j = 0; j < lines.length; j++) {
    var m = dateRe.exec(lines[j]);
    if (m) sum = (sum + Number(m[3])) % 1000003;
  }
  var mm;
  mailRe.lastIndex = 0;
  while ((mm = mailRe.exec(text)) !== null) { count++; sum = (sum + mm[1].length) % 1000003; }
}
var replaced = text.replace(/\d+/g, "#");
print("regexp " + count + " " + sum + " " + replaced.length + " " + /GET \/path\/399/.test(text));
