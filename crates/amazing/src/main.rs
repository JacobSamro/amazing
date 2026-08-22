//! amazing — browser driver binary.
//!
//! Phase 0 artifact: builds and runs, prints the component pipeline status.
//! Each phase replaces one line of "not started" with a real capability
//! plus its demo subcommand (`parse` in Phase 1, `style` in Phase 2).

use std::io::Read;
use std::process::ExitCode;

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
    ("amazing-paint", "Phase 4: software rasterizer — not started"),
    ("amazing-net", "Phase 5: HTTP/1.1 — not started"),
    ("amazing-js", "Phase 6: ECMA-262 interpreter — not started"),
];

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
        },
        Some("parse") => cmd_parse(args.get(2).map(String::as_str)),
        Some("style") => cmd_style(&args[2..]),
        Some("layout") => cmd_layout(&args[2..]),
        Some(other) => {
            eprintln!("unknown command: {other}");
            eprintln!("usage: amazing [version|parse [file]|style <page.html> [extra.css…]|layout <page.html> [viewport_width]]");
            ExitCode::FAILURE
        },
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
            },
            Err(e) => {
                eprintln!("amazing {what}: cannot open {p}: {e}");
                return Err(ExitCode::FAILURE);
            },
        },
        None => {
            if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                eprintln!("amazing {what}: cannot read stdin: {e}");
                return Err(ExitCode::FAILURE);
            }
        },
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
        },
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
        },
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
            },
            Err(code) => return code,
        }
    }

    let sheet = amazing_css::parse_stylesheet(&css);
    let styled = amazing_css::compute(&dom, &[sheet]);

    print_styled(&dom, amazing_dom::NodeIdx(0), 0, &styled);
    ExitCode::SUCCESS
}

fn collect_style_text(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, out: &mut String) {    if dom.name(node) == Some("style") {
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
                let decls: Vec<String> =
                    delta.iter().map(|(k, v)| format!("{k}:{v}")).collect();
                println!("<{name} style=\"{}\">", decls.join("; "));
            },
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
        },
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
            },
            Err(code) => return code,
        }
    }

    let fonts = amazing_layout::font::FontSource::fallback();
    let root = amazing_layout::layout_document(&html, &css, width, &fonts);
    print!("{}", amazing_layout::dump_layout(&dom, &root));
    ExitCode::SUCCESS
}
