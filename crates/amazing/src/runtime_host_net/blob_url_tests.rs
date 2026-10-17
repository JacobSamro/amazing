//! Installed Interpreter/Window blob: URLs through fetch(), XHR and the Blob API.
use super::*;
use crate::js_scheduler::DriverScheduler;

fn page() -> DriverScheduler {
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse("<body></body>"),
        native_forwarded: false,
    }));
    let mut scheduler = DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY).unwrap();
    scheduler.set_location_url_initial("https://example.test/page").unwrap();
    scheduler
}
fn pump(scheduler: &mut DriverScheduler, condition: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while scheduler.run_script(condition).unwrap() != JsValue::Bool(true) {
        scheduler.pump_now(false).unwrap();
        assert!(Instant::now() < deadline, "missed checkpoint {condition}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn create_object_url_fetch_ranges_and_revoke() {
    let mut s = page();
    s.run_script(
        "var out = {}; var blob = new Blob(['hello ', 'world'], {type: 'Text/PLAIN'});\
         var url = URL.createObjectURL(blob);\
         out.shape = url.indexOf('blob:https://example.test/') === 0;\
         fetch(url).then(function(r){ out.status = r.status; out.type = r.headers.get('content-type');\
           out.len = r.headers.get('content-length'); return r.text(); })\
         .then(function(t){ out.text = t; return fetch(url, {headers: {Range: 'bytes=6-'}}); })\
         .then(function(r){ out.rstatus = r.status; out.cr = r.headers.get('content-range'); return r.text(); })\
         .then(function(t){ out.rtext = t; URL.revokeObjectURL(url); return fetch(url); })\
         .then(function(){ out.revoked = 'resolved'; }, function(e){ out.revoked = e instanceof TypeError; })\
         .then(function(){ out.done = true; });",
    )
    .unwrap();
    pump(&mut s, "out.done === true");
    assert_eq!(
        s.run_script(
            "out.shape && out.status === 200 && out.type === 'text/plain' && out.len === '11' \
             && out.text === 'hello world' && out.rstatus === 206 && out.cr === 'bytes 6-10/11' \
             && out.rtext === 'world' && out.revoked === true"
        )
        .unwrap(),
        JsValue::Bool(true)
    );
}

#[test]
fn xhr_reads_a_blob_url_and_blob_readers_resolve() {
    let mut s = page();
    s.run_script(
        "var out = {}; var f = new File([new Uint8Array([104, 105])], 'a/b.txt', {type: 'text/x'});\
         out.name = f.name; var url = URL.createObjectURL(f);\
         var x = new XMLHttpRequest(); x.open('GET', url); x.onload = function(){ out.xhr = x.responseText + x.status; };\
         x.send();\
         f.arrayBuffer().then(function(b){ out.ab = b.byteLength; return f.bytes(); })\
          .then(function(u){ out.bytes = u[0] + ',' + u[1]; return f.slice(-1, undefined, 'A/B').text(); })\
          .then(function(t){ out.slice = t; out.sliceType = f.slice(0, 1, 'A/B').type; out.done = true; });",
    )
    .unwrap();
    pump(&mut s, "out.done === true && out.xhr !== undefined");
    assert_eq!(
        s.run_script(
            "out.name === 'a:b.txt' && out.xhr === 'hi200' && out.ab === 2 && out.bytes === '104,105' \
             && out.slice === 'i' && out.sliceType === 'a/b'"
        )
        .unwrap(),
        JsValue::Bool(true)
    );
}
