//! Phase 3 gate + WPT css/CSS2 tracking baseline.
//!
//! The real Web Platform Tests suite cannot run here yet (no floats,
//! tables, abspos), so this harness runs a curated subset of CSS2.1
//! geometry cases modeled on what css/CSS2 checks — each one drives the
//! full pipeline (HTML → cascade → box tree → block/inline layout) and
//! asserts concrete pixel rectangles. The pass rate printed at the end is
//! the Phase 3 tracking number; it must be interpreted against the subset
//! declared below, not the full WPT corpus.
//!
//! Subset covered: block stacking, §10.3.3 constraint solving incl.
//! auto-margin centering, box-model arithmetic, sibling margin collapse,
//! em/%/keyword units, auto heights, specified heights, inline wrapping,
//! `<br>`, anonymous blocks, display:none, line-height forms, UA-sheet
//! defaults.

use amazing_css::{compute, parse_stylesheet};
use amazing_dom::Dom;
use amazing_layout::font::FontSource;
use amazing_layout::{box_tree::build_layout_tree, block::layout_root, Flow, LayoutBox};

/// Full pipeline for a page; returns the DOM (for tag lookups) and the
/// laid-out root box.
fn lay(html: &str, css: &str, width: f32) -> (Dom, LayoutBox) {
    let dom = amazing_html::parse(html);
    let sheet = parse_stylesheet(css);
    let styled = compute(&dom, &[sheet]);
    let fonts = FontSource::fallback();
    let mut root = build_layout_tree(&dom, &styled, &fonts);
    layout_root(&mut root, width, &fonts);
    (dom, root)
}

/// Depth-first search for the first descendant box with the given tag name.
fn find<'a>(b: &'a LayoutBox, dom: &Dom, tag: &str) -> Option<&'a LayoutBox> {
    if b.node.and_then(|n| dom.name(n)) == Some(tag) {
        return Some(b);
    }
    match &b.flow {
        Flow::Blocks(kids) => kids.iter().find_map(|k| find(k, dom, tag)),
        Flow::Inlines(_) | Flow::Empty => None,
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn wpt_css2_subset_report() {
    // Each entry: (name, result). A result of Err carries the wrong
    // geometry so the report is actionable.
    let mut results: Vec<(&'static str, Result<(), String>)> = Vec::new();

    results.push((
        "blocks stack vertically",
        (|| {
            let (_, root) = lay(
                "<body><div style=\"height:100px\"></div><div style=\"height:60px\"></div></body>",
                "",
                800.0,
            );
            let Flow::Blocks(kids) = &root.flow else { return Err("root not block container".into()) };
            let (a, b) = (&kids[0], &kids[1]);
            if !close(a.border_box.y, 8.0) {
                return Err(format!("first div y={} want 8", a.border_box.y));
            }
            if !close(b.border_box.y, 108.0) {
                return Err(format!("second div y={} want 108", b.border_box.y));
            }
            Ok(())
        })(),
    ));

    results.push((
        "width:400px + margin:auto centers in 800px viewport",
        (|| {
            let (_, root) = lay(
                "<body><div style=\"width:400px;height:10px;margin:0 auto\"></div></body>",
                "",
                800.0,
            );
            let Flow::Blocks(kids) = &root.flow else { return Err("no children".into()) };
            let x = kids[0].border_box.x;
            if !close(x, 200.0) {
                return Err(format!("x={x} want 200"));
            }
            Ok(())
        })(),
    ));

    results.push((
        "box model arithmetic: border box = content + padding + border",
        (|| {
            let (_, root) = lay(
                "<body><div style=\"width:100px;height:20px;padding:10px;border:5px solid black\"></div></body>",
                "",
                800.0,
            );
            let Flow::Blocks(kids) = &root.flow else { return Err("no children".into()) };
            let d = &kids[0];
            let ok = close(d.content_box.w, 100.0)
                && close(d.border_box.w, 130.0)
                && close(d.border_box.h, 50.0);
            if !ok {
                return Err(format!("content={:?} border={:?}", d.content_box, d.border_box));
            }
            Ok(())
        })(),
    ));

    results.push((
        "sibling margins collapse to the max (30 bottom / 20 top → 30)",
        (|| {
            let (_, root) = lay("<body><p style=\"margin-bottom:30px\">a</p><p style=\"margin-top:20px\">b</p></body>", "", 800.0);
            let Flow::Blocks(kids) = &root.flow else { return Err("no children".into()) };
            let gap = kids[1].border_box.y - (kids[0].border_box.y + kids[0].border_box.h);
            // First child's top margin is NOT collapsed into body here (its
            // own 0 top margin vs body's 8 doesn't matter); expected gap 30.
            if !close(gap, 30.0) {
                return Err(format!("gap={gap} want 30"));
            }
            Ok(())
        })(),
    ));

    results.push((
        "em lengths use the element's own font size (2em @ 20px = 40)",
        (|| {
            let (_, root) = lay(
                "<body><div style=\"font-size:20px;width:100px;margin-left:2em\"></div></body>",
                "",
                800.0,
            );
            let Flow::Blocks(kids) = &root.flow else { return Err("no children".into()) };
            let x = kids[0].border_box.x;
            // body margin 8 + 40
            if !close(x, 48.0) {
                return Err(format!("x={x} want 48"));
            }
            Ok(())
        })(),
    ));

    results.push((
        "percentage width resolves against the containing block (50% of 784)",
        (|| {
            let (_, root) = lay("<body><div style=\"width:50%;height:10px\"></div></body>", "", 800.0);
            let Flow::Blocks(kids) = &root.flow else { return Err("no children".into()) };
            let w = kids[0].border_box.w;
            // Containing block = body's content box: 800 − 8 − 8 = 784.
            if !close(w, 392.0) {
                return Err(format!("w={w} want 392"));
            }
            Ok(())
        })(),
    ));

    results.push((
        "font-size keyword ladder (x-large → 24px) cascades to text",
        (|| {
            let (dom, root) = lay(
                "<body><p style=\"font-size:x-large\">hi</p></body>",
                "",
                800.0,
            );
            let p = find(&root, &dom, "p").expect("p box");
            if !close(p.font_size, 24.0) {
                return Err(format!("font_size={} want 24", p.font_size));
            }
            Ok(())
        })(),
    ));

    results.push((
        "long inline text wraps within the content box",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"width:120px\">one two three four five six seven eight nine ten eleven twelve</div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").expect("div box");
            if d.lines.len() < 3 {
                return Err(format!("only {} lines for 120px-wide box", d.lines.len()));
            }
            for line in &d.lines {
                for w in &line.words {
                    if w.x < d.content_box.x - 0.01 {
                        return Err(format!("word {:?} left of content box", w.text));
                    }
                }
            }
            Ok(())
        })(),
    ));

    results.push((
        "<br> produces two lines",
        (|| {
            let (dom, root) = lay("<body><p>a<br>b</p></body>", "", 800.0);
            let p = find(&root, &dom, "p").expect("p box");
            if p.lines.len() != 2 {
                return Err(format!("{} lines want 2", p.lines.len()));
            }
            Ok(())
        })(),
    ));

    results.push((
        "mixed block/inline content creates anonymous blocks",
        (|| {
            let (dom, root) = lay("<body><p>a</p>bare<div>c</div></body>", "", 800.0);
            let Flow::Blocks(kids) = &root.flow else { return Err("no children".into()) };
            let tags: Vec<String> =
                kids.iter().map(|k| k.node.and_then(|n| dom.name(n)).unwrap_or("#anon").to_string()).collect();
            if tags != vec!["p", "#anon", "div"] {
                return Err(format!("tags {tags:?} want [p,#anon,div]"));
            }
            Ok(())
        })(),
    ));

    results.push((
        "display:none subtree generates no boxes",
        (|| {
            let (dom, root) = lay(
                "<body><p style=\"display:none\"><span>x</span></p><p>visible</p></body>",
                "",
                800.0,
            );
            if find(&root, &dom, "span").is_some() {
                return Err("display:none span still laid out".into());
            }
            Ok(())
        })(),
    ));

    results.push((
        "auto height grows to fit stacked children",
        (|| {
            let (dom, root) = lay(
                "<body><div><div style=\"height:40px\"></div><div style=\"height:25px\"></div></div></body>",
                "",
                800.0,
            );
            let outer = find(&root, &dom, "div").unwrap(); // first div found is the OUTER one
            if !close(outer.border_box.h, 65.0) {
                return Err(format!("h={} want 65", outer.border_box.h));
            }
            Ok(())
        })(),
    ));

    results.push((
        "specified height overrides content extent",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"height:33px\"><div style=\"height:90px\"></div></div></body>",
                "",
                800.0,
            );
            let outer = find(&root, &dom, "div").unwrap();
            if !close(outer.content_box.h, 33.0) {
                return Err(format!("h={} want 33 (child overflows)", outer.content_box.h));
            }
            Ok(())
        })(),
    ));

    results.push((
        "unitless line-height multiplies font size (2 × 16 = 32/line)",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"line-height:2\">a<br>b<br>c</div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").expect("div");
            if d.lines.len() != 3 {
                return Err(format!("{} lines want 3", d.lines.len()));
            }
            if !close(d.lines[1].y_top - d.lines[0].y_top, 32.0)
                || !close(d.lines[2].y_top - d.lines[1].y_top, 32.0)
            {
                return Err("line pitch ≠ 32".into());
            }
            Ok(())
        })(),
    ));

    results.push((
        "UA sheet defaults: body margin 8px, h1 2em font",
        (|| {
            let (dom, root) = lay("<body><h1>Title</h1></body>", "", 800.0);
            if !close(root.border_box.x, 8.0) || !close(root.border_box.y, 8.0) {
                return Err(format!("body at ({},{}) want (8,8)", root.border_box.x, root.border_box.y));
            }
            let h1 = find(&root, &dom, "h1").expect("h1");
            if !close(h1.font_size, 32.0) {
                return Err(format!("h1 font {} want 32", h1.font_size));
            }
            Ok(())
        })(),
    ));

    results.push((
        "nested font-size percentages compound (200% then .5em → 16px)",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"font-size:200%\"><p style=\"font-size:.5em\">x</p></div></body>",
                "",
                800.0,
            );
            let p = find(&root, &dom, "p").expect("p");
            if !close(p.font_size, 16.0) {
                return Err(format!("p font {} want 16", p.font_size));
            }
            Ok(())
        })(),
    ));

    results.push((
        "inline text sits in the first line box of its paragraph",
        (|| {
            let (dom, root) = lay("<body><p>hello</p></body>", "", 800.0);
            let p = find(&root, &dom, "p").expect("p");
            if p.lines.len() != 1 || p.lines[0].words.len() != 1 {
                return Err(format!("{:#?}", p.lines));
            }
            let w = &p.lines[0].words[0];
            if w.text != "hello" || !close(w.x, p.content_box.x) {
                return Err("word not flush at content edge".into());
            }
            Ok(())
        })(),
    ));

    // --- report ---------------------------------------------------------
    let total = results.len();
    let passed = results.iter().filter(|(_, r)| r.is_ok()).count();
    println!("\n=== WPT css/CSS2 subset (Phase 3 curated): {passed}/{total} ===");
    for (name, r) in &results {
        match r {
            Ok(()) => println!("  PASS  {name}"),
            Err(e) => println!("  FAIL  {name}: {e}"),
        }
    }
    assert_eq!(passed, total, "subset regressions present");
}
