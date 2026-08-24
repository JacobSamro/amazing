//! Phase 3.5/3.6 gate + WPT css-flexbox-1 / css-grid-1 tracking baseline.
//!
//! Like the Phase 3 suite, this runs a curated subset modeled on the WPT
//! suites — each case drives the full pipeline (HTML → cascade → box tree
//! → block/flex/grid layout) and asserts concrete pixel rectangles. The
//! printed pass rate is the Phase 3.5/3.6 tracking number against the
//! declared subset, not the full WPT corpus.
//!
//! Subset covered: flex row/column/wrap, grow/shrink, justify-content,
//! align-items, auto margins, order, gaps; grid track sizing (px/fr/%/
//! repeat/auto-fill), line-number and span placement, implicit tracks,
//! gaps, content alignment.
//!
//! All fixtures use px-specified heights so expectations are independent
//! of font metrics. Body's UA margin (8px) is the only offset in play.

use amazing_css::{compute, parse_stylesheet};
use amazing_dom::Dom;
use amazing_layout::font::FontSource;
use amazing_layout::{box_tree::build_layout_tree, block::layout_root, Flow, LayoutBox};

/// Full pipeline; returns the DOM (for tag lookups) and the laid-out root.
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
        Flow::Blocks(kids) | Flow::Flex(kids) | Flow::Grid(kids) => {
            kids.iter().find_map(|k| find(k, dom, tag))
        },
        Flow::Inlines(_) | Flow::Empty => None,
    }
}

/// The child boxes of a container box.
fn kids(b: &LayoutBox) -> &[LayoutBox] {
    match &b.flow {
        Flow::Blocks(k) | Flow::Flex(k) | Flow::Grid(k) => k,
        _ => panic!("container has no child boxes"),
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

/// (x, y, w, h) of a box's border rect, as a plain tuple for easy compares.
fn rect(b: &LayoutBox) -> (f32, f32, f32, f32) {
    (
        b.border_box.x,
        b.border_box.y,
        b.border_box.w,
        b.border_box.h,
    )
}

fn at(got: (f32, f32, f32, f32), x: f32, y: f32, w: f32, h: f32) -> Result<(), String> {
    let ok = close(got.0, x) && close(got.1, y) && close(got.2, w) && close(got.3, h);
    if ok {
        Ok(())
    } else {
        Err(format!("rect ({:?}) want ({x},{y},{w},{h})", got))
    }
}

#[test]
fn wpt_flexbox_grid_subset_report() {
    let mut results: Vec<(&'static str, Result<(), String>)> = Vec::new();

    // --- flexbox (css-flexbox-1) ---------------------------------------

    results.push((
        "flex row places items side by side at their widths",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex\"><a style=\"width:100px;height:20px\"></a><b style=\"width:200px;height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            if k.len() != 2 {
                return Err(format!("{} children", k.len()));
            }
            // Body margin 8 shifts the container; items are relative to it.
            let x0 = d.content_box.x;
            at(rect(&k[0]), x0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), x0 + 100.0, 8.0, 200.0, 20.0)
        })(),
    ));

    results.push((
        "flex-grow splits free space by ratio",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex\"><a style=\"width:100px;height:20px;flex-grow:1\"></a><b style=\"width:100px;height:20px;flex-grow:3\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            // Content width 784; free = 784 − 200 = 584 → 146 per grow
            // unit → widths 246 and 538 (246 + 538 = 784).
            at(rect(&k[0]), 8.0, 8.0, 246.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 246.0, 8.0, 538.0, 20.0)
        })(),
    ));

    results.push((
        "flex-shrink scales overflowing items down by basis weight",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex\"><a style=\"width:600px;height:20px\"></a><b style=\"width:600px;height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            // Overflow 1200 − 784 = 416; equal shrink factors and bases →
            // each loses 208 → 392 each (exactly fills the line).
            at(rect(&k[0]), 8.0, 8.0, 392.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 392.0, 8.0, 392.0, 20.0)
        })(),
    ));

    results.push((
        "flex-wrap moves items that exceed the line to a second line",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex;flex-wrap:wrap;width:300px\"><a style=\"width:200px;height:20px\"></a><b style=\"width:200px;height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            // Initial margins are zero, so the 300px container stays at the
            // body's content edge. The older fixture accidentally treated
            // absent margins as `auto`.
            at(rect(&k[0]), 8.0, 8.0, 200.0, 20.0)?;
            at(rect(&k[1]), 8.0, 8.0 + 20.0, 200.0, 20.0)
        })(),
    ));

    results.push((
        "justify-content:center centers the items on the line",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex;justify-content:center\"><a style=\"width:100px;height:20px\"></a></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            // (784 − 100) / 2 = 342 from the content edge.
            at(rect(&k[0]), 8.0 + 342.0, 8.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "justify-content:space-between pins the ends and spreads the rest",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex;justify-content:space-between\"><a style=\"width:100px;height:20px\"></a><b style=\"width:100px;height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 784.0 - 100.0, 8.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "align-items:center centers items in the cross axis",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex;align-items:center;height:60px\"><a style=\"width:100px;height:20px\"></a></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0 + 20.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "column direction stacks items vertically",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex;flex-direction:column\"><a style=\"width:100px;height:30px\"></a><b style=\"width:100px;height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 30.0)?;
            at(rect(&k[1]), 8.0, 8.0 + 30.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "auto margins absorb free space before justify-content",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex\"><a style=\"width:100px;height:20px\"></a><b style=\"width:100px;height:20px;margin-left:auto\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 784.0 - 100.0, 8.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "order moves an item later without changing document order of others",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex\"><a style=\"width:100px;height:20px;order:2\"></a><b style=\"width:100px;height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            // b (order 0) paints first; a (order 2) second.
            at(rect(&k[0]), 8.0 + 100.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0, 8.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "row-gap and column-gap space the items",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:flex;gap:10px 20px\"><a style=\"width:100px;height:20px\"></a><b style=\"width:100px;height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 120.0, 8.0, 100.0, 20.0)
        })(),
    ));

    // --- grid (css-grid-1) ----------------------------------------------

    results.push((
        "grid px tracks place items row-major",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:100px 200px;grid-template-rows:20px 30px\"><a></a><b></b><c></c><d></d></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 100.0, 8.0, 200.0, 20.0)?;
            at(rect(&k[2]), 8.0, 8.0 + 20.0, 100.0, 30.0)?;
            at(rect(&k[3]), 8.0 + 100.0, 8.0 + 20.0, 200.0, 30.0)
        })(),
    ));

    results.push((
        "fr track takes the leftover after fixed tracks and gaps",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:100px 1fr;column-gap:50px\"><a style=\"height:20px\"></a><b style=\"height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 150.0, 8.0, 784.0 - 150.0, 20.0)
        })(),
    ));

    results.push((
        "line-number placement puts items on explicit cells",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:100px 200px\"><a style=\"grid-column:2;grid-row:1;height:20px\"></a><b style=\"height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            // a locked to column 2 row 1; b auto-fills (1,1).
            at(rect(&k[0]), 8.0 + 100.0, 8.0, 200.0, 20.0)?;
            at(rect(&k[1]), 8.0, 8.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "negative line numbers anchor to the explicit grid end",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:100px 200px\"><a style=\"grid-column:1 / -1;height:20px\"></a></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            // 1 / -1 spans the whole explicit grid.
            at(rect(&k[0]), 8.0, 8.0, 300.0, 20.0)
        })(),
    ));

    results.push((
        "span keyword widens an item and wraps the next",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:100px 200px\"><a style=\"grid-column:span 2;height:30px\"></a><b style=\"height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 300.0, 30.0)?;
            at(rect(&k[1]), 8.0, 8.0 + 30.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "implicit rows are created on demand and sized by content",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:100px 100px\"><a style=\"height:20px\"></a><b style=\"height:20px\"></b><c style=\"height:40px\"></c></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 100.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[2]), 8.0, 8.0 + 20.0, 100.0, 40.0)
        })(),
    ));

    results.push((
        "repeat(2, N) expands to two equal fixed tracks",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:repeat(2, 100px)\"><a style=\"height:20px\"></a><b style=\"height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 100.0, 8.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "auto-fill repeats definite tracks to fill the container",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:repeat(auto-fill, 100px);width:350px\"><a style=\"height:20px\"></a><b style=\"height:20px\"></b><c style=\"height:20px\"></c></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            // Initial margins are zero. Three 100px tracks fit into 350px,
            // so all items share row 1 from the body content edge.
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 100.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[2]), 8.0 + 200.0, 8.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "percentage tracks resolve against the content width",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:50% 200px\"><a style=\"height:20px\"></a><b style=\"height:20px\"></b></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 392.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 392.0, 8.0, 200.0, 20.0)
        })(),
    ));

    results.push((
        "grid gaps separate both tracks",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:100px 100px;grid-template-rows:20px 20px;gap:10px 20px\"><a></a><b></b><c></c><d></d></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[1]), 8.0 + 120.0, 8.0, 100.0, 20.0)?;
            at(rect(&k[2]), 8.0, 8.0 + 30.0, 100.0, 20.0)?;
            at(rect(&k[3]), 8.0 + 120.0, 8.0 + 30.0, 100.0, 20.0)
        })(),
    ));

    results.push((
        "justify-content:center centers the track list",
        (|| {
            let (dom, root) = lay(
                "<body><div style=\"display:grid;grid-template-columns:100px;justify-content:center\"><a style=\"height:20px\"></a></div></body>",
                "",
                800.0,
            );
            let d = find(&root, &dom, "div").ok_or("no div")?;
            let k = kids(d);
            at(rect(&k[0]), 8.0 + 342.0, 8.0, 100.0, 20.0)
        })(),
    ));

    // --- report ---------------------------------------------------------
    let total = results.len();
    let passed = results.iter().filter(|(_, r)| r.is_ok()).count();
    println!("\n=== WPT flexbox/grid subset (Phase 3.5/3.6 curated): {passed}/{total} ===");
    for (name, r) in &results {
        match r {
            Ok(()) => println!("  PASS  {name}"),
            Err(e) => println!("  FAIL  {name}: {e}"),
        }
    }
    assert_eq!(passed, total, "subset regressions present");
}
