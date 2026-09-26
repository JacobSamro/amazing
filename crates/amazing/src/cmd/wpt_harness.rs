// DOM-1 experimental classic-script runner. Assertions belong to upstream WPT.
// --scaffold is a separate, explicitly non-conformance plumbing diagnostic.
use amazing_js::Interpreter;
use amazing_js::value::{FunctionBody, JsFunction, JsObject, JsValue, NativeFn, Obj, Throw};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
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
    let body = path.as_deref().and_then(|path| {
        let name = path.split(['?', '#']).next()?.strip_prefix('/')?;
        if name.contains('%') || name.contains('\\') || name.split('/').any(|part| part == "..") {
            return None;
        }
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
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
    if !scaffold {
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
            ("{{hosts[alt][]}}", "not-web-platform.test"),
            ("{{hosts[alt][www2]}}", "www2.not-web-platform.test"),
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

fn collect_iframes(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    frames: &mut Vec<(u32, String)>,
) {
    if dom.name(node) == Some("iframe")
        && let Some(src) = dom
            .attrs_of(node)
            .unwrap_or(&[])
            .iter()
            .find(|(name, _)| name == "src")
            .map(|(_, value)| value)
    {
        frames.push((node.0, src.clone()));
    }
    for child in dom.children(node) {
        collect_iframes(dom, child, frames);
    }
}

fn service_iframes(
    host: &Rc<RefCell<crate::BrowserDomHost>>,
    scheduler: &mut crate::js_scheduler::DriverScheduler,
    navigator: &mut crate::navigator::Navigator,
    page_url: &str,
    loaded: &mut BTreeMap<u32, (u64, String)>,
) -> Result<(), String> {
    let mut iframes = Vec::new();
    collect_iframes(&host.borrow().dom, amazing_dom::NodeIdx(0), &mut iframes);
    for (node, src) in iframes {
        if src.is_empty()
            || src == "about:blank"
            || loaded.get(&node).is_some_and(|(_, old)| old == &src)
        {
            continue;
        }
        let child_url = resolve_page_url(page_url, &src)?;
        let child = match loaded.get(&node) {
            Some((id, _)) => *id,
            None => navigator.create_child(1)?,
        };
        let use_process = crate::runtime_host_ipc::enabled();
        navigator.navigate_child(child, &child_url, |url, process_body| {
            if use_process {
                Ok(process_body.to_vec())
            } else {
                amazing_net::fetch(url, &amazing_net::FetchOptions::default())
                    .map(|response| response.body)
                    .map_err(|error| format!("iframe navigation: {error}"))
            }
        })?;
        loaded.insert(node, (child, src));
        scheduler
            .dispatch_dom_event(node, "load")
            .map_err(|error| error.describe())?;
    }
    Ok(())
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
    for script in scripts {
        let (text, label, harness) = match script.src {
            Some(src) => {
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
    let mut loaded_iframes = BTreeMap::new();
    service_iframes(&host, &mut scheduler, navigator, &url, &mut loaded_iframes)?;
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
    while !COMPLETE.get() {
        service_iframes(&host, &mut scheduler, navigator, &url, &mut loaded_iframes)?;
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
        scheduler.pump_now(true).map_err(|e| e.describe())?;
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn supervise(options: &Options, args: &[String], report: &mut Report) -> Result<(), String> {
    if !options.scaffold {
        let revision = Command::new("git")
            .arg("-C")
            .arg(&options.root)
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|e| e.to_string())?;
        if !revision.status.success()
            || String::from_utf8_lossy(&revision.stdout).trim() != REVISION
        {
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
