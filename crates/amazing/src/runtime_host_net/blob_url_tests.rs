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

#[test]
fn file_reader_events_are_real_progress_events_in_order() {
    let mut s = page();
    s.run_script(
        "var out = {log: []}; var r = new FileReader(); var b = new Blob(['h\\u00e9llo'], {type: 'text/plain'});\
         out.proto = r instanceof EventTarget && r instanceof FileReader && ProgressEvent.prototype instanceof Event;\
         ['loadstart','progress','load','loadend'].forEach(function(t){ r.addEventListener(t, function(e){\
           out.log.push(t + ':' + (e instanceof ProgressEvent) + ':' + (e.target === r) + ':' + e.lengthComputable + ':' + r.readyState + ':' + e.eventPhase); }); });\
         r.onloadend = function(e){ out.res = r.result; out.done = true; };\
         out.nullBefore = r.result === null && r.readyState === 0;\
         r.readAsText(b);\
         try { r.readAsText(b); } catch (e) { out.busy = e.name; }",
    )
    .unwrap();
    pump(&mut s, "out.done === true");
    assert_eq!(
        s.run_script(
            "out.proto && out.nullBefore && out.busy === 'InvalidStateError' && out.res === 'h\\u00e9llo' \
             && out.log.join('|') === 'loadstart:true:true:true:1:2|progress:true:true:true:1:2|load:true:true:true:2:2|loadend:true:true:true:2:2'"
        )
        .unwrap(),
        JsValue::Bool(true)
    );
}

#[test]
fn file_reader_abort_and_read_kinds() {
    let mut s = page();
    s.run_script(
        "var out = {log: []}; var r = new FileReader(); var b = new Blob([new Uint8Array([0, 255, 65])]);\
         r.onabort = function(){ out.log.push('abort:' + r.error.name); }; r.onloadend = function(){ out.log.push('end'); };\
         r.onload = function(){ out.log.push('load'); };\
         r.readAsArrayBuffer(b); r.abort(); out.afterAbort = r.readyState + ',' + r.result;\
         var r2 = new FileReader(); r2.onload = function(){ out.bin = r2.result.length + ',' + r2.result.charCodeAt(1);\
           var r3 = new FileReader(); r3.onload = function(){ out.url = r3.result; var r4 = new FileReader();\
             r4.onload = function(){ out.ab = r4.result.byteLength + ':' + (r4.result === r4.result); out.done = true; }; r4.readAsArrayBuffer(b); };\
           r3.readAsDataURL(b); };\
         r2.readAsBinaryString(b);",
    )
    .unwrap();
    pump(&mut s, "out.done === true");
    assert_eq!(
        s.run_script(
            "out.log.join('|') === 'abort:AbortError|end' && out.afterAbort === '2,null' && out.bin === '3,255' \
             && out.url === 'data:application/octet-stream;base64,AP9B' && out.ab === '3:true'"
        )
        .unwrap(),
        JsValue::Bool(true)
    );
}

#[test]
fn xhr_is_an_event_target_with_progress_events() {
    let mut s = page();
    s.run_script(
        "var out = {log: []}; var x = new XMLHttpRequest();\
         out.chain = x instanceof EventTarget && x instanceof XMLHttpRequestEventTarget && x.upload instanceof XMLHttpRequestEventTarget && x.upload === x.upload;\
         out.handlerNull = x.onload === null && x.onreadystatechange === null;\
         x.onreadystatechange = function(e){ out.log.push('rsc' + x.readyState + (e instanceof ProgressEvent ? 'P' : 'E')); };\
         x.addEventListener('load', function(e){ out.log.push('load:' + (e instanceof ProgressEvent) + ':' + (e.target === x) + ':' + e.lengthComputable + ':' + e.loaded + '/' + e.total); });\
         x.onloadend = function(e){ out.log.push('end'); out.done = true; };\
         x.open('GET', URL.createObjectURL(new Blob(['abcd'])));\
         x.send();",
    )
    .unwrap();
    pump(&mut s, "out.done === true");
    assert_eq!(
        s.run_script(
            "out.chain && out.handlerNull && out.log.join('|') === 'rsc1E|rsc2E|rsc3E|rsc4E|load:true:true:true:4/4|end'"
        )
        .unwrap(),
        JsValue::Bool(true)
    );
}
