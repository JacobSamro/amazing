//! `amazing axtree <page.html|url> [viewport_width]` — driver adoption of the
//! accessibility tree seam (`amazing-a11y`, see `.agents/a11y/questions.md`).
//!
//! `amazing_a11y::build(&dom, &styled, &layout)` takes the driver's own DOM,
//! computed styles and layout tree, so this command fetches/parses a page the
//! same way `render`/`render-page` do, resolves its author stylesheets
//! (including `<link rel=stylesheet>`, via `page::collect_author_css`) and
//! computes style + layout, then builds and prints the tree with
//! `SchemaSnapshot::dump()` (a formatter the a11y crate already ships "for
//! tests and the CLI").
//!
//! Known limitation: layout uses `amazing_layout::NoReplacedElements` and the
//! fallback text system, same as the a11y crate's own `build_document`
//! convenience entry — an `<img>`/`<video>`/`<canvas>` gets its CSS-box size
//! but not decoded intrinsic dimensions or real glyph metrics. Wiring the
//! driver's real replaced-content/text providers through here is a follow-up
//! once this seam is exercised.
use std::process::ExitCode;

use crate::document::{DocumentKind, parse_document_bytes};

pub(crate) fn cmd_axtree(args: &[String]) -> ExitCode {
    let page = match args.first() {
        Some(page) => page.as_str(),
        None => {
            eprintln!("usage: amazing axtree <page.html|url> [viewport_width]");
            return ExitCode::FAILURE;
        }
    };
    let width: f32 = args.get(1).and_then(|w| w.parse().ok()).unwrap_or(1024.0);

    let remote = page.starts_with("http://") || page.starts_with("https://");
    let (base_url, html, content_type) = if remote {
        match super::real_pages::fetch_document(page) {
            Ok(document) => document,
            Err(error) => {
                eprintln!("amazing axtree: fetch: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        match super::parse::read_bytes(Some(page), "axtree") {
            Ok(bytes) => (page.to_owned(), bytes, None),
            Err(code) => return code,
        }
    };

    let document = match parse_document_bytes(&html, &base_url, content_type.as_deref()) {
        Ok(document) => document,
        Err(error) => {
            eprintln!("amazing axtree: {error}");
            return error.exit_code();
        }
    };
    if document.kind != DocumentKind::Html {
        eprintln!("amazing axtree: XML documents are not supported yet");
        return ExitCode::from(3);
    }
    let dom = document.dom;

    let mut sources = Vec::new();
    if let Err(error) = crate::page::collect_author_css(&dom, amazing_dom::NodeIdx(0), &base_url, &mut sources) {
        eprintln!("amazing axtree: stylesheet fetch: {error}");
    }
    let sheets: Vec<amazing_css::Stylesheet> = sources.into_iter().map(|(_, _, sheet)| sheet).collect();
    let styled = amazing_css::compute_with_environment(
        &dom,
        &sheets,
        &amazing_css::MediaEnvironment { viewport_width_px: width.max(1.0), ..amazing_css::MediaEnvironment::default() },
    );
    let text = amazing_text::FallbackTextSystem::new();
    let layout = amazing_layout::layout_styled_document(
        &dom,
        &styled,
        &text,
        &amazing_layout::NoReplacedElements,
        width,
        None,
    );
    let tree = amazing_a11y::build(&dom, &styled, &layout);
    let snapshot = tree.snapshot();
    print!("{}", snapshot.dump());
    println!("nodes\t{}", snapshot.nodes.len());
    ExitCode::SUCCESS
}
