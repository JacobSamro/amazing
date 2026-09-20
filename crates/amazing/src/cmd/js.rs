//! JavaScript commands and page script execution.
use std::{cell::RefCell, rc::Rc, process::ExitCode};
use amazing_js::DomHost;
use crate::dom_host::BrowserDomHost;
use super::parse::read_source;
pub(crate) struct PageScript {
    pub(crate) src: Option<String>,
    pub(crate) body: String,
}

pub(crate) fn collect_script_text(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    scripts: &mut Vec<PageScript>,
) {
    if dom.name(node) == Some("script") {
        let mut source = String::new();
        for child in dom.children(node) {
            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                source.push_str(data);
            }
        }
        let src = dom
            .attrs_of(node)
            .and_then(|attrs| attrs.iter().find(|(name, _)| name == "src"))
            .map(|(_, value)| value.clone());
        if src.is_some() || !source.trim().is_empty() {
            scripts.push(PageScript { src, body: source });
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
    let scripts = {
        let host_ref = host.borrow();
        let mut scripts = Vec::new();
        collect_script_text(&host_ref.dom, amazing_dom::NodeIdx(0), &mut scripts);
        scripts
    };
    if scripts.is_empty() {
        return Ok(());
    }
    let mut js = amazing_js::Interpreter::new();
    js.set_dom_host(host.clone());
    js.set_location_url(page);
    let handlers = {
        let host_ref = host.borrow();
        let mut handlers = Vec::new();
        collect_inline_handlers(&host_ref.dom, amazing_dom::NodeIdx(0), &mut handlers);
        handlers
    };
    for (node, event, body) in handlers {
        js.install_inline_handler(node, &event, &body)
            .map_err(|error| format!("inline onclick failed: {}", error.describe()))?;
    }
    for script in scripts {
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
        js.run_script(&source)
            .map_err(|error| format!("page script failed: {}", error.describe()))?;
    }
    js.dispatch_dom_event(0, "DOMContentLoaded")
        .map_err(|error| format!("DOMContentLoaded listener failed: {}", error.describe()))?;
    js.dispatch_dom_event(0, "load")
        .map_err(|error| format!("load listener failed: {}", error.describe()))?;
    let body = host.borrow_mut().query_selector("body");
    if let Some(body) = body {
        js.dispatch_dom_event(body, "load")
            .map_err(|error| format!("body load listener failed: {}", error.describe()))?;
    }
    Ok(())
}

pub(crate) fn resolve_script_url(page: &str, source: &str) -> String {
    if source.contains("://") || source.starts_with("file:") {
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
    let mut js = amazing_js::Interpreter::new();
    match js.run_script(&source) {
        Ok(value) => {
            for line in js.output.borrow().iter() {
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
    let html = match read_source(Some(html_path), "domrun html") {
        Ok(source) => source,
        Err(code) => return code,
    };
    let script = match read_source(Some(script_path), "domrun script") {
        Ok(source) => source,
        Err(code) => return code,
    };
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(&html),
    }));
    let mut js = amazing_js::Interpreter::new();
    js.set_dom_host(host.clone());
    match js.run_script(&script) {
        Ok(value) => {
            for line in js.output.borrow().iter() {
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
                    amazing_js::value::JsValue::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "Error".to_string());
            let message = object
                .borrow()
                .get_own("message")
                .and_then(|v| match v {
                    amazing_js::value::JsValue::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if message.is_empty() {
                name
            } else {
                format!("{name}: {message}")
            }
        }
        amazing_js::value::JsValue::Str(s) => s.to_string(),
        other => format!("{other:?}"),
    }
}

