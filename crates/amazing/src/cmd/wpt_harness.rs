// DOM-1 experimental classic-script runner. Assertions belong to upstream WPT.
// --scaffold is a separate, explicitly non-conformance plumbing diagnostic.
use amazing_js::Interpreter;
use amazing_js::value::{FunctionBody, JsFunction, JsObject, JsValue, NativeFn, Obj, Throw};
use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const REVISION: &str = "c2a2fce69e5c2509d9dc28440ec8b42532e5c293";
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
const FIXTURE_LIMIT: usize = 16 * 1024 * 1024;
thread_local! { static COMPLETE: Cell<bool> = const { Cell::new(false) }; }

/// IA-TESTDRIVER: `testdriver-vendor.js` is a per-implementation resource --
/// every real browser ships its own, so it does not exist anywhere in the
/// pinned upstream WPT checkout at all. Before this ticket, a test that
/// loaded it (`<script src="/resources/testdriver-vendor.js">`, the standard
/// WPT boilerplate right after `testdriver.js`) failed `local_file`'s
/// `canonicalize()` and aborted the *entire* file -- every subtest lost as
/// `error`, not just the ones that actually call a `test_driver` action at
/// runtime. This is served in its place (see `execute_page`'s script loop
/// below), the same way that loop already substitutes its own reporter for
/// `resources/testharnessreport.js`.
///
/// It runs *after* `testdriver.js` in script order (real WPT test files load
/// them in that order), which matters: `testdriver.js` unconditionally
/// assigns a fresh, every-method-throws `window.test_driver_internal` object
/// near its own end, so this script's job is to override individual methods
/// on that *already-created* object, not to create it.
///
/// Native backends live in `runtime_host_integrator_assist.rs`
/// (`IaNativeEntry::Td*`, installed as the global `__amazing_td_*`
/// functions this script calls). See that file's module-level and per-method
/// doc comments for what each one actually does and its known gaps
/// (no per-key `KeyboardEvent` extras; approximate, author-CSS-free
/// `getClientRects`/`elementsFromPoint` geometry; no cookie store or
/// Permissions API to back `delete_all_cookies`/`set_permission` with real
/// effects).
const TESTDRIVER_VENDOR_JS: &str = r#"(function() {
    "use strict";
    function promised(fn) {
        return function() {
            var self = this, args = arguments;
            return new Promise(function(resolve, reject) {
                try { resolve(fn.apply(self, args)); } catch (e) { reject(e); }
            });
        };
    }
    // testdriver.js's own click()/send_keys() wrappers call these directly
    // (via `getPointerInteractablePaintTree`/`inView`) before ever reaching
    // `test_driver_internal`; only defined when missing, so a real
    // implementation from `layout`/`dom-bindings` automatically wins later.
    if (typeof Element !== "undefined" && !Element.prototype.getClientRects) {
        Element.prototype.getClientRects = function() { return __amazing_td_get_client_rects(this); };
    }
    if (typeof Element !== "undefined" && !Element.prototype.getBoundingClientRect) {
        Element.prototype.getBoundingClientRect = function() {
            var rects = __amazing_td_get_client_rects(this);
            return rects[0] || {x: 0, y: 0, width: 0, height: 0, top: 0, left: 0, right: 0, bottom: 0};
        };
    }
    if (typeof Document !== "undefined" && !Document.prototype.elementsFromPoint) {
        Document.prototype.elementsFromPoint = function(x, y) { return __amazing_td_elements_from_point(x, y); };
    }
    if (typeof Document !== "undefined" && !Document.prototype.elementFromPoint) {
        Document.prototype.elementFromPoint = function(x, y) {
            var hits = __amazing_td_elements_from_point(x, y);
            return hits.length ? hits[0] : null;
        };
    }
    if (typeof window.test_driver_internal === "undefined") { return; }
    window.test_driver_internal.in_automation = true;
    window.test_driver_internal.click = promised(function(element) {
        __amazing_td_click(element);
    });
    window.test_driver_internal.send_keys = promised(function(element, keys) {
        __amazing_td_send_keys(element, keys);
    });
    window.test_driver_internal.action_sequence = promised(function(actions, context) {
        __amazing_td_action_sequence(actions, context);
    });
    window.test_driver_internal.set_permission = promised(function(params, context) {
        __amazing_td_set_permission(params, context);
    });
    window.test_driver_internal.delete_all_cookies = promised(function(context) {
        __amazing_td_delete_all_cookies(context);
    });
    window.test_driver_internal.get_computed_label = promised(function(element) {
        return __amazing_td_get_computed_label(element);
    });
    window.test_driver_internal.get_computed_role = promised(function(element) {
        return __amazing_td_get_computed_role(element);
    });
})();
"#;
/// Root-relative path every real WPT test loads this resource from
/// (`/resources/testdriver-vendor.js`), normalized the same way `execute_page`
/// normalizes a script's own `src` before comparing.
const TESTDRIVER_VENDOR_PATH: &str = "resources/testdriver-vendor.js";

struct FixtureServer {
    origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl FixtureServer {
    fn start(root: PathBuf, generated: Option<(String, Vec<u8>)>) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let origin = format!(
            "http://{}",
            listener.local_addr().map_err(|error| error.to_string())?
        );
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            let mut connections: Vec<std::thread::JoinHandle<()>> = Vec::new();
            while !worker_stop.load(Ordering::Acquire) {
                let mut index = 0;
                while index < connections.len() {
                    if connections[index].is_finished() {
                        let finished = connections.swap_remove(index);
                        let _ = finished.join();
                    } else {
                        index += 1;
                    }
                }
                match listener.accept() {
                    Ok((stream, _)) if connections.len() < 64 => {
                        let root = root.clone();
                        let generated = generated.clone();
                        connections.push(std::thread::spawn(move || {
                            serve_fixture(stream, &root, generated.as_ref());
                        }));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
            for connection in connections {
                let _ = connection.join();
            }
        });
        Ok(Self {
            origin,
            stop,
            worker: Some(worker),
        })
    }

    fn url(&self, relative: &str) -> String {
        format!("{}/{}", self.origin, relative.trim_start_matches('/'))
    }

    /// A fixture handle that serves nothing itself. The pinned fetch/api and
    /// xhr tests run against the std-only WPT resource shim selected through
    /// `AMAZING_WPT_SHIM_ADDR`, which the transport routes by WPT host, so the
    /// document URL must stay on `web-platform.test`. Nested navigation still
    /// needs an origin to strip, which is all this handle provides.
    fn origin_only(origin: &str) -> Self {
        Self {
            origin: origin.to_string(),
            stop: Arc::new(AtomicBool::new(true)),
            worker: None,
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve_fixture(mut stream: TcpStream, root: &Path, generated: Option<&(String, Vec<u8>)>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
        let mut byte = [0];
        if stream.read_exact(&mut byte).is_err() {
            return;
        }
        request.push(byte[0]);
    }
    let path = String::from_utf8_lossy(&request)
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|line| line.split_once(' '))
        .map(|(path, _)| path.to_owned());
    let mut served_name = String::new();
    let body = path.as_deref().and_then(|path| {
        let name = path.split(['?', '#']).next()?.strip_prefix('/')?;
        if name.contains('%') || name.contains('\\') || name.split('/').any(|part| part == "..") {
            return None;
        }
        served_name = name.to_owned();
        if let Some((generated_name, bytes)) = generated
            && name == generated_name.trim_start_matches('/')
        {
            return Some(bytes.clone());
        }
        let file = root.join(name).canonicalize().ok()?;
        if !file.starts_with(root) || file.metadata().ok()?.len() > FIXTURE_LIMIT as u64 {
            return None;
        }
        std::fs::read(file).ok()
    });
    let (status, bytes) = match body {
        Some(bytes) if bytes.len() <= FIXTURE_LIMIT => ("200 OK", bytes),
        _ => ("404 Not Found", Vec::new()),
    };
    // A real WPT server sends a `Content-Type` header derived from the
    // resource's extension; this fixture previously sent none at all, so
    // `content_type()` (`runtime_host_html.rs`) had nothing to sniff and its
    // own extension fallback never runs for a fetch through this server
    // (that fallback only triggers for a resolver-backed or `file:` fetch,
    // neither of which this loopback HTTP fetch is). That silently failed
    // every navigation to a non-HTML child document (XML/XHTML iframes:
    // "iframe response has no supported media type"), which is what made
    // `iframe.contentWindow.document`'s elements look broken (e.g.
    // `localName` never actually ran on a real element) -- the child
    // document simply never loaded. Mirrors the WPT server's own
    // mime-type mapping closely enough for what this corpus actually needs.
    let content_type = match Path::new(&served_name).extension().and_then(|ext| ext.to_str()) {
        Some("html" | "htm") => "text/html",
        Some("xhtml" | "xht") => "application/xhtml+xml",
        Some("xml") => "application/xml",
        Some("svg") => "image/svg+xml",
        Some("js" | "mjs") => "text/javascript",
        Some("json") => "application/json",
        Some("css") => "text/css",
        Some("txt") => "text/plain",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(&bytes);
}

fn quote(value: &str) -> String {
    let mut s = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if c < ' ' => s.push_str(&format!("\\u{:04x}", c as u32)),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

fn quote_utf16(value: &[u16]) -> String {
    let mut s = String::from("\"");
    for decoded in std::char::decode_utf16(value.iter().copied()) {
        let c = match decoded {
            Ok(c) => c,
            Err(unpaired) => {
                s.push_str(&format!("\\u{:04x}", unpaired.unpaired_surrogate()));
                continue;
            }
        };
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if c < ' ' => s.push_str(&format!("\\u{:04x}", c as u32)),
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

struct Options {
    scaffold: bool,
    root: PathBuf,
    test: String,
    width: u32,
    height: u32,
    timeout: u32,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let scaffold = args.first().is_some_and(|s| s == "--scaffold");
        let a = if scaffold { &args[1..] } else { args };
        if a.len() != 5 {
            return Err("usage: amazing wpt-harness [--scaffold] <root> <relative-test.html> <width> <height> <timeout-ms>".into());
        }
        let number = |i: usize| {
            a[i].parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| format!("expected positive integer, got {}", a[i]))
        };
        Ok(Self {
            scaffold,
            root: PathBuf::from(&a[0]),
            test: a[1].clone(),
            width: number(2)?,
            height: number(3)?,
            timeout: number(4)?,
        })
    }
}

#[derive(Default)]
struct Report {
    subtests: Vec<String>,
    counts: [usize; 5], // pass, fail, timeout, notrun, error
    harness: Option<i32>,
    detail: Option<String>, // JSON string
    error: bool,
    unsupported: bool,
    timed_out: bool,
    verified: bool,
}
impl Report {
    fn accept(&mut self, line: &str) {
        let fields: Vec<_> = line.splitn(3, '\t').collect();
        match fields.as_slice() {
            ["subtest", code, json] => {
                let code = code.parse::<i32>().unwrap_or(-1);
                let category = match code {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    3 => 3,
                    _ => 4,
                };
                self.counts[category] += 1;
                self.subtests.push((*json).into());
            }
            ["harness", code, detail] => {
                if self.harness.is_some() {
                    self.error = true;
                }
                self.harness = code.parse().ok();
                self.detail = Some((*detail).into());
            }
            ["error", detail] => {
                self.error = true;
                self.detail = Some((*detail).into());
            }
            ["unsupported", detail] => {
                self.unsupported = true;
                self.detail = Some((*detail).into());
            }
            _ => {
                self.error = true;
                self.detail = Some(quote("invalid worker protocol"));
            }
        }
    }
    fn status(&self) -> &'static str {
        if self.error {
            "error"
        } else if self.unsupported {
            "unsupported"
        } else if self.timed_out || self.harness == Some(2) {
            "timeout"
        } else if self.harness != Some(0) {
            "error"
        } else if self.counts[1..].iter().any(|n| *n != 0) {
            "fail"
        } else {
            "pass"
        }
    }
    fn json(&self, options: &Options) -> String {
        let status = self.status();
        format!(
            "{{\"schema\":1,\"suite\":{},\"wpt_revision\":{},\"test\":{},\"viewport\":[{},{}],\"timeout_ms\":{},\"status\":{},\"harness_status\":{},\"detail\":{},\"file_counts\":{{\"pass\":{},\"fail\":{},\"timeout\":{},\"error\":{},\"unsupported\":{}}},\"counts\":{{\"pass\":{},\"fail\":{},\"timeout\":{},\"notrun\":{},\"error\":{}}},\"subtests\":[{}]}}",
            quote(if options.scaffold {
                "scaffolding-only"
            } else {
                "wpt"
            }),
            if self.verified && !options.scaffold {
                quote(REVISION)
            } else {
                "null".into()
            },
            quote(&options.test),
            options.width,
            options.height,
            options.timeout,
            quote(status),
            self.harness.map_or("null".into(), |n| n.to_string()),
            self.detail.as_deref().unwrap_or("null"),
            usize::from(status == "pass"),
            usize::from(status == "fail"),
            usize::from(status == "timeout"),
            usize::from(status == "error"),
            usize::from(status == "unsupported"),
            self.counts[0],
            self.counts[1],
            self.counts[2],
            self.counts[3],
            self.counts[4],
            self.subtests.join(",")
        )
    }
}

fn emit(line: &str) -> Result<(), Throw> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}")
        .and_then(|_| out.flush())
        .map_err(|_| Throw::type_error("runner output closed"))
}
fn property(value: Option<&JsValue>, name: &str) -> JsValue {
    match value {
        Some(JsValue::Object(o)) => o.borrow().get_own(name).unwrap_or(JsValue::Undefined),
        _ => JsValue::Undefined,
    }
}
fn status_code(value: JsValue) -> i32 {
    match value {
        JsValue::Num(n) if n.is_finite() && n.fract() == 0.0 => n as i32,
        _ => -1,
    }
}
fn string_value(value: JsValue) -> String {
    match value {
        JsValue::Str(s) => quote_utf16(s.units()),
        _ => "null".into(),
    }
}
fn result_callback(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let status = status_code(property(args.first(), "status"));
    let name = string_value(property(args.first(), "name"));
    let message = string_value(property(args.first(), "message"));
    emit(&format!(
        "subtest\t{status}\t{{\"name\":{name},\"status\":{status},\"message\":{message}}}"
    ))?;
    Ok(JsValue::Undefined)
}
fn completion_callback(
    _: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let status = status_code(property(args.get(1), "status"));
    let message = string_value(property(args.get(1), "message"));
    emit(&format!("harness\t{status}\t{message}"))?;
    COMPLETE.set(true);
    Ok(JsValue::Undefined)
}
fn install_callback(js: &mut Interpreter, name: &str, callback: NativeFn) -> Result<(), String> {
    let mut object = JsObject::function(JsFunction {
        name: name.into(),
        params: Vec::new(),
        body: FunctionBody::Native(callback),
        is_arrow: false,
        is_async: false,
        is_generator: false,
        env: js.global_env.clone(),
        home: None,
        proto_for_new: None,
    });
    object.proto = Some(js.function_proto.clone());
    let value = JsValue::Object(Obj(Rc::new(RefCell::new(object))));
    js.global_obj.borrow_mut().set_own(name, value.clone());
    js.global_env
        .declare_var(name, value)
        .map_err(|e| e.describe())
}

fn local_file(root: &Path, base: &Path, source: &str) -> Result<PathBuf, String> {
    if source.contains(['?', '#', ':', '%', '\\']) {
        return Err(format!("unsupported resource URL: {source}"));
    }
    let file = if let Some(relative) = source.strip_prefix('/') {
        root.join(relative)
    } else {
        base.join(source)
    };
    let file = file
        .canonicalize()
        .map_err(|e| format!("resource {source}: {e}"))?;
    if !file.starts_with(root) {
        return Err("resource escapes root".into());
    }
    Ok(file)
}
fn source(root: &Path, path: &Path, scaffold: bool) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    // A marker-pinned (non-git, venus tarball) checkout already proved its
    // provenance once in `supervise`; there is no git object database to
    // `git show` a comparison from, and re-extracting the same tarball
    // per resource would defeat the point of pinning it once. Skip the
    // per-resource byte check there, same as `supervise`'s own checkout
    // check.
    if !scaffold && !checkout_is_marker_pinned(root) {
        // Verify every executed HTML/JS resource, not just checkout HEAD.
        let relative = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let original = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["show", &format!("{REVISION}:{relative}")])
            .output()
            .map_err(|e| e.to_string())?;
        if !original.status.success() || original.stdout != bytes {
            return Err(format!(
                "resource differs from pinned upstream bytes: {relative}"
            ));
        }
    }
    let mut text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    if !scaffold && path.file_name().is_some_and(|name|
        name.to_string_lossy().contains(".sub.")) {
        for (placeholder, value) in [
            ("{{host}}", "web-platform.test"),
            ("{{ports[http][0]}}", "80"),
            ("{{ports[http][1]}}", "8001"),
            ("{{ports[https][0]}}", "443"),
            ("{{ports[https][1]}}", "8443"),
            ("{{domains[www2]}}", "www2.web-platform.test"),
            ("{{domains[www1]}}", "www1.web-platform.test"),
            ("{{domains[www]}}", "www.web-platform.test"),
            ("{{hosts[][www]}}", "www.web-platform.test"),
            ("{{hosts[alt][]}}", "not-web-platform.test"),
            ("{{hosts[alt][www]}}", "www.not-web-platform.test"),
            ("{{hosts[alt][www2]}}", "www2.not-web-platform.test"),
            // NET-38: websockets/constants.sub.js builds its ws:// / wss://
            // base URL from these; any value routes through StdResolver's
            // *.web-platform.test host match regardless of the port number.
            ("{{ports[ws][0]}}", "80"),
            ("{{ports[wss][0]}}", "443"),
            ("{{ports[h2][0]}}", "443"),
            ("{{ports[http-local][0]}}", "80"),
            ("{{ports[http-public][0]}}", "80"),
            ("{{ports[https-local][0]}}", "443"),
            ("{{ports[https-public][0]}}", "443"),
        ] {
            text = text.replace(placeholder, value);
        }
    }
    Ok(text)
}
/// Window variant of WPT AnyHtmlHandler at revision c2a2fce69e5c2509d9dc28440ec8b42532e5c293.
/// Template and metadata rules: tools/serve/serve.py AnyHtmlHandler and
/// tools/manifest/sourcefile.py read_script_metadata/parse_variants.
fn any_window_wrapper(js: &str, test: &str) -> Result<String, String> {
    let mut metadata = Vec::new();
    for line in js.lines() {
        let Some(rest) = line
            .strip_prefix("//")
            .and_then(|s| s.trim_start().strip_prefix("META:"))
        else {
            break;
        };
        let Some((key, value)) = rest.trim_start().split_once('=') else {
            break;
        };
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            break;
        }
        metadata.push((key, value));
    }
    if let Some((_, globals)) = metadata.iter().find(|(key, _)| *key == "global") {
        if !globals.split(',').any(|part| part.trim() == "window") {
            return Err("unsupported: .any.js source excludes window global".into());
        }
    }
    let text_escape = |value: &str| value.replace('&', "&amp;").replace('<', "&lt;");
    let attr_escape = |value: &str| value.replace('&', "&amp;").replace('"', "&quot;");
    let mut meta = String::new();
    let mut scripts = String::new();
    for (key, value) in metadata {
        match key {
            "timeout" if value == "long" => {
                meta.push_str("<meta name=\"timeout\" content=\"long\">\n")
            }
            "title" => meta.push_str(&format!("<title>{}</title>\n", text_escape(value))),
            "script" => scripts.push_str(&format!(
                "<script src=\"{}\"></script>\n",
                attr_escape(value)
            )),
            _ => {}
        }
    }
    let path = format!("/{}", test.trim_start_matches('/'));
    Ok(format!(
        "<!doctype html>\n<meta charset=utf-8>\n{meta}<script>\nself.GLOBAL = {{\n  isWindow: function() {{ return true; }},\n  isWorker: function() {{ return false; }},\n  isShadowRealm: function() {{ return false; }},\n}};\n</script>\n<script src=\"/resources/testharness.js\"></script>\n<script src=\"/resources/testharnessreport.js\"></script>\n{scripts}<div id=log></div>\n<script src=\"{}\"></script>\n",
        attr_escape(&path)
    ))
}
fn collect_scripts(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    scripts: &mut Vec<crate::PageScript>,
) -> Result<(), String> {
    if dom.name(node) == Some("script") {
        let attrs = dom.attrs_of(node).unwrap_or(&[]);
        let attr = |name: &str| {
            attrs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        if attr("type") == Some("module") || attr("async").is_some() || attr("defer").is_some() {
            return Err("unsupported: module/async/defer script scheduling".into());
        }
        if attr("type")
            .is_some_and(|v| !["", "text/javascript", "application/javascript"].contains(&v))
        {
            return Ok(());
        }
        crate::collect_script_text(dom, node, scripts);
    } else {
        for child in dom.children(node) {
            collect_scripts(dom, child, scripts)?;
        }
    }
    Ok(())
}

fn resolve_page_url(base: &str, value: &str) -> Result<String, String> {
    let base = amazing_net::Url::parse(base).map_err(|error| error.to_string())?;
    amazing_net::resolve(value, &base)
        .map(|url| url.to_string())
        .map_err(|error| error.to_string())
}

fn fetch_page(
    navigator: &mut crate::navigator::Navigator,
    url: &str,
    record: bool,
    target: Option<usize>,
) -> Result<crate::navigator::NavigationResult<Vec<u8>>, String> {
    let use_process = crate::runtime_host_ipc::enabled();
    navigator.navigate(url, record, target, 0, move |url, process_body| {
        if use_process {
            Ok(process_body.to_vec())
        } else {
            let response = amazing_net::fetch(url, &amazing_net::FetchOptions::default())
                .map_err(|error| format!("fixture navigation: {error}"))?;
            Ok(response.body)
        }
    })
}

fn fixture_file(root: &Path, fixture: &FixtureServer, url: &str) -> Result<PathBuf, String> {
    let relative = url
        .strip_prefix(&fixture.origin)
        .and_then(|path| path.strip_prefix('/'))
        .ok_or_else(|| format!("unsupported: navigation left fixture origin: {url}"))?;
    local_file(
        root,
        root,
        relative.split(['?', '#']).next().unwrap_or(relative),
    )
}

fn execute(options: &Options) -> Result<(), String> {
    COMPLETE.set(false);
    let root = options.root.canonicalize().map_err(|e| e.to_string())?;
    let file = local_file(&root, &root, &options.test)?;
    let expected_html = match file.extension().and_then(|s| s.to_str()) {
        Some("html" | "htm") => source(&root, &file, options.scaffold)?,
        Some("js") if !options.scaffold && (options.test.ends_with(".any.js") || options.test.ends_with(".window.js")) => {
            let js = source(&root, &file, false)?;
            any_window_wrapper(&js, &options.test)?
        }
        _ => return Err("unsupported: classic-script .html/.htm or window .any.js/.window.js tests only".into()),
    };
    let mut navigator = crate::navigator::Navigator::new(1);
    let shim_selected = !options.scaffold && std::env::var_os("AMAZING_WPT_SHIM_ADDR").is_some();
    let (fixture, url, history_state) = if shim_selected {
        // Pinned fetch/api and xhr resource requests must resolve to the
        // `web-platform.test` origins the std-only shim answers, not to the
        // loopback fixture server, so the document URL stays on that host.
        (
            FixtureServer::origin_only("http://web-platform.test"),
            format!("http://web-platform.test/{}", options.test),
            None,
        )
    } else {
        let generated =
            (options.test.ends_with(".any.js") || options.test.ends_with(".window.js"))
                .then(|| (options.test.clone(), expected_html.as_bytes().to_vec()));
        let fixture = FixtureServer::start(root.clone(), generated)?;
        let url = fixture.url(&options.test);
        let page = fetch_page(&mut navigator, &url, true, None)?;
        if page.rendered != expected_html.as_bytes() {
            return Err("fixture navigation differed from pinned page bytes".into());
        }
        (fixture, page.url, page.history_state)
    };
    // NET-40: `.https.` tests (e.g. `websockets/mixed-content.https.any.js`)
    // assert on `location.protocol` and mixed-content restrictions
    // (`client_restricts_mixed_content` in `runtime_host_net.rs`, NET-39)
    // that only make sense for a secure top-level document. This fixture has
    // no TLS listener, so only the DOCUMENT URL handed to the scheduler is
    // rewritten to `https:`; the fetch that already happened above (and any
    // nested navigation, which strips against `fixture.origin`, left
    // unchanged) stays on the real loopback/shim `http://` socket.
    let url = if options.test.contains(".https.") {
        url.replacen("http://", "https://", 1)
    } else {
        url
    };
    execute_page(
        options,
        &root,
        file,
        expected_html,
        url,
        &fixture,
        &mut navigator,
        history_state,
        0,
    )
}

fn execute_page(
    options: &Options,
    root: &Path,
    file: PathBuf,
    html: String,
    url: String,
    fixture: &FixtureServer,
    navigator: &mut crate::navigator::Navigator,
    history_state: Option<amazing_js::SerializedValue>,
    depth: usize,
) -> Result<(), String> {
    if depth >= 32 {
        return Err("unsupported: navigation depth limit".into());
    }
    let host = Rc::new(RefCell::new(crate::BrowserDomHost {
        dom: amazing_html::parse(&html),
        native_forwarded: false,
    }));
    let mut scripts = Vec::new();
    collect_scripts(&host.borrow().dom, amazing_dom::NodeIdx(0), &mut scripts)?;
    let mut scheduler =
        crate::js_scheduler::DriverScheduler::new(host.clone(), crate::js_scheduler::JS_CAPACITY)
            .map_err(|error| error.describe())?;
    scheduler
        .js
        .set_viewport_size(options.width, options.height);
    scheduler.set_location_url(&url);
    if let Some(state) = history_state {
        let value = scheduler
            .js
            .structured_deserialize(&state)
            .map_err(|e| e.describe())?;
        scheduler.js.set_history_state(value);
    }
    // This runner reads a UTF-8 HTML source directly as its top-level page.
    scheduler.set_document_metadata("UTF-8", "text/html", "");
    let refresh_images = |scheduler: &mut crate::js_scheduler::DriverScheduler| -> Result<(), String> {
        let page = host.borrow();
        let styles = amazing_css::compute(&page.dom, &[]);
        let media = crate::page::load_page_media_in_viewport(&page.dom,
            &file.to_string_lossy(), &styles, options.width as f32, 1.0);
        scheduler.set_image_loader_states(&media).map_err(|error| error.describe())
    };
    refresh_images(&mut scheduler)?;
    // The engine's browser-global identity also supplies the standard self alias.
    let global = JsValue::Object(scheduler.js.global_obj.clone());
    scheduler
        .js
        .global_obj
        .borrow_mut()
        .set_own("self", global.clone());
    scheduler
        .js
        .global_env
        .declare_var("self", global)
        .map_err(|e| e.describe())?;
    // Native DOM listener methods currently require an explicit receiver.
    // Global event functions are Window operations even when called bare.
    scheduler.run_script("var addEventListener = window.addEventListener.bind(window); var removeEventListener = window.removeEventListener.bind(window); window.parent = window; window.top = window;")
        .map_err(|e| format!("window host initialization: {}", e.describe()))?;
    install_callback(&mut scheduler.js, "__amazing_result", result_callback)?;
    install_callback(&mut scheduler.js, "__amazing_complete", completion_callback)?;
    let mut handlers = Vec::new();
    crate::collect_inline_handlers(&host.borrow().dom, amazing_dom::NodeIdx(0), &mut handlers);
    for (node, event, body) in handlers {
        scheduler
            .install_inline_handler(node, &event, &body)
            .map_err(|e| e.describe())?;
    }
    let harness_path = root
        .join(if options.scaffold {
            "scaffold-harness.js"
        } else {
            "resources/testharness.js"
        })
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut installed = false;
    // HTML-29: opens the top-level document's `document.write` parser-
    // active window for the duration of its own top-level script phase,
    // narrowed to right after each script as it runs, and closed once its
    // load event has fired below -- matching the window during which a
    // real parser has an insertion point at all.
    scheduler.begin_document_parsing(0);
    for script in scripts {
        scheduler.narrow_write_insertion_point(0, script.node.0);
        let (text, label, harness) = match script.src {
            Some(src) => {
                let normalized = src.trim_start_matches('/').replace('\\', "/");
                if !options.scaffold && normalized == TESTDRIVER_VENDOR_PATH {
                    // IA-TESTDRIVER: does not exist in the pinned upstream
                    // checkout at all -- see `TESTDRIVER_VENDOR_JS`'s doc
                    // comment -- so it never reaches `local_file`, which
                    // would otherwise fail to `canonicalize` it and abort
                    // this whole test file.
                    (TESTDRIVER_VENDOR_JS.to_string(), src, false)
                } else {
                    let path = local_file(&root, file.parent().unwrap(), &src)?;
                    // Runner callbacks replace the visual reporter, whether its
                    // URL is root-relative or relative to the upstream test.
                    if !options.scaffold
                        && path
                            == root
                                .join("resources/testharnessreport.js")
                                .canonicalize()
                                .map_err(|e| e.to_string())?
                    {
                        continue;
                    }
                    let harness = path == harness_path;
                    (source(&root, &path, options.scaffold)?, src, harness)
                }
            }
            None => (script.body, "inline script".into(), false),
        };
        scheduler
            .run_script(&text)
            .map_err(|e| format!("{label}: {}", e.describe()))?;
        if harness {
            if installed {
                return Err("duplicate harness script".into());
            }
            scheduler.run_script("add_result_callback(__amazing_result); add_completion_callback(__amazing_complete); setup({output:false});")
                .map_err(|e| format!("callback installation: {}", e.describe()))?;
            installed = true;
        }
    }
    if !installed {
        return Err("no harness script loaded".into());
    }
    refresh_images(&mut scheduler)?;
    // Iframes present in the initial DOM were already queued by
    // `set_location_url` above (`RuntimeBindings::navigate_iframes`, run from
    // `set_document_url`); this drains that queue, giving each one its own
    // realm/WindowProxy and firing `DOMContentLoaded`/`load` on the child
    // document and `load` on the frame element (JSRT-9 + HTML-25..27).
    scheduler.dispatch_pending_iframe_loads().map_err(|e| e.describe())?;
    for event in ["DOMContentLoaded", "load"] {
        scheduler
            .dispatch_dom_event(0, event)
            .map_err(|e| e.describe())?;
    }
    use amazing_js::DomHost;
    let body = host.borrow_mut().query_selector("body");
    if let Some(body) = body {
        scheduler
            .dispatch_dom_event(body, "load")
            .map_err(|e| e.describe())?;
    }
    scheduler.end_document_parsing(0);
    // WPT runs against a deterministic 60 Hz virtual display. Each completed
    // turn gets one rendering opportunity, including a stable rAF timestamp.
    let mut frame = 0_u64;
    while !COMPLETE.get() {
        // Drains iframes queued since the last turn: script-driven
        // `src`/DOM mutations already pushed them via
        // `invoke_operation_with_html_hooks`'s `navigate_iframes` call.
        scheduler.dispatch_pending_iframe_loads().map_err(|e| e.describe())?;
        if let Some((next, replace, state)) = scheduler.js.take_history_update() {
            let next = resolve_page_url(navigator.current_url().unwrap_or(&url), &next)?;
            let serialized = scheduler
                .js
                .structured_serialize(&state)
                .map_err(|e| e.describe())?;
            navigator.same_document(next.clone(), replace, Some(serialized), 0)?;
            scheduler.set_location_url(&next);
            scheduler.js.set_history_length(navigator.history_length());
            scheduler.js.set_history_state(state);
        }
        if let Some(delta) = scheduler.js.take_history_delta() {
            if delta == 0 {
                let current = navigator
                    .current_url()
                    .ok_or("reload has no page")?
                    .to_owned();
                let page = fetch_page(navigator, &current, false, None)?;
                let file = fixture_file(root, fixture, &page.url)?;
                let html = source(root, &file, options.scaffold)?;
                if page.rendered != html.as_bytes() {
                    return Err("fixture reload differed from pinned page bytes".into());
                }
                return execute_page(
                    options,
                    root,
                    file,
                    html,
                    page.url,
                    fixture,
                    navigator,
                    page.history_state,
                    depth + 1,
                );
            } else if let Some((target, target_url)) = navigator
                .target(delta)
                .map(|(target, url)| (target, url.to_owned()))
            {
                if navigator.same_document_target(target) {
                    let (restored_url, _, state) = navigator.traverse_same_document(target, 0)?;
                    scheduler.set_location_url(restored_url);
                    scheduler.js.set_history_length(navigator.history_length());
                    let value = state
                        .as_ref()
                        .map(|state| scheduler.js.structured_deserialize(state))
                        .transpose()
                        .map_err(|e| e.describe())?
                        .unwrap_or(JsValue::Null);
                    scheduler.js.set_history_state(value);
                } else {
                    let page = fetch_page(navigator, &target_url, false, Some(target))?;
                    let file = fixture_file(root, fixture, &page.url)?;
                    let html = source(root, &file, options.scaffold)?;
                    if page.rendered != html.as_bytes() {
                        return Err("fixture traversal differed from pinned page bytes".into());
                    }
                    return execute_page(
                        options,
                        root,
                        file,
                        html,
                        page.url,
                        fixture,
                        navigator,
                        page.history_state,
                        depth + 1,
                    );
                }
            }
        }
        if let Some(next) = scheduler.js.take_navigation() {
            let next = resolve_page_url(navigator.current_url().unwrap_or(&url), &next)?;
            let page = fetch_page(navigator, &next, true, None)?;
            let file = fixture_file(root, fixture, &page.url)?;
            let html = source(root, &file, options.scaffold)?;
            if page.rendered != html.as_bytes() {
                return Err("fixture navigation differed from pinned page bytes".into());
            }
            return execute_page(
                options,
                root,
                file,
                html,
                page.url,
                fixture,
                navigator,
                page.history_state,
                depth + 1,
            );
        }
        scheduler.pump_at(Duration::from_micros(frame.saturating_mul(16_667)), true)
            .map_err(|e| e.describe())?;
        frame = frame.saturating_add(1);
    }
    Ok(())
}

/// The venus build hosts have no git; `scripts/ensure-wpt.sh` extracts the
/// pinned revision from a GitHub tarball there instead, and marks it with an
/// empty `.pinned-<rev7>` file (its own convention -- see that script). A
/// `git` command always fails against that checkout even though it is
/// correctly pinned, so both pin checks below (the checkout-level one in
/// `supervise` and the per-resource one in `source`) accept this marker as
/// an alternative proof to a real git checkout at exactly `REVISION`.
fn checkout_is_marker_pinned(root: &Path) -> bool {
    root.join(format!(".pinned-{}", &REVISION[..7])).exists()
}

fn supervise(options: &Options, args: &[String], report: &mut Report) -> Result<(), String> {
    if !options.scaffold {
        let pinned = if checkout_is_marker_pinned(&options.root) {
            true
        } else {
            let revision = Command::new("git")
                .arg("-C")
                .arg(&options.root)
                .args(["rev-parse", "HEAD"])
                .output()
                .map_err(|e| e.to_string())?;
            revision.status.success()
                && String::from_utf8_lossy(&revision.stdout).trim() == REVISION
        };
        if !pinned {
            return Err(format!("WPT checkout must be pinned at {REVISION}"));
        }
        report.verified = true;
    }
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .arg("wpt-harness-worker")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if std::env::var_os("AMAZING_HTTP_CACHE_DIR").is_none() {
        let root = std::env::var_os("AMAZING_STORAGE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        command.env(
            "AMAZING_HTTP_CACHE_DIR",
            root.join(format!("wpt-http-cache-{}", std::process::id())),
        );
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut total = 0;
        for line in BufReader::new(stdout.take(OUTPUT_LIMIT + 1)).lines() {
            let line = line.map_err(|e| e.to_string())?;
            total += line.len() as u64 + 1;
            if total > OUTPUT_LIMIT {
                return Err("worker output limit exceeded".to_string());
            }
            if sender.send(line).is_err() {
                break;
            }
        }
        Ok::<_, String>(())
    });
    let start = Instant::now();
    let mut worker_error = None;
    loop {
        for line in receiver.try_iter() {
            report.accept(&line);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    worker_error = Some(format!("worker exited: {status}"));
                }
                break;
            }
            Ok(None) if start.elapsed() < Duration::from_millis(options.timeout.into()) => {
                std::thread::sleep(Duration::from_millis(5))
            }
            other => {
                let _ = child.kill();
                let _ = child.wait();
                match other {
                    Err(e) => worker_error = Some(e.to_string()),
                    _ => report.timed_out = true,
                }
                break;
            }
        }
    }
    let read_result = reader.join().map_err(|_| "output reader panicked")?;
    for line in receiver.try_iter() {
        report.accept(&line);
    }
    read_result?;
    if let Some(error) = worker_error {
        return Err(error);
    }
    if report.timed_out {
        report.detail = Some(quote(
            "worker deadline exceeded; completed subtests retained",
        ));
    } else if report.harness.is_none() && !report.error && !report.unsupported {
        return Err("worker exited without completion callback".into());
    }
    Ok(())
}

pub fn run(args: &[String]) -> ExitCode {
    let options = match Options::parse(&args[1..]) {
        Ok(o) => o,
        Err(e) => {
            println!(
                "{{\"schema\":1,\"status\":\"error\",\"detail\":{}}}",
                quote(&e)
            );
            return ExitCode::FAILURE;
        }
    };
    if args[0] == "wpt-harness-worker" {
        if let Err(e) = execute(&options) {
            let kind = if e.starts_with("unsupported:") {
                "unsupported"
            } else {
                "error"
            };
            let _ = emit(&format!("{kind}\t{}", quote(&e)));
        }
        return ExitCode::SUCCESS;
    }
    let mut report = Report::default();
    if let Err(e) = supervise(&options, &args[1..], &mut report) {
        report.error = true;
        report.detail = Some(quote(&e));
    }
    println!("{}", report.json(&options));
    if report.status() == "pass" {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod any_window_tests {
    use super::any_window_wrapper;

    #[test]
    fn any_window_wrapper_orders_upstream_metadata_before_source() {
        let source = "// META: global=window,worker\n// META: timeout=long\n// META: title=a < b & c\n// META: script=/common/helper.js\ntest(() => {}, 'case');\n";
        let html = any_window_wrapper(source, "url/example.any.js").unwrap();
        assert!(html.starts_with("<!doctype html>\n<meta charset=utf-8>\n"));
        assert!(html.contains("<meta name=\"timeout\" content=\"long\">"));
        assert!(html.contains("<title>a &lt; b &amp; c</title>"));
        assert!(html.contains("self.GLOBAL = {"));
        let harness = html.find("/resources/testharness.js").unwrap();
        let reporter = html.find("/resources/testharnessreport.js").unwrap();
        let include = html.find("/common/helper.js").unwrap();
        let test = html.find("/url/example.any.js").unwrap();
        assert!(harness < reporter && reporter < include && include < test);
    }

    #[test]
    fn any_window_wrapper_obeys_upstream_global_selection() {
        assert!(
            any_window_wrapper("// META: global=worker\n", "url/worker.any.js")
                .unwrap_err()
                .contains("excludes window")
        );
    }
}
