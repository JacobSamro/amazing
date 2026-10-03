//! JavaScript commands and page script execution.
use std::{cell::RefCell, rc::Rc, process::ExitCode, time::{Duration, Instant}};
use amazing_js::DomHost;
use crate::dom_host::BrowserDomHost;
use super::parse::read_source;
pub(crate) struct PageScript {
    pub(crate) kind: ScriptKind,
    /// HTML-29: the script element's own tree position, so a top-level
    /// script's `document.write` call can be narrowed to the parser's
    /// insertion point right after it (`DriverScheduler::
    /// narrow_write_insertion_point`) rather than always landing at the end
    /// of the body.
    pub(crate) node: amazing_dom::NodeIdx,
    /// Captured when this script record is created; a recycled slot is a different node.
    pub(crate) node_generation: Option<u64>,
    pub(crate) src: Option<String>,
    pub(crate) body: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptKind { Classic, Module }

/// A `<script>` is classic JavaScript only when its `type` is absent, empty, or
/// a JavaScript MIME type. Data blocks (`application/ld+json`, `text/template`,
/// `importmap`, ...) must never execute. Modules use their own registry rather
/// than the shared classic script environment.
pub(crate) fn is_classic_javascript_type(raw: Option<&str>) -> bool {
    let Some(raw) = raw else { return true };
    // Matches `amazing_html::tree`'s own `classic_or_module_script_type`
    // exactly (HTML-31 fix): the spec's "script block's type string" strips
    // only leading/trailing ASCII whitespace, never `;parameter`s and never
    // non-ASCII whitespace -- `raw.split(';').next()` and `str::trim()`
    // (Unicode-whitespace-aware) both wrongly accepted types this crate's
    // own `is_classic_javascript_type` must reject, e.g.
    // "text/javascript;charset=utf-8" or "text/javascript\u{00A0}". HTML-32:
    // shares `amazing_html::is_javascript_mime_type`'s full mimesniff list
    // (this used to hand-roll its own 8-of-16-entry subset, in sync with
    // `tree.rs`'s copy of the same gap).
    let value = amazing_html::strip_ascii_whitespace(raw).to_ascii_lowercase();
    value.is_empty() || amazing_html::is_javascript_mime_type(&value)
}

/// Builds a classic or module `PageScript` for an already-known node, excluding
/// data blocks and classic `nomodule` fallbacks.
/// Shared by `collect_script_text`'s full-tree walk (non-interleaved callers:
/// iframes, and any full-DOM fallback) and HTML-31's interleaved drivers,
/// which already know each script's `NodeIdx` from `ResumableParser`'s own
/// pause/deferred signals and need no tree walk at all.
pub(crate) fn script_source_from_node(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
) -> Option<PageScript> {
    let node_generation = Some(dom.node_generation(node)?);
    let attrs = dom.attrs_of(node).unwrap_or(&[]);
    let script_type = attrs
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("type"))
        .map(|(_, value)| value.as_str());
    let kind = if script_type.is_some_and(|value|
        amazing_html::strip_ascii_whitespace(value).eq_ignore_ascii_case("module")) {
        ScriptKind::Module
    } else if is_classic_javascript_type(script_type) {
        if attrs.iter().any(|(name, _)| name.eq_ignore_ascii_case("nomodule")) { return None; }
        ScriptKind::Classic
    } else { return None; };
    let mut source = String::new();
    for child in dom.children(node) {
        if let amazing_dom::NodeKind::Text { data } | amazing_dom::NodeKind::CDATASection { data } = &dom.get(child).kind {
            source.push_str(data);
        }
    }
    // HTML-32: an SVG `<script>` (unlike its HTML sibling) names its
    // external resource with `href`/`xlink:href`, not `src` -- SVG has no
    // `src` attribute at all. WPT's `unclosed-svg-script.html` ("SVG
    // scripts with self-closing start tag should run") uses a self-closing
    // `<script href=.../>`, which the HTML-31/32 tree-builder fixes now
    // correctly pause/classify (see `tree.rs::in_foreign_content`), but
    // this lookup still missed its source entirely: `src` is absent and a
    // self-closing tag has no inline text either, so `script_source_from_
    // node` returned `None` (silently "not a script") for every such
    // element until now.
    let src = attrs
        .iter()
        .find(|(name, _)| name == "src")
        .map(|(_, value)| value.clone())
        .or_else(|| {
            if dom.namespace(node) != Some(amazing_dom::Namespace::Svg) {
                return None;
            }
            attrs
                .iter()
                .find(|(name, _)| name == "href" || name.eq_ignore_ascii_case("xlink:href"))
                .map(|(_, value)| value.clone())
        });
    if src.is_some() || !source.trim().is_empty() {
        Some(PageScript { node, node_generation, src, body: source, kind })
    } else {
        None
    }
}

pub(crate) fn collect_script_text(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    scripts: &mut Vec<PageScript>,
) {
    if dom.name(node) == Some("script") {
        if let Some(script) = script_source_from_node(dom, node) {
            scripts.push(script);
        }
        return;
    }
    for child in dom.children(node) {
        collect_script_text(dom, child, scripts);
    }
}

/// HTML-31: drives a script-interleaved parse of `html` into `host`'s
/// existing `Dom` (swapped out to a fresh `Dom` for the parser to build into,
/// and swapped back before every script and before this returns -- see
/// `amazing_html::ResumableParser::swap_dom`), calling `run_script` for every
/// classic, parser-blocking script right after tree construction pauses at
/// it. The DOM `run_script` observes/mutates is exactly what has been parsed
/// so far, matching a real browser -- not the finished document, which is
/// the whole point (this is the fix for the architectural gap
/// `dom/nodes/MutationObserver-document.html` exposed: a script's own
/// `document.write` also inserts at the real tokenizer position now, via
/// `RuntimeBindings::register_active_parser` + `document_write`'s check for
/// it, not a DOM splice).
///
/// `defer`/`async`/module scripts never pause construction; this returns
/// them, as `PageScript`s, in document order, once the whole document
/// reaches EOF, for the caller to run after (deferred: in order, before
/// `DOMContentLoaded`; this driver has no real concurrent fetch to race an
/// async script against, so it runs those the same way, immediately after
/// deferred ones, still ahead of `DOMContentLoaded`).
///
/// `run_script` returning `Err` stops the interleaved parse early (the
/// remaining source is left unparsed, like `document.close()` truncating a
/// real one) and this returns that `Err` without any deferred scripts; a
/// caller that wants to keep going after a script error (the lenient
/// real-page path) should swallow its own errors inside `run_script` and
/// always return `Ok(())`.
pub(crate) fn run_interleaved_parse(
    host: &Rc<RefCell<BrowserDomHost>>,
    scheduler: &mut crate::js_scheduler::DriverScheduler,
    html: &str,
    mut run_script: impl FnMut(&mut crate::js_scheduler::DriverScheduler, &PageScript) -> Result<(), String>,
) -> Result<Vec<PageScript>, String> {
    let taken = std::mem::take(&mut host.borrow_mut().dom);
    let parser = Rc::new(RefCell::new(amazing_html::ResumableParser::new(taken)));
    parser.borrow_mut().push_source(html);
    scheduler.register_active_parser(0, parser.clone());
    let mut deferred_events = Vec::new();
    let outcome: Result<(), String> = loop {
        let signal = parser.borrow_mut().run();
        deferred_events.extend(parser.borrow_mut().take_deferred_or_async());
        match signal {
            amazing_html::ParseSignal::Eof => break Ok(()),
            amazing_html::ParseSignal::Script(node) => {
                parser.borrow_mut().swap_dom(&mut host.borrow_mut().dom);
                let script = script_source_from_node(&host.borrow().dom, node);
                let result = scheduler.refresh_parser_browsing_contexts()
                    .map_err(|error| error.describe()).and_then(|_| match script {
                    Some(script) => run_script(scheduler, &script),
                    // Slipped past `tree.rs`'s own classic/module MIME check
                    // some other way (defensive; not expected in practice).
                    None => Ok(()),
                });
                parser.borrow_mut().swap_dom(&mut host.borrow_mut().dom);
                if let Err(error) = result {
                    break Err(error);
                }
            }
        }
    };
    // Every `Script` branch above ends by swapping the real tree back into
    // the parser before checking its `run_script` result, so this always
    // hands the final (or, on an early error, partial) tree back to
    // `host.dom` exactly once, whichever way the loop exited.
    parser.borrow_mut().swap_dom(&mut host.borrow_mut().dom);
    scheduler.unregister_active_parser(0);
    outcome?;
    scheduler.refresh_parser_browsing_contexts().map_err(|error| error.describe())?;
    let dom_ref = host.borrow();
    Ok(deferred_events
        .into_iter()
        .filter_map(|(node, _mode)| script_source_from_node(&dom_ref.dom, node))
        .collect())
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

pub(crate) fn execute_inline_scripts(host: &Rc<RefCell<BrowserDomHost>>, page: &str, html: &str) -> Result<(), String> {
    if let Some(mut scheduler) = prepare_page_scripts(host, page, html, false)? {
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
    html: &str,
    probe: Option<&str>,
    viewport: (f32, f32),
) -> LenientScriptResult {
    let mut errors = Vec::new();
    let mut probe_lines = Vec::new();
    let mut scheduler = match crate::js_scheduler::DriverScheduler::new(
        host.clone(), crate::js_scheduler::JS_CAPACITY) {
        Ok(scheduler) => scheduler,
        Err(error) => {
            errors.push(format!("scheduler init: {}", error.describe()));
            return LenientScriptResult { errors, probe: probe_lines };
        }
    };
    scheduler.set_location_url(page);
    // IA-LIVE-3: give matchMedia the real per-page viewport instead of the
    // interpreter's hardcoded default (runtime_host_integrator_assist.rs).
    scheduler.set_viewport(viewport.0, viewport.1);
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
    // HTML-32: `host`'s DOM starts empty; the interleaved parse below builds
    // it directly from `html`, running each classic parser-blocking script
    // against the DOM as constructed so far (same mechanism as
    // `wpt_harness.rs::execute_page`/`prepare_page_scripts`). Unlike those,
    // this pass is lenient: a script's own failure is collected into
    // `errors`, never propagated as an `Err` from the closure below, since
    // that would abort the *whole* interleaved parse early
    // (`run_interleaved_parse`'s own contract) -- the opposite of what a
    // real-page best-effort pass wants. Budget-checking moves inside the
    // closure for the same reason: once exceeded, every remaining script
    // (blocking or deferred) is skipped by returning `Ok(())` without
    // running it, rather than a `continue` in a `for` loop.
    let script_result = run_interleaved_parse(host, &mut scheduler, html, |scheduler, script| {
        if !budget_exceeded && Instant::now() >= deadline {
            budget_exceeded = true;
            errors.push(format!(
                "script phase exceeded its {budget}s budget; remaining page scripts skipped"
            ));
        }
        if budget_exceeded {
            return Ok(());
        }
        let source = match &script.src {
            Some(src) if script.kind == ScriptKind::Module => match scheduler.load_module_source(&scheduler.page_script_url(page, script)) {
                Ok(source) => source,
                Err(error) => { errors.push(format!("external module {src}: {error}")); return Ok(()); }
            },
            Some(src) => {
                let script_url = scheduler.page_script_url(page, script);
                let fetched = amazing_net::fetch(&script_url, &amazing_net::FetchOptions::default())
                    .map_err(|error| format!("{error}"))
                    .and_then(|response| response.text()
                        .ok_or_else(|| format!("external script {src} was not text")));
                match fetched {
                    Ok(source) => source,
                    Err(error) => { errors.push(format!("external script {src}: {error}")); return Ok(()); }
                }
            }
            None => script.body.clone(),
        };
        let script_url = scheduler.page_script_url(page, script);
        if let Err(error) = scheduler.run_page_script(&script, &source, &script_url) {
            errors.push(format!("page script: {}", error.describe()));
        }
        Ok(())
    });
    // The closure above always returns `Ok(())`, so `run_interleaved_parse`
    // itself cannot fail here; treat the theoretically-impossible `Err` the
    // same as "no deferred/async scripts" rather than unwrapping, since this
    // whole function's contract is to never panic on a real page.
    let deferred = script_result.unwrap_or_default();
    let handlers = {
        let host_ref = host.borrow();
        let mut handlers = Vec::new();
        collect_inline_handlers(&host_ref.dom, amazing_dom::NodeIdx(0), &mut handlers);
        handlers
    };
    for (node, event, body) in handlers {
        if !budget_exceeded && Instant::now() >= deadline {
            budget_exceeded = true;
            errors.push(format!("script phase exceeded its {budget}s budget; remaining inline handlers skipped"));
        }
        if budget_exceeded { continue; }
        if let Err(error) = scheduler.install_inline_handler(node, &event, &body) {
            errors.push(format!("inline {event} handler on node {node}: {}", error.describe()));
        }
    }
    if !budget_exceeded {
        for script in &deferred {
            let source = match &script.src {
                Some(src) if script.kind == ScriptKind::Module => match scheduler.load_module_source(&scheduler.page_script_url(page, script)) {
                    Ok(source) => source,
                    Err(error) => {
                        errors.push(format!("external module {src}: {error}"));
                        let script_url = scheduler.page_script_url(page, script);
                        if let Err(error) = scheduler.finish_module_script(script, None, &script_url, false) { errors.push(error.describe()); }
                        continue;
                    }
                },
                Some(src) => {
                    let script_url = scheduler.page_script_url(page, script);
                    let fetched = amazing_net::fetch(&script_url, &amazing_net::FetchOptions::default())
                        .map_err(|error| format!("{error}"))
                        .and_then(|response| response.text()
                            .ok_or_else(|| format!("external script {src} was not text")));
                    match fetched {
                        Ok(source) => source,
                        Err(error) => { errors.push(format!("external script {src}: {error}")); continue; }
                    }
                }
                None => script.body.clone(),
            };
            let script_url = scheduler.page_script_url(page, script);
            let result = scheduler.run_page_script(script, &source, &script_url);
            if let Err(error) = &result {
                errors.push(format!("page script: {}", error.describe()));
            }
            if script.kind == ScriptKind::Module
                && let Err(error) = scheduler.finish_module_script(script, result.as_ref().err(), &script_url, true) { errors.push(error.describe()); }
        }
    }
    if budget_exceeded {
        errors.push("skipped DOMContentLoaded/load dispatch and the event loop: script phase budget already exceeded".to_owned());
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
/// HTML-32: `host`'s DOM starts empty; this builds it directly from `html`
/// via `run_interleaved_parse`, the same way `wpt_harness.rs::execute_page`
/// does, rather than assuming a caller already parsed it. `force_scheduler`
/// is kept only for API stability at its two call sites -- a scheduler (and
/// the interleaved parse that needs one to run any script against) is now
/// always created, since there is no longer a pre-built DOM to inspect for
/// "are there actually any scripts" before deciding.
pub(crate) fn prepare_page_scripts(
    host: &Rc<RefCell<BrowserDomHost>>,
    page: &str,
    html: &str,
    _force_scheduler: bool,
) -> Result<Option<crate::js_scheduler::DriverScheduler>, String> {
    let mut scheduler = crate::js_scheduler::DriverScheduler::new(host.clone(), crate::js_scheduler::JS_CAPACITY)
        .map_err(|error| error.describe())?;
    scheduler.set_location_url(page);
    let deferred = run_interleaved_parse(host, &mut scheduler, html, |scheduler, script| {
        let source = match &script.src {
            Some(_) if script.kind == ScriptKind::Module => scheduler.load_module_source(&scheduler.page_script_url(page, script))?,
            Some(src) => {
                let script_url = scheduler.page_script_url(page, script);
                let response =
                    amazing_net::fetch(&script_url, &amazing_net::FetchOptions::default())
                        .map_err(|error| format!("external script {src}: {error}"))?;
                response
                    .text()
                    .ok_or_else(|| format!("external script {src} was not text"))?
            }
            None => script.body.clone(),
        };
        let script_url = scheduler.page_script_url(page, script);
        scheduler.run_page_script(&script, &source, &script_url)
            .map(|_| ())
            .map_err(|error| format!("page script failed: {}", error.describe()))
    })?;
    let handlers = {
        let host_ref = host.borrow();
        let mut handlers = Vec::new();
        collect_inline_handlers(&host_ref.dom, amazing_dom::NodeIdx(0), &mut handlers);
        handlers
    };
    for (node, event, body) in handlers {
        scheduler.install_inline_handler(node, &event, &body)
            .map_err(|error| format!("inline handler failed: {}", error.describe()))?;
    }
    for script in &deferred {
        let source = match &script.src {
            Some(_) if script.kind == ScriptKind::Module => scheduler.load_module_source(&scheduler.page_script_url(page, script))?,
            Some(src) => {
                let script_url = scheduler.page_script_url(page, script);
                let response =
                    amazing_net::fetch(&script_url, &amazing_net::FetchOptions::default())
                        .map_err(|error| format!("external script {src}: {error}"))?;
                response
                    .text()
                    .ok_or_else(|| format!("external script {src} was not text"))?
            }
            None => script.body.clone(),
        };
        let script_url = scheduler.page_script_url(page, script);
        scheduler.run_page_script(script, &source, &script_url)
            .map_err(|error| format!("page script failed: {}", error.describe()))?;
        if script.kind == ScriptKind::Module {
            scheduler.finish_module_script(script, None, &script_url, true)
                .map_err(|error| format!("module load handler failed: {}", error.describe()))?;
        }
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
    let mut scheduler = match crate::js_scheduler::DriverScheduler::new(host.clone(), crate::js_scheduler::JS_CAPACITY) {
        Ok(scheduler) => scheduler,
        Err(error) => { eprintln!("amazing run: {}", error.describe()); return ExitCode::FAILURE; }
    };
    let entry = if let Some(path) = path {
        let url = crate::runtime_binding_host::modules::source_url(path);
        scheduler.set_location_url(&url);
        let script = PageScript { kind: ScriptKind::Classic, node: amazing_dom::NodeIdx(0), node_generation: host.borrow().dom.node_generation(amazing_dom::NodeIdx(0)), src: Some(url.clone()), body: String::new() };
        scheduler.run_page_script(&script, &source, &url)
    } else { scheduler.run_script(&source) };
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
        assert!(is_classic_javascript_type(Some("TEXT/JavaScript")));
        assert!(is_classic_javascript_type(Some("application/javascript")));
        // HTML-32: these 8 were missing from the matched set entirely
        // (only 8 of mimesniff's 16-entry JavaScript MIME type list were
        // recognized), caught while triaging WPT's
        // `script-type-and-language-js.html`.
        for data in [
            "application/x-ecmascript",
            "APPLICATION/X-ECMASCRIPT",
            "text/x-ecmascript",
            "text/javascript1.0",
            "text/javascript1.5",
        ] {
            assert!(is_classic_javascript_type(Some(data)), "{data} must run as classic JS");
        }
        for data in [
            "module",
            "application/ld+json",
            "application/json",
            "text/template",
            "importmap",
            "text/html",
            "speculationrules",
            // HTML-31: per spec the "script block's type string" is matched
            // verbatim against the legacy JS type list -- no `;parameter`
            // stripping (WPT's script-type-and-language-with-params.html:
            // "A script with a charset param in its type should not run")
            // and no non-ASCII whitespace stripping (script-type-
            // whitespace.html covers several Unicode blank codepoints, one
            // of which is exercised here).
            "TEXT/JavaScript; charset=utf-8",
            "text/javascript\u{00A0}",
            // Only 1.0 through 1.5 are recognized legacy versions (WPT's
            // `script-type-and-language-js.html` expects 1.6-1.9 to be
            // rejected even with the otherwise-valid "text/" prefix).
            "text/javascript1.6",
            "javascript1.0",
        ] {
            assert!(
                !is_classic_javascript_type(Some(data)),
                "{data} must not run as classic JS"
            );
        }
    }
}
