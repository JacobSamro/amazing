#!/usr/bin/env bash
set -euo pipefail
cargo build -q -p amazing-net --example wpt_shim --locked
cargo build -q -p amazing --locked
root=$(mktemp -d)
pipe=$(mktemp -u)
mkfifo "$pipe"
exec 3<>"$pipe"
"$CARGO_TARGET_DIR/debug/examples/wpt_shim" <&3 >"$root/shim-url" &
shim_pid=$!
trap 'printf "\n" >&3; wait "$shim_pid" 2>/dev/null || true; exec 3>&-; rm -rf "$root" "$pipe"' EXIT
for i in {1..100}; do
  if test -s "$root/shim-url"; then break; fi
  sleep 0.05
done
base=$(cat "$root/shim-url")
test -n "$base"
cp crates/amazing/src/cmd/wpt_harness_fixtures/scaffold-harness.js "$root/scaffold-harness.js"
cat > "$root/xhr_http.html" <<HTML
<!doctype html>
<script src="scaffold-harness.js"></script>
<script>
var t = async_test("XHR async network state and events");
var xhr = new XMLHttpRequest();
var events = [];
xhr.addEventListener("readystatechange", function(e) { events.push("r" + xhr.readyState); });
xhr.addEventListener("loadstart", function(e) { events.push("start"); });
xhr.addEventListener("load", function(e) {
  events.push("load");
  assert_equals(xhr.status, 200);
  assert_equals(xhr.responseText, "top");
  assert_equals(events.join(","), "r1,start,r2,r3,r3,r4,load");
  t.done();
  done();
});
xhr.addEventListener("error", function(e) { throw new Error("XHR network error"); });
xhr.open("GET", "$base/fetch/api/resources/cors-top.txt");
xhr.send();
assert_equals(xhr.readyState, 1);
</script>
HTML
"$CARGO_TARGET_DIR/debug/amazing" wpt-harness --scaffold "$root" xhr_http.html 800 600 5000
cat > "$root/xhr_upload.html" <<'HTML'
<!doctype html>
<script src="scaffold-harness.js"></script>
<script>
var t = async_test("XHR upload listeners and same-origin WPT host routing");
var xhr = new XMLHttpRequest();
var upload = [];
xhr.open("POST", "http://web-platform.test/fetch/api/resources/echo-content.py");
xhr.upload.addEventListener("loadstart", function() { upload.push("start"); });
xhr.upload.addEventListener("load", function() { upload.push("load"); });
xhr.addEventListener("load", function() {
  assert_equals(xhr.status, 200);
  assert_equals(upload.join(","), "start,load");
  t.done();
  done();
});
xhr.send("hello");
</script>
HTML
AMAZING_WPT_SHIM_ADDR="${base#http://}" \
  "$CARGO_TARGET_DIR/debug/amazing" wpt-harness --scaffold "$root" xhr_upload.html 800 600 5000
cat > "$root/fetch_json.html" <<'HTML'
<!doctype html>
<script src="scaffold-harness.js"></script>
<script>
var t = async_test("Fetch JSON from the selected WPT resource shim");
fetch("http://web-platform.test/fetch/api/resources/data.json")
  .then(function(response) {
    assert_equals(response.status, 200);
    return response.json();
  })
  .then(function(value) {
    assert_equals(value.key, "value");
    t.done();
    done();
  });
</script>
HTML
AMAZING_WPT_SHIM_ADDR="${base#http://}" \
  "$CARGO_TARGET_DIR/debug/amazing" wpt-harness --scaffold "$root" fetch_json.html 800 600 5000
cat > "$root/fetch_abort_body.html" <<'HTML'
<!doctype html>
<script src="scaffold-harness.js"></script>
<script>
var t = async_test("Response body rejects after its Fetch signal aborts");
var controller = new AbortController();
fetch("http://web-platform.test/fetch/api/resources/data.json", { signal: controller.signal })
  .then(function(response) {
    controller.abort();
    return response.text();
  })
  .then(function() { throw new Error("body unexpectedly resolved"); }, function(error) {
    assert_equals(error.name, "AbortError");
    t.done();
    done();
  });
</script>
HTML
AMAZING_WPT_SHIM_ADDR="${base#http://}" \
  "$CARGO_TARGET_DIR/debug/amazing" wpt-harness --scaffold "$root" fetch_abort_body.html 800 600 5000
