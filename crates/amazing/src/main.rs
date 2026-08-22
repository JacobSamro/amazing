//! amazing — browser driver binary.
//!
//! Phase 0 artifact: builds and runs, prints the component pipeline status.
//! Each phase replaces one line of "not started" with a real capability
//! plus its demo subcommand (`parse` in Phase 1, `style` in Phase 2).

use std::cell::RefCell;
use std::io::Read;
use std::process::ExitCode;
use std::rc::Rc;

use amazing_js::DomHost;

mod gui;

const PIPELINE: &[(&str, &str)] = &[
    ("amazing-common", "byte streams + text utils"),
    (
        "amazing-html",
        "Phase 1: WHATWG tokenizer/tree builder — `parse` subcommand",
    ),
    (
        "amazing-css",
        "Phase 2: syntax 3 + selectors + cascade — `style` subcommand",
    ),
    (
        "amazing-layout",
        "Phase 3: block/inline FC layout — `layout` subcommand",
    ),
    (
        "amazing-paint",
        "Phase 4: software rasterizer — `render` subcommand",
    ),
    (
        "amazing-net",
        "Phase 5: HTTP/1.1 fetcher (library; TLS deferred)",
    ),
    (
        "amazing-js",
        "Phase 6: ECMA-262 parser + tree-walking interpreter — `run` subcommand",
    ),
    (
        "amazing-dom-js",
        "Phase 7: live DOM/JS bridge — `domrun` subcommand",
    ),
    (
        "amazing-gui",
        "Phase 8: native Windows shell — `gui` subcommand",
    ),
];

struct BrowserDomHost {
    dom: amazing_dom::Dom,
}

impl BrowserDomHost {
    fn text_content(&self, node: amazing_dom::NodeIdx, out: &mut String) {
        if let amazing_dom::NodeKind::Text { data } = &self.dom.get(node).kind {
            out.push_str(data);
        }
        for child in self.dom.children(node) {
            self.text_content(child, out);
        }
    }

    fn matches_selector(&self, node: amazing_dom::NodeIdx, selector: &str) -> bool {
        let Some(name) = self.dom.name(node) else { return false };
        let selector = selector.trim();
        if selector.is_empty() || selector.contains(char::is_whitespace) { return false }
        let (tag, id, class) = if let Some(rest) = selector.strip_prefix('#') {
            (None, Some(rest), None)
        } else if let Some(rest) = selector.strip_prefix('.') {
            (None, None, Some(rest))
        } else if let Some((tag, rest)) = selector.split_once('#') {
            (Some(tag), Some(rest), None)
        } else if let Some((tag, rest)) = selector.split_once('.') {
            (Some(tag), None, Some(rest))
        } else {
            (Some(selector), None, None)
        };
        if tag.is_some_and(|tag| !tag.eq_ignore_ascii_case(name)) { return false }
        let attrs = self.dom.attrs_of(node).unwrap_or(&[]);
        if let Some(want) = id {
            if attrs.iter().find(|(k, _)| k == "id").map(|(_, v)| v.as_str()) != Some(want) {
                return false;
            }
        }
        if let Some(want) = class {
            let classes = attrs.iter().find(|(k, _)| k == "class").map(|(_, v)| v.as_str()).unwrap_or("");
            if !classes.split_ascii_whitespace().any(|item| item == want) { return false }
        }
        true
    }

    fn find(&self, node: amazing_dom::NodeIdx, selector: &str) -> Option<u32> {
        if self.matches_selector(node, selector) { return Some(node.0) }
        for child in self.dom.children(node) {
            if let Some(found) = self.find(child, selector) { return Some(found) }
        }
        None
    }

    fn css_property_name(name: &str) -> String {
        let mut out = String::new();
        for ch in name.chars() {
            if ch.is_ascii_uppercase() {
                out.push('-');
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn style_declarations(&self, node: amazing_dom::NodeIdx) -> Vec<(String, String)> {
        let raw = self
            .dom
            .attrs_of(node)
            .and_then(|attrs| attrs.iter().find(|(key, _)| key == "style"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        raw.split(';')
            .filter_map(|declaration| {
                let (name, value) = declaration.split_once(':')?;
                let name = name.trim().to_ascii_lowercase();
                if name.is_empty() { return None }
                Some((name, value.trim().to_string()))
            })
            .collect()
    }

    fn write_style_declarations(
        &mut self,
        node: amazing_dom::NodeIdx,
        declarations: &[(String, String)],
    ) -> Result<(), String> {
        let serialized = declarations
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, value)| format!("{name}: {value};"))
            .collect::<Vec<_>>()
            .join(" ");
        self.set_attribute(node.0, "style", &serialized)
    }
}

impl amazing_js::DomHost for BrowserDomHost {
    fn query_selector(&mut self, selector: &str) -> Option<u32> {
        self.find(amazing_dom::NodeIdx(0), selector)
    }

    fn get_text_content(&self, node: u32) -> Option<String> {
        let idx = amazing_dom::NodeIdx(node);
        if idx.0 as usize >= self.dom.node_count() { return None }
        let mut text = String::new();
        self.text_content(idx, &mut text);
        Some(text)
    }

    fn set_text_content(&mut self, node: u32, text: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if idx.0 as usize >= self.dom.node_count() || self.dom.name(idx).is_none() {
            return Err("textContent requires an element".into());
        }
        let children: Vec<_> = self.dom.children(idx).collect();
        for child in children { self.dom.detach(child); }
        self.dom.create_text(idx, text);
        Ok(())
    }

    fn get_attribute(&self, node: u32, name: &str) -> Option<Option<String>> {
        if node as usize >= self.dom.node_count() { return None }
        let attrs = self.dom.attrs_of(amazing_dom::NodeIdx(node))?;
        Some(attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone()))
    }

    fn set_attribute(&mut self, node: u32, name: &str, value: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() { return Err("unknown DOM node".into()) }
        let mut attrs = self.dom.attrs_of(idx).ok_or_else(|| "attribute requires an element".to_string())?.to_vec();
        if let Some((_, current)) = attrs.iter_mut().find(|(key, _)| key == name) {
            *current = value.to_string();
        } else {
            attrs.push((name.to_string(), value.to_string()));
        }
        self.dom.set_attrs(idx, attrs);
        Ok(())
    }

    fn create_element(&mut self, name: &str) -> Result<u32, String> {
        Ok(self.dom.alloc_unattached(amazing_dom::NodeKind::Element {
            name: name.to_ascii_lowercase(), attrs: Vec::new(),
        }).0)
    }

    fn append_child(&mut self, parent: u32, child: u32) -> Result<(), String> {
        let parent = amazing_dom::NodeIdx(parent);
        let child = amazing_dom::NodeIdx(child);
        if parent.0 as usize >= self.dom.node_count() || child.0 as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        if self.dom.name(parent).is_none() { return Err("append target is not an element".into()) }
        let mut ancestor = Some(parent);
        while let Some(current) = ancestor {
            if current == child { return Err("cannot append an ancestor into its descendant".into()) }
            ancestor = self.dom.get(current).parent;
        }
        if self.dom.get(child).parent.is_some() { self.dom.detach(child); }
        self.dom.attach_last(parent, child);
        Ok(())
    }

    fn append_text(&mut self, parent: u32, text: &str) -> Result<(), String> {
        let parent = amazing_dom::NodeIdx(parent);
        if parent.0 as usize >= self.dom.node_count() { return Err("unknown DOM node".into()) }
        if self.dom.name(parent).is_none() { return Err("append target is not an element".into()) }
        self.dom.create_text(parent, text);
        Ok(())
    }

    fn get_style(&self, node: u32, property: &str) -> Option<String> {
        if node as usize >= self.dom.node_count() || self.dom.name(amazing_dom::NodeIdx(node)).is_none() {
            return None;
        }
        let property = BrowserDomHost::css_property_name(property).to_ascii_lowercase();
        if property == "csstext" {
            return self
                .dom
                .attrs_of(amazing_dom::NodeIdx(node))
                .and_then(|attrs| attrs.iter().find(|(key, _)| key == "style"))
                .map(|(_, value)| value.clone())
                .or_else(|| Some(String::new()));
        }
        self.style_declarations(amazing_dom::NodeIdx(node))
            .into_iter()
            .find(|(name, _)| name == &property)
            .map(|(_, value)| value)
    }

    fn set_style(&mut self, node: u32, property: &str, value: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() || self.dom.name(idx).is_none() {
            return Err("style requires an element".into());
        }
        if property.eq_ignore_ascii_case("cssText") {
            return self.set_attribute(node, "style", value);
        }
        let property = BrowserDomHost::css_property_name(property).to_ascii_lowercase();
        let mut declarations = self.style_declarations(idx);
        if let Some((_, current)) = declarations.iter_mut().find(|(name, _)| name == &property) {
            *current = value.trim().to_string();
        } else {
            declarations.push((property, value.trim().to_string()));
        }
        self.write_style_declarations(idx, &declarations)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("version") | None => {
            println!("amazing v{}", env!("CARGO_PKG_VERSION"));
            println!("zero-dependency browser engine — pipeline status:");
            for (crate_name, status) in PIPELINE {
                println!("  {crate_name:<15} {status}");
            }
            println!("dom: arena skeleton present (u32 indices, no Rc<RefCell>)");
            ExitCode::SUCCESS
        }
        Some("parse") => cmd_parse(args.get(2).map(String::as_str)),
        Some("style") => cmd_style(&args[2..]),
        Some("layout") => cmd_layout(&args[2..]),
        Some("render") => cmd_render(&args[2..]),
        Some("fetch") => cmd_fetch(&args[2..]),
        Some("run") => cmd_run(args.get(2).map(String::as_str)),
        Some("domrun") => cmd_domrun(&args[2..]),
        Some("gui") => gui::run(args.get(2).map(String::as_str)),
        Some(other) => {
            eprintln!("unknown command: {other}");
            eprintln!("usage: amazing [version|parse [file]|style <page.html> [extra.css…]|layout <page.html> [viewport_width]|render <page.html> <out.bmp> [width]|run [script.js]|domrun <page.html> <script.js>|gui [url]]");
            ExitCode::FAILURE
        }
    }
}

/// Reads a file (or stdin when `path` is None) as lossy UTF-8.
fn read_source(path: Option<&str>, what: &str) -> Result<String, ExitCode> {
    let mut bytes = Vec::new();
    match path {
        Some(p) => match std::fs::File::open(p) {
            Ok(mut f) => {
                if let Err(e) = f.read_to_end(&mut bytes) {
                    eprintln!("amazing {what}: cannot read {p}: {e}");
                    return Err(ExitCode::FAILURE);
                }
            }
            Err(e) => {
                eprintln!("amazing {what}: cannot open {p}: {e}");
                return Err(ExitCode::FAILURE);
            }
        },
        None => {
            if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                eprintln!("amazing {what}: cannot read stdin: {e}");
                return Err(ExitCode::FAILURE);
            }
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Phase 1 demo artifact: parse HTML with the WHATWG tokenizer + tree
/// builder, dump the DOM tree to stdout.
///
/// Bytes are decoded as lossy UTF-8 (documented Phase 1 deviation).
fn cmd_parse(path: Option<&str>) -> ExitCode {
    match read_source(path, "parse") {
        Ok(src) => {
            let dom = amazing_html::parse(&src);
            print!("{}", dom.dump(amazing_dom::NodeIdx(0)));
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Phase 2 demo artifact: compute styles for an HTML page.
///
/// Stylesheets come from every `<style>` element in document order plus any
/// extra CSS files given after the page path (`<link rel=stylesheet>` needs
/// fetching, so it waits for Phase 5).
///
/// Output prints each element in tree order as `<tag style="prop:value; …">`
/// containing only the properties whose computed value differs from the
/// parent element's (the readable delta view from `StyledDom`).
fn cmd_style(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing style <page.html> [extra.css…]");
            return ExitCode::FAILURE;
        }
    };

    let html = match read_source(Some(html_path), "style") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let dom = amazing_html::parse(&html);

    // Gather author stylesheets: inline <style> elements first.
    let mut css = String::new();
    collect_style_text(&dom, amazing_dom::NodeIdx(0), &mut css);
    for extra in &args[1..] {
        match read_source(Some(extra), "style") {
            Ok(s) => {
                css.push('\n');
                css.push_str(&s);
            }
            Err(code) => return code,
        }
    }

    let sheet = amazing_css::parse_stylesheet(&css);
    let styled = amazing_css::compute(&dom, &[sheet]);

    print_styled(&dom, amazing_dom::NodeIdx(0), 0, &styled);
    ExitCode::SUCCESS
}

fn collect_style_text(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, out: &mut String) {
    if dom.name(node) == Some("style") {
        for child in dom.children(node) {
            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                out.push_str(data);
                out.push('\n');
            }
        }
        return;
    }
    for child in dom.children(node) {
        collect_style_text(dom, child, out);
    }
}

fn print_styled(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    depth: usize,
    styled: &amazing_css::StyledDom,
) {
    // Only elements print; document/text/comment nodes pass through.
    let is_element = dom.name(node).is_some();
    if let Some(name) = dom.name(node) {
        for _ in 0..depth {
            print!("  ");
        }
        match styled.delta_from_parent(node, dom.get(node).parent) {
            delta if delta.is_empty() => println!("<{name}>"),
            delta => {
                let decls: Vec<String> = delta.iter().map(|(k, v)| format!("{k}:{v}")).collect();
                println!("<{name} style=\"{}\">", decls.join("; "));
            }
        }
    }
    // Unprinted nodes (document/text/comments) don't advance the indent.
    let child_depth = if is_element { depth + 1 } else { depth };
    for child in dom.children(node) {
        print_styled(dom, child, child_depth, styled);
    }
}

/// Phase 3 demo artifact: lay out an HTML page and print the layout tree —
/// every box with its absolute border/content rectangles, plus line boxes
/// with placed word fragments.
///
/// Viewport width comes from the second argument (default 800px).
/// Stylesheets: inline `<style>` elements + extra CSS file arguments, same
/// as `amazing style`. Text measurement uses the built-in fallback font
/// metrics (a real TTF can be plugged in later phases).
fn cmd_layout(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing layout <page.html> [viewport_width]");
            return ExitCode::FAILURE;
        }
    };
    let width: f32 = args.get(1).and_then(|w| w.parse().ok()).unwrap_or(800.0);

    let html = match read_source(Some(html_path), "layout") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let dom = amazing_html::parse(&html);

    // Author CSS: <style> elements + extra files (same policy as `style`).
    let mut css = String::new();
    collect_style_text(&dom, amazing_dom::NodeIdx(0), &mut css);
    for extra in args.get(2..).unwrap_or(&[]) {
        match read_source(Some(extra), "layout") {
            Ok(s) => {
                css.push('\n');
                css.push_str(&s);
            }
            Err(code) => return code,
        }
    }

    let fonts = amazing_layout::font::FontSource::fallback();
    let root = amazing_layout::layout_document(&html, &css, width, &fonts);
    print!("{}", amazing_layout::dump_layout(&dom, &root));
    ExitCode::SUCCESS
}

/// Phase 4 demo: `amazing render <page.html> <out.bmp> [viewport_width]`
/// (plus optional extra CSS files after the width). Lays out the page,
/// rasterizes it into the software framebuffer, and writes a 24-bit BMP
/// alongside a PPM twin (`<out>.ppm`).
fn cmd_render(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing render <page.html> <out.bmp> [viewport_width] [extra.css…]");
            return ExitCode::FAILURE;
        }
    };
    let out_path = args.get(1).map(String::as_str).unwrap_or("render.bmp");
    let width: f32 = args.get(2).and_then(|w| w.parse().ok()).unwrap_or(800.0);

    let html = match read_source(Some(html_path), "render") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let dom = amazing_html::parse(&html);

    let mut css = String::new();
    collect_style_text(&dom, amazing_dom::NodeIdx(0), &mut css);
    for extra in args.get(3..).unwrap_or(&[]) {
        match read_source(Some(extra), "render") {
            Ok(s) => {
                css.push('\n');
                css.push_str(&s);
            }
            Err(code) => return code,
        }
    }

    let fonts = amazing_layout::font::FontSource::fallback();
    let root = amazing_layout::layout_document(&html, &css, width, &fonts);
    // Page height = root margin-box bottom edge (what layout_root returns),
    // clamped so a runaway document cannot exhaust memory.
    let page_h = (root.border_box.y + root.border_box.h + root.margin.bottom.unwrap_or(0.0))
        .ceil()
        .clamp(1.0, 8192.0) as u32;
    let surface = amazing_paint::paint_document(&root, width.max(1.0) as u32, page_h, &fonts);

    if let Err(e) = surface.save_bmp(out_path) {
        eprintln!("amazing render: cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    let ppm_path = match out_path.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.ppm"),
        None => format!("{out_path}.ppm"),
    };
    if let Err(e) = surface.save_ppm(&ppm_path) {
        eprintln!("amazing render: cannot write {ppm_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "rendered {}x{} px → {out_path} (+{ppm_path})",
        surface.width, surface.height
    );
    ExitCode::SUCCESS
}

/// Phase 5 demo: `amazing fetch <url|file|path> [-i]`. Performs an HTTP/1.1
/// GET (or reads a file:// / bare path), prints status + headers + body.
/// `-i` includes the response headers.
fn cmd_fetch(args: &[String]) -> ExitCode {
    let target = match args.first() {
        Some(p) if !p.starts_with('-') => p,
        _ => {
            eprintln!("usage: amazing fetch <url|file> [-i]");
            return ExitCode::FAILURE;
        }
    };
    let show_headers = args.iter().any(|a| a == "-i");

    match amazing_net::fetch(target, &amazing_net::FetchOptions::default()) {
        Ok(resp) => {
            println!("{} {} ({})", resp.status, resp.reason, resp.url);
            if show_headers {
                for (name, value) in resp.headers.iter() {
                    println!("{name}: {value}");
                }
                println!();
            }
            print!("{}", resp.text().unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("amazing fetch: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 6 demo: execute one classic JavaScript script and print captured
/// `console.log` lines followed by the script completion value.
fn cmd_run(path: Option<&str>) -> ExitCode {
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
fn cmd_domrun(args: &[String]) -> ExitCode {
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
    let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(&html) }));
    let mut js = amazing_js::Interpreter::new();
    js.set_dom_host(host.clone());
    match js.run_script(&script) {
        Ok(value) => {
            for line in js.output.borrow().iter() { println!("{line}"); }
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

fn format_js_error(value: &amazing_js::value::JsValue) -> String {
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
