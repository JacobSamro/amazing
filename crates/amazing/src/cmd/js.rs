//! JavaScript commands and page script execution.
use std::{cell::RefCell, rc::Rc, process::ExitCode, time::{Duration, Instant}};
use amazing_js::DomHost;
use crate::dom_host::BrowserDomHost;
use super::parse::read_source;
pub(crate) struct PageScript {
    /// HTML-29: the script element's own tree position, so a top-level
    /// script's `document.write` call can be narrowed to the parser's
    /// insertion point right after it (`DriverScheduler::
    /// narrow_write_insertion_point`) rather than always landing at the end
    /// of the body.
    pub(crate) node: amazing_dom::NodeIdx,
    pub(crate) src: Option<String>,
    pub(crate) body: String,
}

/// A `<script>` is classic JavaScript only when its `type` is absent, empty, or
/// a JavaScript MIME type. Data blocks (`application/ld+json`, `text/template`,
/// `importmap`, ...) must never execute, and modules are not supported here, so
/// both are skipped instead of being parsed as classic scripts. Executing a
/// data block or a module as classic JS produced spurious parse errors on real
/// pages.
fn is_classic_javascript_type(raw: Option<&str>) -> bool {
    let Some(raw) = raw else { return true };
    let value = raw.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    value.is_empty()
        || matches!(
            value.as_str(),
            "text/javascript"
                | "application/javascript"
                | "application/ecmascript"
                | "text/ecmascript"
                | "application/x-javascript"
                | "text/x-javascript"
                | "text/jscript"
                | "text/livescript"
        )
}

pub(crate) fn collect_script_text(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    scripts: &mut Vec<PageScript>,
) {
    if dom.name(node) == Some("script") {
        let attrs = dom.attrs_of(node).unwrap_or(&[]);
        let script_type = attrs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("type"))
            .map(|(_, value)| value.as_str());
        if !is_classic_javascript_type(script_type) {
            return;
        }
        let mut source = String::new();
        for child in dom.children(node) {
            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                source.push_str(data);
            }
        }
        let src = attrs
            .iter()
            .find(|(name, _)| name == "src")
            .map(|(_, value)| value.clone());
        if src.is_some() || !source.trim().is_empty() {
            scripts.push(PageScript { node, src, body: source });
        }
        return;
    }
    for child in dom.children(node) {
        collect_script_text(dom, child, scripts);
    }
}

pub(crate) fn collect_inline_handlers(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    handlers: &mut Vec<(u32, String, String)>,
) {
    if let Some(attrs) = dom.attrs_of(node) {
        for (name, body) in attrs {
            if let Some(event) = name.strip_prefix("on").filter(|event| !event.is_empty()) {
                handlers.push((node.0, event.to_owned(), body.clone()));
            }
        }
    }
    for child in dom.children(node) {
        collect_inline_handlers(dom, child, handlers);
    }
}

pub(crate) fn execute_inline_scripts(host: &Rc<RefCell<BrowserDomHost>>, page: &str) -> Result<(), String> {
    if let Some(mut scheduler) = prepare_page_scripts(host, page, false)? {
        scheduler.pump_at(std::time::Duration::from_millis(16), true)
            .map_err(|error| format!("page event loop failed: {}", error.describe()))?;
    }
    Ok(())
}

/// Result of the lenient real-page script pass: the collected failures plus the
/// console lines produced by the optional API probe script.
pub(crate) struct LenientScriptResult {
    pub(crate) errors: Vec<String>,
    pub(crate) probe: Vec<String>,
}

/// Real-page headless path: run every script, handler and the bounded event
/// loop, but collect each failure instead of aborting at the first one. The
/// page still paints, and the caller reports the errors. The reftest path keeps
/// the strict `prepare_page_scripts`, where a thrown script is never a pass.
///
/// When `probe` is supplied it runs in the same interpreter after the page's
/// scripts, events and event-loop steps; only its console output is returned in
/// `probe`, so a live corpus can report missing globals per page.
pub(crate) fn execute_page_scripts_lenient(
    host: &Rc<RefCell<BrowserDomHost>>,
    page: &str,
    probe: Option<&str>,
) -> LenientScriptResult {
    let mut errors = Vec::new();
    let mut probe_lines = Vec::new();
    let (scripts, handlers) = {
        let host_ref = host.borrow();
        let mut scripts = Vec::new();
        collect_script_text(&host_ref.dom, amazing_dom::NodeIdx(0), &mut scripts);
        let mut handlers = Vec::new();
        collect_inline_handlers(&host_ref.dom, amazing_dom::NodeIdx(0), &mut handlers);
        (scripts, handlers)
    };
    if scripts.is_empty() && handlers.is_empty() && probe.is_none() {
        return LenientScriptResult { errors, probe: probe_lines };
    }
    let mut scheduler = match crate::js_scheduler::DriverScheduler::new(
        host.clone(), crate::js_scheduler::JS_CAPACITY) {
        Ok(scheduler) => scheduler,
        Err(error) => {
            errors.push(format!("scheduler init: {}", error.describe()));
            return LenientScriptResult { errors, probe: probe_lines };
        }
    };
    scheduler.set_location_url(page);
    // IA-LIVE-2: a real page's script phase can burn far more wall time than
    // any single sub-operation's own timeout suggests. Each blocking
    // synchronous network primitive underneath (external <script src> fetch,
    // sync/async XHR bridging in `runtime_host_net.rs`) is individually
    // bounded (10-15s), but nothing previously capped how many of those a
    // page's own script could chain sequentially -- `docs.rs/serde` and
    // `vuejs.org` each burned minutes of real wall time this way with zero
    // errors surfaced (a script-level fetch/XHR failure is an unhandled
    // promise rejection, invisible to this harness, not a `Throw` from
    // `run_script`). Cap the WHOLE script phase (handlers, scripts, the
    // load/DOMContentLoaded dispatch and the event-loop pump) at one overall
    // deadline so one page can no longer blow a corpus run's per-page
    // budget; remaining scripts/handlers/pumps are skipped and reported
    // instead of silently eating the rest of the process's time. 45s is
    // comfortably under the 150s the corpus harnesses kill a child at, while
    // leaving room for the paint phase's own (separately bounded) font/image
    // fetches.
    let budget = std::env::var("AMAZING_SCRIPT_PHASE_BUDGET_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45u64);
    let deadline = Instant::now() + Duration::from_secs(budget);
    let mut budget_exceeded = false;
    macro_rules! check_budget {
        ($skipped:expr) => {
            if !budget_exceeded && Instant::now() >= deadline {
                budget_exceeded = true;
                errors.push(format!(
                    "script phase exceeded its {budget}s budget; {} skipped",
                    $skipped
                ));
            }
            if budget_exceeded {
                continue;
            }
        };
    }
    for (node, event, body) in handlers {
        check_budget!("remaining inline handlers");
        if let Err(error) = scheduler.install_inline_handler(node, &event, &body) {
            errors.push(format!("inline {event} handler on node {node}: {}", error.describe()));
        }
    }
    // HTML-29: brackets the page's own top-level script phase so a
    // `document.write` call inserts at the parser's insertion point
    // (narrowed to right after each script) rather than always implicitly
    // reopening the document; ended once the load event has fired below.
    scheduler.begin_document_parsing(0);
    for script in scripts {
        check_budget!("remaining page scripts");
        scheduler.narrow_write_insertion_point(0, script.node.0);
        let source = match script.src {
            Some(src) => {
                let script_url = resolve_script_url(page, &src);
                let fetched = amazing_net::fetch(&script_url, &amazing_net::FetchOptions::default())
                    .map_err(|error| format!("{error}"))
                    .and_then(|response| response.text()
                        .ok_or_else(|| format!("external script {src} was not text")));
                match fetched {
                    Ok(source) => source,
                    Err(error) => { errors.push(format!("external script {src}: {error}")); continue; }
                }
            }
            None => script.body,
        };
        if let Err(error) = scheduler.run_script(&source) {
            errors.push(format!("page script: {}", error.describe()));
        }
    }
    if budget_exceeded {
        errors.push("skipped DOMContentLoaded/load dispatch and the event loop: script phase budget already exceeded".to_owned());
        scheduler.end_document_parsing(0);
        return LenientScriptResult { errors, probe: probe_lines };
    }
    if let Err(error) = scheduler.dispatch_dom_event(0, "DOMContentLoaded") {
        errors.push(format!("DOMContentLoaded listener: {}", error.describe()));
    }
    if let Err(error) = scheduler.dispatch_dom_event(0, "load") {
        errors.push(format!("load listener: {}", error.describe()));
    }
    let body = host.borrow_mut().query_selector("body");
    if let Some(body) = body
        && let Err(error) = scheduler.dispatch_dom_event(body, "load")
    {
        errors.push(format!("body load listener: {}", error.describe()));
    }
    scheduler.end_document_parsing(0);
    // Keep the page's timers/rAF bounded and deterministic for headless runs.
    for _ in 0..16 {
        if Instant::now() >= deadline {
            errors.push(format!("script phase exceeded its {budget}s budget during the event loop"));
            break;
        }
        match scheduler.pump_at(std::time::Duration::from_millis(16), true) {
            Ok(true) => {},
            Ok(false) => break,
            Err(error) => { errors.push(format!("page event loop: {}", error.describe())); break; }
        }
    }
    if let Some(source) = probe {
        let before = scheduler.js.output.borrow().len();
        if let Err(error) = scheduler.run_script(source) {
            errors.push(format!("probe script: {}", error.describe()));
        }
        let output = scheduler.js.output.borrow();
        probe_lines.extend(output.iter().skip(before).cloned());
    }
    LenientScriptResult { errors, probe: probe_lines }
}

/// Starts the page's script and event-loop lifetime. WPT keeps this scheduler
/// through first paint and its reftest checkpoint; ordinary render performs
/// its existing 16 ms checkpoint in `execute_inline_scripts`.
pub(crate) fn prepare_page_scripts(
    host: &Rc<RefCell<BrowserDomHost>>,
    page: &str,
    force_scheduler: bool,
) -> Result<Option<crate::js_scheduler::DriverScheduler>, String> {
    let scripts = {
        let host_ref = host.borrow();
        let mut scripts = Vec::new();
        collect_script_text(&host_ref.dom, amazing_dom::NodeIdx(0), &mut scripts);
        scripts
    };
    if scripts.is_empty() && !force_scheduler {
        return Ok(None);
    }
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host.clone(), crate::js_scheduler::JS_CAPACITY)
        .map_err(|error| error.describe())?;
    scheduler.set_location_url(page);
    let handlers = {
        let host_ref = host.borrow();
        let mut handlers = Vec::new();
        collect_inline_handlers(&host_ref.dom, amazing_dom::NodeIdx(0), &mut handlers);
        handlers
    };
    for (node, event, body) in handlers {
        scheduler.install_inline_handler(node, &event, &body)
            .map_err(|error| format!("inline onclick failed: {}", error.describe()))?;
    }
    // HTML-29: see the matching comment in `execute_page_scripts_lenient`.
    scheduler.begin_document_parsing(0);
    for script in scripts {
        scheduler.narrow_write_insertion_point(0, script.node.0);
        let source = match script.src {
            Some(src) => {
                let script_url = resolve_script_url(page, &src);
                let response =
                    amazing_net::fetch(&script_url, &amazing_net::FetchOptions::default())
                        .map_err(|error| format!("external script {src}: {error}"))?;
                response
                    .text()
                    .ok_or_else(|| format!("external script {src} was not text"))?
            }
            None => script.body,
        };
        scheduler.run_script(&source)
            .map_err(|error| format!("page script failed: {}", error.describe()))?;
    }
    scheduler.dispatch_dom_event(0, "DOMContentLoaded")
        .map_err(|error| format!("DOMContentLoaded listener failed: {}", error.describe()))?;
    scheduler.dispatch_dom_event(0, "load")
        .map_err(|error| format!("load listener failed: {}", error.describe()))?;
    let body = host.borrow_mut().query_selector("body");
    if let Some(body) = body {
        scheduler.dispatch_dom_event(body, "load")
            .map_err(|error| format!("body load listener failed: {}", error.describe()))?;
    }
    scheduler.end_document_parsing(0);
    Ok(Some(scheduler))
}

pub(crate) fn resolve_script_url(page: &str, source: &str) -> String {
    // The WPT runner supplies this only to its renderer child. A filesystem
    // drive root is not the web server's URL root.
    if let Some(root) = std::env::var_os("AMAZING_WPT_ROOT")
        && let Some(mapped) = wpt_root_relative_url(page, source, std::path::Path::new(&root))
    {
        return mapped;
    }
    if source.contains("://") || source.starts_with("file:")
        || source.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
    {
        return source.to_owned();
    }
    if (page.starts_with("http://") || page.starts_with("https://") || page.starts_with("file:"))
        && let Ok(base) = amazing_net::Url::parse(page)
        && let Ok(resolved) = amazing_net::resolve(source, &base)
    {
        return resolved.to_string();
    }
    std::path::Path::new(page)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(source)
        .to_string_lossy()
        .into_owned()
}

/// Phase 6 demo: execute one classic JavaScript script and print captured
/// `console.log` lines followed by the script completion value.
pub(crate) fn cmd_run(path: Option<&str>) -> ExitCode {
    let source = match read_source(path, "run") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_dom::Dom::new(), native_forwarded: false }));
    let mut scheduler = match crate::js_scheduler::DriverScheduler::new(host, crate::js_scheduler::JS_CAPACITY) {
        Ok(scheduler) => scheduler,
        Err(error) => { eprintln!("amazing run: {}", error.describe()); return ExitCode::FAILURE; }
    };
    let entry = scheduler.run_script(&source);
    let checkpoint = scheduler.pump_at(std::time::Duration::from_millis(16), true);
    let result = entry.and_then(|value| checkpoint.map(|_| value));
    match result {
        Ok(value) => {
            for line in scheduler.js.output.borrow().iter() {
                println!("{line}");
            }
            println!("=> {:?}", value);
            ExitCode::SUCCESS
        }
        Err(thrown) => {
            eprintln!("amazing run: {}", format_js_error(&thrown.0));
            ExitCode::FAILURE
        }
    }
}

/// Phase 7 demo: execute a script against a live arena DOM and print the
/// resulting tree. The bridge intentionally starts with the high-value DOM
/// surface: querySelector, textContent, attributes, createElement, append.
pub(crate) fn cmd_domrun(args: &[String]) -> ExitCode {
    let (html_path, script_path) = match (args.first(), args.get(1)) {
        (Some(html), Some(script)) => (html, script),
        _ => {
            eprintln!("usage: amazing domrun <page.html> <script.js>");
            return ExitCode::FAILURE;
        }
    };
    let html = match super::parse::read_bytes(Some(html_path), "domrun html") {
        Ok(source) => source,
        Err(code) => return code,
    };
    let document = match crate::document::parse_document_bytes(&html, html_path, None) {
        Ok(document) => document,
        Err(error) => {
            eprintln!("amazing domrun: {error}");
            return error.exit_code();
        }
    };
    if document.kind == crate::document::DocumentKind::Xml {
        let error = crate::document::PageError::Unsupported("XML DOM scripting is not implemented");
        eprintln!("amazing domrun: {error}");
        return error.exit_code();
    }
    let script = match read_source(Some(script_path), "domrun script") {
        Ok(source) => source,
        Err(code) => return code,
    };
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: document.dom,
        native_forwarded: false,
    }));
    let mut scheduler = match crate::js_scheduler::DriverScheduler::new(host.clone(), crate::js_scheduler::JS_CAPACITY) {
        Ok(scheduler) => scheduler,
        Err(error) => { eprintln!("amazing domrun: {}", error.describe()); return ExitCode::FAILURE; }
    };
    let entry = scheduler.run_script(&script);
    let checkpoint = scheduler.pump_at(std::time::Duration::from_millis(16), true);
    let result = entry.and_then(|value| checkpoint.map(|_| value));
    match result {
        Ok(value) => {
            for line in scheduler.js.output.borrow().iter() {
                println!("{line}");
            }
            println!("=> {:?}", value);
            print!("{}", host.borrow().dom.dump(amazing_dom::NodeIdx(0)));
            ExitCode::SUCCESS
        }
        Err(thrown) => {
            eprintln!("amazing domrun: {}", format_js_error(&thrown.0));
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn format_js_error(value: &amazing_js::value::JsValue) -> String {
    match value {
        amazing_js::value::JsValue::Object(object) => {
            let name = object
                .borrow()
                .get_own("name")
                .and_then(|v| match v {
                    amazing_js::value::JsValue::Str(s) => Some(s.to_utf8_lossy()),
                    _ => None,
                })
                .unwrap_or_else(|| "Error".to_string());
            let message = object
                .borrow()
                .get_own("message")
                .and_then(|v| match v {
                    amazing_js::value::JsValue::Str(s) => Some(s.to_utf8_lossy()),
                    _ => None,
                })
                .unwrap_or_default();
            if message.is_empty() {
                name
            } else {
                format!("{name}: {message}")
            }
        }
        amazing_js::value::JsValue::Str(s) => s.to_utf8_lossy(),
        other => format!("{other:?}"),
    }
}

/// Map a URL-absolute resource into the explicitly configured WPT checkout.
/// Ordinary pages outside that checkout retain their normal URL semantics.
fn wpt_root_relative_url(page: &str, source: &str, root: &std::path::Path) -> Option<String> {
    if !source.starts_with('/') {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let page = if page.starts_with("file:") {
        amazing_net::Url::parse(page).ok()?.to_file_path().ok()?
    } else {
        std::path::PathBuf::from(page)
    };
    if !page.canonicalize().ok()?.starts_with(&root) {
        return None;
    }
    let invalid = || Some("about:invalid#blocked-wpt-resource".to_owned());
    if source.starts_with("//") {
        return invalid();
    }
    let Ok(url) = amazing_net::Url::parse(&format!("https://wpt.invalid{source}")) else {
        return invalid();
    };
    let decoded = url.decoded_path().replace('\\', "/");
    let mut relative = std::path::PathBuf::new();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {},
            ".." => { relative.pop(); },
            segment if segment.contains([':', '\0']) => return invalid(),
            segment => relative.push(segment),
        }
    }
    let resolved = root.join(relative);
    if let Ok(actual) = resolved.canonicalize()
        && !actual.starts_with(&root)
    {
        return invalid();
    }
    Some(resolved.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_relative_resources_use_the_wpt_root_not_the_test_directory() {
        let root = std::env::temp_dir().join(format!("amazing-wpt-root-{}", std::process::id()));
        std::fs::create_dir_all(root.join("css/nested")).unwrap();
        std::fs::create_dir_all(root.join("common")).unwrap();
        let page = root.join("css/nested/test.html");
        std::fs::write(&page, "<!doctype html>").unwrap();
        std::fs::write(root.join("common/helper script.js"), "42").unwrap();
        let expected = root.canonicalize().unwrap().join("common/helper script.js");
        for source in ["/common/helper%20script.js?cache=1#fragment", "/../common/helper%20script.js"] {
            assert_eq!(wpt_root_relative_url(&page.to_string_lossy(), source, &root), Some(expected.to_string_lossy().into_owned()));
        }
        assert_eq!(wpt_root_relative_url(&page.to_string_lossy(), "relative.js", &root), None);
        assert_eq!(wpt_root_relative_url("https://example.org/page", "/common/helper.js", &root), None);
        assert_eq!(wpt_root_relative_url(&page.to_string_lossy(), "/C:/outside.js", &root), Some("about:invalid#blocked-wpt-resource".into()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_classic_javascript_script_types_execute() {
        assert!(is_classic_javascript_type(None));
        assert!(is_classic_javascript_type(Some("")));
        assert!(is_classic_javascript_type(Some("text/javascript")));
        assert!(is_classic_javascript_type(Some("TEXT/JavaScript; charset=utf-8")));
        assert!(is_classic_javascript_type(Some("application/javascript")));
        for data in [
            "module",
            "application/ld+json",
            "application/json",
            "text/template",
            "importmap",
            "text/html",
            "speculationrules",
        ] {
            assert!(
                !is_classic_javascript_type(Some(data)),
                "{data} must not run as classic JS"
            );
        }
    }
}
