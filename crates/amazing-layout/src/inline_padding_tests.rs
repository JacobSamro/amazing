//! Actual inline positions, scopes and containing-block bases.
use crate::*;
use amazing_text::{FallbackTextSystem, GlyphId};

fn page(html: &str, css: &str, shaped: bool) -> LayoutBox {
    let dom = amazing_html::parse(&format!("<body><p id='target'>{html}</p>"));
    let css = format!("body{{margin:0;font:20px/20px serif}}p{{margin:0;width:200px;height:160px;white-space:nowrap}}{css}");
    if shaped {
        layout_dom_document_with_text_and_replaced_in_viewport(
            &dom,
            &css,
            500.0,
            Some(500.0),
            &FallbackTextSystem::new(),
            &NoReplacedElements,
        )
    } else {
        layout_dom_document_with_replaced_in_viewport(
            &dom,
            &css,
            500.0,
            Some(500.0),
            &font::FontSource::fallback(),
            &NoReplacedElements,
        )
    }
}
fn lines(root: &LayoutBox) -> Vec<&LineBox> {
    let mut out = root.lines.iter().collect::<Vec<_>>();
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &root.flow {
        for child in children {
            out.extend(lines(child));
        }
    }
    for child in &root.positioned {
        out.extend(lines(child));
    }
    out
}
fn position(root: &LayoutBox, character: char, shaped: bool, vertical: bool) -> f32 {
    if shaped {
        let glyph = lines(root)
            .into_iter()
            .flat_map(|line| &line.glyphs)
            .find(|glyph| glyph.glyph == GlyphId(character as u32))
            .unwrap();
        if vertical {
            glyph.y
        } else {
            glyph.x
        }
    } else {
        lines(root)
            .into_iter()
            .flat_map(|line| &line.words)
            .find(|word| word.text.contains(character))
            .unwrap()
            .x
    }
}
fn near(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.05,
        "actual {actual}, expected {expected}"
    );
}

#[test]
fn nested_scopes_apply_edges_once_across_distinct_text_sources() {
    let html = "A<span>B<b>C</b>D</span>E";
    for shaped in [false, true] {
        let before = page(html, "", shaped);
        let after = page(
            html,
            "span{padding-left:10%;padding-right:10%}b{padding-left:5%;padding-right:3%}",
            shaped,
        );
        for (character, delta) in [
            ('A', 0.0),
            ('B', 20.0),
            ('C', 30.0),
            ('D', 36.0),
            ('E', 56.0),
        ] {
            near(
                position(&after, character, shaped, false)
                    - position(&before, character, shaped, false),
                delta,
            );
        }
    }
}

#[test]
fn empty_and_atomic_descendants_keep_the_enclosing_padding() {
    for html in [
        "A<span></span>B",
        "A<span><i style='display:inline-block;width:30px;height:10px'></i></span>B",
    ] {
        for shaped in [false, true] {
            let before = page(html, "", shaped);
            let after = page(html, "span{padding-left:7%;padding-right:13%}", shaped);
            near(
                position(&after, 'B', shaped, false) - position(&before, 'B', shaped, false),
                40.0,
            );
            assert!(lines(&after).iter().all(|line| line
                .glyphs
                .iter()
                .all(|glyph| glyph.glyph != GlyphId('\u{fffc}' as u32))));
        }
    }
}

#[test]
fn percentage_basis_is_actual_content_inline_size_and_lengths_zoom_once() {
    let html = "A<span>B</span>C";
    for shaped in [false, true] {
        for (context, padding, delta) in [
            ("p{padding:30px}", "10%", 20.0),
            ("span{font-size:40px}", "10%", 20.0),
            ("span{zoom:2}", "calc(10% + 10px)", 40.0),
            ("p{zoom:2}span{zoom:.5}", "calc(10% + 10px)", 50.0),
            ("", "clamp(10px, 20%, 30px)", 30.0),
            ("", "min(25%, 35px)", 35.0),
        ] {
            let before = page(html, context, shaped);
            let after = page(
                html,
                &format!("{context}span{{padding-left:{padding}}}"),
                shaped,
            );
            near(
                position(&after, 'B', shaped, false) - position(&before, 'B', shaped, false),
                delta,
            );
        }
    }
}

#[test]
fn containing_writing_mode_controls_all_percentage_sides() {
    let html = "A<span>B</span>C";
    for mode in [
        "horizontal-tb",
        "vertical-rl",
        "vertical-lr",
        "sideways-rl",
        "sideways-lr",
    ] {
        let vertical = mode != "horizontal-tb";
        let css =
            format!("p{{writing-mode:{mode};height:300px}}span{{writing-mode:horizontal-tb}}");
        let before = page(html, &css, true);
        let after = page(
            html,
            &format!(
                "{css}span{{padding-top:2%;padding-right:3%;padding-bottom:4%;padding-left:5%}}"
            ),
            true,
        );
        let basis = if vertical { 300.0 } else { 200.0 };
        let edge_sum = lines(&after)
            .iter()
            .flat_map(|line| &line.words)
            .filter(|word| word.text.is_empty())
            .map(|word| word.width)
            .sum::<f32>();
        near(edge_sum, basis * if vertical { 0.06 } else { 0.08 });
        // The physical percentage basis stays the containing writing mode
        // even for a descendant using a different writing mode.
        let _ = before;
    }
}

#[test]
fn inline_padding_stays_computed_and_non_atomic_query_is_unavailable() {
    let dom = amazing_html::parse("<body><p><span id='scope'>A<b>B</b>C</span></p>");
    let styled = amazing_css::compute(
        &dom,
        &[amazing_css::parse_stylesheet(
            "body{margin:0}p{width:200px}span{padding-inline:10%}",
        )],
    );
    let scope = (0..dom.node_count())
        .map(|i| NodeIdx(i as u32))
        .find(|node| {
            dom.attrs_of(*node).is_some_and(|attrs| {
                attrs
                    .iter()
                    .any(|(key, value)| key == "id" && value == "scope")
            })
        })
        .unwrap();
    let root = layout_styled_document(
        &dom,
        &styled,
        &FallbackTextSystem::new(),
        &NoReplacedElements,
        500.0,
        Some(500.0),
    );
    assert_eq!(styled.get(scope).unwrap().get("padding-left"), Some("10%"));
    assert!(resolved_style_geometry(&root, LayoutSource::Element(scope)).is_none());
}

#[test]
fn intrinsic_percentage_contribution_is_zero_but_absolute_edges_remain() {
    fn width(root: &LayoutBox) -> f32 {
        if let Flow::Blocks(children) = &root.flow {
            return children[0].border_box.w;
        }
        root.border_box.w
    }
    for shaped in [false, true] {
        let context = "p{width:max-content;height:auto}";
        let before = page("<span>AB</span>", context, shaped);
        let percentages = page(
            "<span>AB</span>",
            &format!("{context}span{{padding-inline:10%}}"),
            shaped,
        );
        let absolute = page(
            "<span>AB</span>",
            &format!("{context}span{{padding-left:10px;padding-right:20px}}"),
            shaped,
        );
        near(width(&percentages), width(&before));
        near(width(&absolute) - width(&before), 30.0);
    }
}

#[test]
fn sliced_inline_edges_stay_with_content_when_wrapping() {
    let root = page(
        "XX <span>B C</span>",
        "p{width:30px;height:auto;white-space:normal}span{padding-left:4px;padding-right:7px}",
        true,
    );
    let all = lines(&root);
    assert!(all.len() >= 2);
    let pads = all
        .iter()
        .flat_map(|line| &line.words)
        .filter(|word| word.text.is_empty())
        .map(|word| word.width)
        .collect::<Vec<_>>();
    assert_eq!(pads.len(), 2);
    near(pads.iter().sum(), 11.0);
    for line in all {
        let has_padding = line.words.iter().any(|word| word.text.is_empty());
        if has_padding {
            assert!(
                line.words.iter().any(|word| !word.text.is_empty()),
                "detached edge"
            );
        }
    }
}

#[test]
fn padded_float_only_scopes_do_not_add_a_preceding_strut_line() {
    fn floating(root: &LayoutBox) -> Option<&LayoutBox> {
        if root.style.get("float") == Some("left") {
            return Some(root);
        }
        if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &root.flow {
            for child in children {
                if let Some(found) = floating(child) {
                    return Some(found);
                }
            }
        }
        None
    }
    for html in [
        "<span><i style='float:left;width:50px;height:50px'></i></span>",
        "<span><b><i style='float:left;width:50px;height:50px'></i></b></span>",
    ] {
        for shaped in [false, true] {
            for wrapping in ["normal", "nowrap"] {
                let css = format!("p{{white-space:{wrapping}}}");
                let before = page(html, &css, shaped);
                let after = page(html, &format!("{css}span{{padding:30px}}"), shaped);
                let before = floating(&before).unwrap();
                let after = floating(&after).unwrap();
                near(after.border_box.x, before.border_box.x);
                near(after.border_box.y, before.border_box.y);
            }
        }
    }
}

#[test]
fn padding_is_transparent_to_provider_word_boundaries() {
    fn width(root: &LayoutBox) -> f32 {
        if let Flow::Blocks(children) = &root.flow {
            return children[0].border_box.w;
        }
        root.border_box.w
    }
    for html in ["ab<span>cd</span>ef", "é<span>λ</span>μ"] {
        let before = page(
            html,
            "p{width:max-content;height:auto;white-space:normal}",
            true,
        );
        for wrapping in ["normal", "nowrap", "pre"] {
            let after=page(html,&format!("p{{width:min-content;height:auto;white-space:{wrapping}}}span{{padding-left:10px;padding-right:20px}}"),true);
            near(width(&after) - width(&before), 30.0);
        }
    }
}

#[test]
fn empty_cross_axis_padding_retains_the_strut_without_inflating_line_height() {
    for shaped in [false, true] {
        let root = page(
            "<span></span>",
            "span{padding-top:10%;padding-bottom:20%}",
            shaped,
        );
        let actual = lines(&root);
        assert_eq!(actual.len(), 1);
        near(actual[0].height, 20.0);
        assert!(actual[0].glyphs.is_empty());
        let zero = page(
            "<span></span>",
            "p{width:0}span{padding-top:10%;padding-bottom:20%}",
            shaped,
        );
        assert!(lines(&zero).is_empty());
    }
}

#[test]
fn padding_edges_preserve_collapsed_paragraph_and_inter_scope_spaces() {
    for wrapping in ["normal", "nowrap"] {
        for width in ["min-content", "max-content"] {
            for (actual, reference) in [
                (
                    "\n  <span>aaa</span><span>aaa</span>\n",
                    "<span>aaa</span><span>aaa</span>",
                ),
                (
                    "\n <span>aaa </span><span> aaa</span> ",
                    "<span>aaa </span><span>aaa</span>",
                ),
                ("<span>aaa</span>aa ", "<span>aaa</span>aa"),
            ] {
                let css=format!("p{{width:{width};height:auto;white-space:{wrapping}}}span{{padding-left:6px;padding-right:10px}}");
                let a = page(actual, &css, true);
                let b = page(reference, &css, true);
                near(
                    match &a.flow {
                        Flow::Blocks(c) => c[0].border_box.w,
                        _ => a.border_box.w,
                    },
                    match &b.flow {
                        Flow::Blocks(c) => c[0].border_box.w,
                        _ => b.border_box.w,
                    },
                );
                near(
                    lines(&a)
                        .iter()
                        .map(|line| {
                            line.words
                                .iter()
                                .map(|word| word.x + word.width)
                                .fold(0.0, f32::max)
                        })
                        .fold(0.0, f32::max),
                    lines(&b)
                        .iter()
                        .map(|line| {
                            line.words
                                .iter()
                                .map(|word| word.x + word.width)
                                .fold(0.0, f32::max)
                        })
                        .fold(0.0, f32::max),
                );
            }
        }
    }
}

#[test]
fn asymmetric_physical_padding_tracks_actual_bidi_fragment_sides() {
    for base in ["ltr", "rtl"] {
        for direction in ["ltr", "rtl"] {
            for bidi in ["normal", "embed", "isolate"] {
                for character in ['B', 'א'] {
                    let root=page(&format!("<span>{character}</span>"),
                        &format!("p{{direction:{base}}}span{{direction:{direction};unicode-bidi:{bidi};padding-left:7px;padding-right:3px;background:green}}"),true);
                    let line = lines(&root)[0];
                    let glyph = line
                        .glyphs
                        .iter()
                        .find(|glyph| glyph.glyph == GlyphId(character as u32))
                        .unwrap();
                    let background = line
                        .decorations
                        .iter()
                        .find(|decoration| decoration.width > 0.0)
                        .unwrap();
                    near(glyph.x - background.x, 7.0);
                    near(background.width, glyph.advance_x.abs() + 10.0);
                }
            }
        }
    }
}

#[test]
fn generated_inline_scope_padding_uses_the_actual_containing_block() {
    for shaped in [false, true] {
        let empty = page(
            "A",
            "p::before{content:'';padding-left:10%;padding-right:5%}",
            shaped,
        );
        let unpadded = page("A", "", shaped);
        near(
            position(&empty, 'A', shaped, false) - position(&unpadded, 'A', shaped, false),
            30.0,
        );
        let base = "p::before{content:'B'}p::after{content:'C'}";
        let before = page("A", base, shaped);
        let after = page(
            "A",
            &format!("{base}p::before,p::after{{padding-left:10%;padding-right:5%}}"),
            shaped,
        );
        for (character, delta) in [('B', 20.0), ('A', 30.0), ('C', 50.0)] {
            near(
                position(&after, character, shaped, false)
                    - position(&before, character, shaped, false),
                delta,
            );
        }
    }
}

#[test]
fn padded_nowrap_intrinsic_runs_include_spaces_and_float_shrink_to_fit_context() {
    for shaped in [false, true] {
        let actual = page(
            "\n <span>aaa</span> aa <span>aaa</span> ",
            "p{float:left;width:min-content;height:auto}span{padding-left:6px;padding-right:10px}",
            shaped,
        );
        let reference = page(
            "<span>aaa</span> aa <span>aaa</span>",
            "p{float:left;width:max-content;height:auto}span{padding-left:6px;padding-right:10px}",
            shaped,
        );
        let width = |root: &LayoutBox| match &root.flow {
            Flow::Blocks(c) => c[0].border_box.w,
            _ => root.border_box.w,
        };
        near(width(&actual), width(&reference));
    }
}

#[test]
fn padded_min_content_float_widths_match_actual_mandatory_break_runs() {
    for shaped in [false, true] {
        for (actual, reference) in [
            (
                "<span>aaa</span><span>aaa</span>",
                "<span>aaa</span><span>aaa</span>",
            ),
            (
                "<span>aaa</span> <span>aaa</span>",
                "<span>aaa</span><br><span>aaa</span>",
            ),
            (
                "<span>aaa </span><span>aaa</span>",
                "<span>aaa</span><br><span>aaa</span>",
            ),
            (
                "<span>aaa</span><span> aaa</span>",
                "<span>aaa</span><span><br>aaa</span>",
            ),
        ] {
            let css="p{float:left;width:min-content;height:auto;white-space:normal}span{padding-left:6px;padding-right:10px}";
            let a = page(actual, css, shaped);
            let b = page(reference, css, shaped);
            let width = |root: &LayoutBox| match &root.flow {
                Flow::Blocks(c) => c[0].border_box.w,
                _ => root.border_box.w,
            };
            assert!(
                (width(&a) - width(&b)).abs() < 0.05,
                "shaped={shaped}, actual={actual}, reference={reference}, widths {} vs {}",
                width(&a),
                width(&b)
            );
        }
    }
}

#[test]
fn sliced_margin_and_padding_intrinsics_match_the_whole_forced_break_selection() {
    // Each reference expresses the same minimum-content opportunity explicitly.
    // Preserve source spaces inside/outside scopes, including opening scope
    // edges that precede a break inside that scope.
    let cases = [
        ("<span>aaa</span><span>aaa</span>", "<span>aaa</span><span>aaa</span>"),
        ("<span>aaa</span>aa<span>aaa</span>", "<span>aaa</span>aa<span>aaa</span>"),
        ("<span>aaa</span> <span>aaa</span>", "<span>aaa</span><br><span>aaa</span>"),
        ("<span>aaa </span><span>aaa</span>", "<span>aaa</span><br><span>aaa</span>"),
        ("<span>aaa</span><span> aaa</span>", "<span>aaa</span><span><br>aaa</span>"),
        ("<span>aaa</span> aa<span>aaa</span>", "<span>aaa</span><br>aa<span>aaa</span>"),
        ("<span>aaa </span>aa<span>aaa</span>", "<span>aaa</span><br>aa<span>aaa</span>"),
        ("<span>aaa</span>aa<span> aaa</span>", "<span>aaa</span>aa<span><br>aaa</span>"),
        ("<span>aaa</span> aa <span>aaa</span>", "<span>aaa</span><br>aa<br><span>aaa</span>"),
        ("<span>aaa</span>aa <span>aaa</span>", "<span>aaa</span>aa<br><span>aaa</span>"),
        ("<span>aaa</span>aa ", "<span>aaa</span>aa"),
    ];
    for shaped in [false, true] {
        for margin in ["0", "0 4px 0 3px", "0 -4px 0 -3px"] {
            for (actual, reference) in cases {
                let css = format!("p{{float:left;width:min-content;height:auto;white-space:normal}}span{{padding-left:6px;padding-right:10px;margin:{margin};background:yellow;box-decoration-break:slice}}");
                let actual = page(actual, &css, shaped);
                let reference = page(reference, &format!("{css}p{{white-space:pre}}"), shaped);
                let width = |root: &LayoutBox| match &root.flow {
                    Flow::Blocks(children) => children[0].border_box.w,
                    _ => root.border_box.w,
                };
                assert!((width(&actual) - width(&reference)).abs() < 0.05,
                    "shaped={shaped};margin={margin}: {} != {}", width(&actual), width(&reference));
            }
        }
    }
}
