// DOM-1 experimental classic-script runner. Assertions belong to upstream WPT.
// --scaffold is a separate, explicitly non-conformance plumbing diagnostic.
use amazing_js::Interpreter;
use amazing_js::value::{FunctionBody, JsFunction, JsObject, JsValue, NativeFn, Obj, Throw};
use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const REVISION: &str = "c2a2fce69e5c2509d9dc28440ec8b42532e5c293";
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
thread_local! { static COMPLETE: Cell<bool> = const { Cell::new(false) }; }

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
        JsValue::Str(s) => quote(&s),
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
    String::from_utf8(bytes).map_err(|e| e.to_string())
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
fn execute(options: &Options) -> Result<(), String> {
    COMPLETE.set(false);
    let root = options.root.canonicalize().map_err(|e| e.to_string())?;
    let file = local_file(&root, &root, &options.test)?;
    if file.extension().and_then(|s| s.to_str()) != Some("html") {
        return Err("unsupported: classic-script .html tests only".into());
    }
    let html = source(&root, &file, options.scaffold)?;
    let host = Rc::new(RefCell::new(crate::BrowserDomHost {
        dom: amazing_html::parse(&html),
    }));
    let mut scripts = Vec::new();
    collect_scripts(&host.borrow().dom, amazing_dom::NodeIdx(0), &mut scripts)?;
    let mut js = Interpreter::new();
    js.set_dom_host(host.clone());
    js.set_viewport_size(options.width, options.height);
    js.set_location_url(format!("http://web-platform.test/{}", options.test));
    // The engine's browser-global identity also supplies the standard self alias.
    let global = JsValue::Object(js.global_obj.clone());
    js.global_obj.borrow_mut().set_own("self", global.clone());
    js.global_env
        .declare_var("self", global)
        .map_err(|e| e.describe())?;
    install_callback(&mut js, "__amazing_result", result_callback)?;
    install_callback(&mut js, "__amazing_complete", completion_callback)?;
    let mut handlers = Vec::new();
    crate::collect_inline_handlers(&host.borrow().dom, amazing_dom::NodeIdx(0), &mut handlers);
    for (node, event, body) in handlers {
        js.install_inline_handler(node, &event, &body)
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
                // WPT's visual reporter is replaced by runner callbacks.
                if !options.scaffold && src == "/resources/testharnessreport.js" {
                    continue;
                }
                let path = local_file(&root, file.parent().unwrap(), &src)?;
                let harness = path == harness_path;
                (source(&root, &path, options.scaffold)?, src, harness)
            }
            None => (script.body, "inline script".into(), false),
        };
        js.run_script(&text)
            .map_err(|e| format!("{label}: {}", e.describe()))?;
        if harness {
            if installed {
                return Err("duplicate harness script".into());
            }
            js.run_script("add_result_callback(__amazing_result); add_completion_callback(__amazing_complete); setup({output:false, message_events:[]});")
                .map_err(|e| format!("callback installation: {}", e.describe()))?;
            installed = true;
        }
    }
    if !installed {
        return Err("no harness script loaded".into());
    }
    for event in ["DOMContentLoaded", "load"] {
        js.dispatch_dom_event(0, event).map_err(|e| e.describe())?;
    }
    use amazing_js::DomHost;
    let body = host.borrow_mut().query_selector("body");
    if let Some(body) = body {
        js.dispatch_dom_event(body, "load")
            .map_err(|e| e.describe())?;
    }
    while !COMPLETE.get() {
        js.drain_microtasks().map_err(|e| e.describe())?;
        js.pump_timers().map_err(|e| e.describe())?;
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
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("wpt-harness-worker")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
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
